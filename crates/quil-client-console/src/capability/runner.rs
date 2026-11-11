use std::{io, path::PathBuf, process::Stdio};

use async_trait::async_trait;
use serde::Serialize;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
};

#[cfg(test)]
use super::tasks::OUTPUT_TRUNCATED_MARKER;
use super::tasks::{limit_output, MAX_TASK_OUTPUT_BYTES as MAX_CAPTURED_OUTPUT_BYTES};
use super::CapabilityInvocation;
use crate::service::ServiceError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CapabilityRunResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

#[async_trait]
pub trait CapabilityRunner: Send + Sync {
    async fn run(
        &self,
        invocation: CapabilityInvocation,
    ) -> Result<CapabilityRunResult, ServiceError>;
}

#[derive(Debug, Clone)]
pub struct CurrentExeRunner {
    current_exe: Result<PathBuf, String>,
    signature_check: bool,
}

impl CurrentExeRunner {
    pub fn new(current_exe: impl Into<PathBuf>) -> Self {
        Self::new_with_signature_check(current_exe, true)
    }

    pub fn new_with_signature_check(
        current_exe: impl Into<PathBuf>,
        signature_check: bool,
    ) -> Self {
        Self {
            current_exe: Ok(current_exe.into()),
            signature_check,
        }
    }

    pub fn current() -> Self {
        Self::current_with_signature_check(true)
    }

    pub fn current_with_signature_check(signature_check: bool) -> Self {
        Self::from_current_exe_result_with_signature_check(std::env::current_exe(), signature_check)
    }

    #[doc(hidden)]
    pub fn from_current_exe_result(current_exe: io::Result<PathBuf>) -> Self {
        Self::from_current_exe_result_with_signature_check(current_exe, true)
    }

    fn from_current_exe_result_with_signature_check(
        current_exe: io::Result<PathBuf>,
        signature_check: bool,
    ) -> Self {
        Self {
            current_exe: current_exe.map_err(|error| {
                format!("Cannot determine the current qclient-console executable: {error}")
            }),
            signature_check,
        }
    }
}

#[async_trait]
impl CapabilityRunner for CurrentExeRunner {
    async fn run(
        &self,
        invocation: CapabilityInvocation,
    ) -> Result<CapabilityRunResult, ServiceError> {
        let current_exe = self
            .current_exe
            .as_ref()
            .map_err(|error| ServiceError::Operation(error.clone()))?;
        if !current_exe.is_absolute() {
            return Err(ServiceError::InvalidInput(
                "Capability runner executable path must be absolute".into(),
            ));
        }

        let (child_argv, private_stdin) =
            invocation
                .child_process(self.signature_check)
                .map_err(|_| {
                    ServiceError::InvalidInput(format!(
                        "Cannot prepare private input for capability {}",
                        invocation.capability_id
                    ))
                })?;

        let mut child = Command::new(current_exe)
            .args(&child_argv)
            .stdin(if private_stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                ServiceError::Operation(format!(
                    "Cannot run capability {}: {error}",
                    invocation.capability_id
                ))
            })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            ServiceError::Operation(format!(
                "Cannot capture stdout for capability {}",
                invocation.capability_id
            ))
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            ServiceError::Operation(format!(
                "Cannot capture stderr for capability {}",
                invocation.capability_id
            ))
        })?;
        let mut stdin = child.stdin.take();
        let write_private_stdin = async move {
            if let Some(payload) = private_stdin {
                let mut stdin = stdin.take().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::BrokenPipe, "private stdin unavailable")
                })?;
                stdin.write_all(&payload).await?;
                stdin.shutdown().await?;
            }
            Ok::<(), io::Error>(())
        };

        let (stdin_result, status, stdout, stderr) = tokio::join!(
            write_private_stdin,
            child.wait(),
            read_limited(stdout),
            read_limited(stderr)
        );
        stdin_result.map_err(|_| {
            ServiceError::Operation(format!(
                "Cannot send private input for capability {}",
                invocation.capability_id
            ))
        })?;
        let status = status.map_err(|error| {
            ServiceError::Operation(format!(
                "Cannot wait for capability {}: {error}",
                invocation.capability_id
            ))
        })?;
        let stdout = stdout.map_err(|error| {
            ServiceError::Operation(format!(
                "Cannot read stdout for capability {}: {error}",
                invocation.capability_id
            ))
        })?;
        let stderr = stderr.map_err(|error| {
            ServiceError::Operation(format!(
                "Cannot read stderr for capability {}: {error}",
                invocation.capability_id
            ))
        })?;

        Ok(CapabilityRunResult {
            // Unix signals do not provide an exit code. Treat those as a stable failure.
            exit_code: status.code().unwrap_or(1),
            stdout: render_limited(stdout),
            stderr: render_limited(stderr),
        })
    }
}

#[derive(Debug)]
struct LimitedOutput {
    bytes: Vec<u8>,
    truncated: bool,
}

async fn read_limited(mut reader: impl AsyncRead + Unpin) -> io::Result<LimitedOutput> {
    let mut bytes = Vec::with_capacity(MAX_CAPTURED_OUTPUT_BYTES);
    let mut buffer = [0_u8; 8192];
    let mut truncated = false;
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        let remaining = MAX_CAPTURED_OUTPUT_BYTES.saturating_sub(bytes.len());
        let retained = read.min(remaining);
        bytes.extend_from_slice(&buffer[..retained]);
        truncated |= retained < read;
    }
    Ok(LimitedOutput { bytes, truncated })
}

fn render_limited(output: LimitedOutput) -> String {
    limit_output(
        String::from_utf8_lossy(&output.bytes).into_owned(),
        output.truncated,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn limited_reader_drains_input_and_marks_truncated_output_within_the_limit() {
        let input = vec![b'x'; MAX_CAPTURED_OUTPUT_BYTES + 1024];
        let mut remaining = input.as_slice();

        let captured = read_limited(&mut remaining).await.unwrap();
        let rendered = render_limited(captured);

        assert!(remaining.is_empty());
        assert!(rendered.len() <= MAX_CAPTURED_OUTPUT_BYTES);
        assert!(rendered.ends_with(OUTPUT_TRUNCATED_MARKER));
    }

    #[tokio::test]
    async fn limited_reader_keeps_non_utf8_output_safe_and_bounded() {
        let input = vec![0xff; MAX_CAPTURED_OUTPUT_BYTES];

        let captured = read_limited(input.as_slice()).await.unwrap();
        let rendered = render_limited(captured);

        assert!(rendered.len() <= MAX_CAPTURED_OUTPUT_BYTES);
        assert!(rendered.ends_with(OUTPUT_TRUNCATED_MARKER));
    }
}
