use std::{
    collections::{BTreeMap, HashMap},
    fs::File,
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use quil_rpc::log_export::{
    bundle_texts, compact_ts, file_readers, produce, write_token_map, ExportOptions, RecordReader,
    Selection, SourceInfo, BUNDLE_LOG,
};

use super::ServiceError;
use crate::domain::{
    LogExportOptions, LogExportPreview, LogSnapshot, OperationOutcome, OperationResult,
};

const MAX_TAIL_BYTES: u64 = 1024 * 1024;
const SYSTEM_LOG_DIR: &str = "/var/log/quilibrium";

#[derive(Default)]
pub(super) struct LogExportStore {
    next_id: AtomicU64,
    prepared: Mutex<HashMap<u64, PreparedLogExport>>,
}

struct PreparedLogExport {
    destination: PathBuf,
    map_path: PathBuf,
    staging: PathBuf,
    token_map: BTreeMap<String, String>,
    cleanup: bool,
}

impl Drop for PreparedLogExport {
    fn drop(&mut self) {
        if self.cleanup {
            let _ = std::fs::remove_dir_all(&self.staging);
        }
    }
}

impl LogExportStore {
    pub(super) async fn prepare(
        &self,
        config_dir: &Path,
        options: LogExportOptions,
    ) -> Result<LogExportPreview, ServiceError> {
        if !(60..=24 * 60 * 60).contains(&options.window_seconds) {
            return Err(ServiceError::InvalidInput(
                "Log export window must be between 1 minute and 24 hours".into(),
            ));
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        let config_dir = config_dir.to_path_buf();
        let (preview, prepared) =
            tokio::task::spawn_blocking(move || prepare_export(&config_dir, options, id))
                .await
                .map_err(|error| {
                    ServiceError::LogExport(format!("Log export task failed: {error}"))
                })??;
        self.prepared
            .lock()
            .map_err(|_| ServiceError::LogExport("Log export state lock is poisoned".into()))?
            .insert(id, prepared);
        Ok(preview)
    }

    pub(super) async fn finish(
        &self,
        id: u64,
        commit: bool,
    ) -> Result<OperationResult, ServiceError> {
        let prepared = self
            .prepared
            .lock()
            .map_err(|_| ServiceError::LogExport("Log export state lock is poisoned".into()))?
            .remove(&id)
            .ok_or_else(|| {
                ServiceError::LogExport("Log export preview expired; generate it again".into())
            })?;
        tokio::task::spawn_blocking(move || finish_export(prepared, commit))
            .await
            .map_err(|error| ServiceError::LogExport(format!("Log write task failed: {error}")))?
    }
}

fn prepare_export(
    config_dir: &Path,
    options: LogExportOptions,
    id: u64,
) -> Result<(LogExportPreview, PreparedLogExport), ServiceError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| ServiceError::LogExport(format!("Invalid system time: {error}")))?
        .as_secs() as i64;
    let selection = Selection {
        since: now.saturating_sub(options.window_seconds),
        until: now,
        min_level: options.min_level.rank(),
        filters: Vec::new(),
        matches: Vec::new(),
    };
    let config = quil_config::load_config(config_dir)
        .map_err(|error| ServiceError::Config(error.to_string()))?;
    let log_dir = config
        .logger
        .log_dir(config_dir)
        .map(|path| absolute_log_dir(config_dir, path));
    let (readers, source, child) =
        open_export_source(log_dir.as_deref(), selection.since, selection.until)?;
    let destination = std::env::current_dir()
        .map_err(|error| {
            ServiceError::LogExport(format!("Cannot read current directory: {error}"))
        })?
        .join(format!("node-log-export-{}", compact_ts(now)));
    let map_path = with_suffix(&destination, ".map.txt");
    let staging = with_suffix(&destination, &format!(".partial-{id}"));
    for path in [&destination, &map_path, &staging] {
        if path.exists() {
            return Err(ServiceError::LogExport(format!(
                "{} already exists; retry later or move the existing file",
                path.display()
            )));
        }
    }
    std::fs::create_dir(&staging).map_err(|error| {
        ServiceError::LogExport(format!("Cannot create {}: {error}", staging.display()))
    })?;
    let mut prepared = PreparedLogExport {
        destination: destination.clone(),
        map_path: map_path.clone(),
        staging: staging.clone(),
        token_map: BTreeMap::new(),
        cleanup: true,
    };
    let log_path = staging.join(BUNDLE_LOG);
    let file = File::create(&log_path).map_err(|error| {
        ServiceError::LogExport(format!("Cannot create {}: {error}", log_path.display()))
    })?;
    let home_paths = [config_dir.to_path_buf()];
    let export = produce(
        readers,
        child,
        &source,
        &selection,
        std::io::BufWriter::new(file),
        &ExportOptions {
            home_paths: &home_paths,
            ..ExportOptions::default()
        },
    )
    .map_err(|error| ServiceError::LogExport(error.to_string()))?;
    let (manifest_text, sums) = bundle_texts(&export.manifest, &export.log)
        .map_err(|error| ServiceError::LogExport(error.to_string()))?;
    std::fs::write(staging.join("manifest.json"), manifest_text)
        .and_then(|_| std::fs::write(staging.join("SHA256SUMS"), sums))
        .map_err(|error| {
            ServiceError::LogExport(format!("Cannot write export manifest: {error}"))
        })?;

    let redactions = export.manifest["redaction"]["counts"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(class, count)| {
            (
                class.clone(),
                count.as_u64().unwrap_or_default(),
                export.manifest["redaction"]["distinct_values"][class]
                    .as_u64()
                    .unwrap_or_default(),
            )
        })
        .collect();
    let source_label = match &source {
        SourceInfo::Files(files) => format!("{} log files", files.len()),
        SourceInfo::Journal(unit) => format!("systemd journal · {unit}"),
    };
    let preview = LogExportPreview {
        id,
        destination: destination.display().to_string(),
        token_map_path: map_path.display().to_string(),
        window_since: export.manifest["window"]["since"]
            .as_str()
            .unwrap_or_default()
            .into(),
        window_until: export.manifest["window"]["until"]
            .as_str()
            .unwrap_or_default()
            .into(),
        source: source_label,
        lines_read: export.stats.lines_read,
        lines_kept: export.stats.lines_kept,
        redactions,
        samples: export.stats.samples.clone(),
        read_errors: export.stats.read_errors.len(),
    };
    prepared.token_map = export.token_map;
    Ok((preview, prepared))
}

fn finish_export(
    mut prepared: PreparedLogExport,
    commit: bool,
) -> Result<OperationResult, ServiceError> {
    if !commit {
        return Ok(OperationResult {
            message: "Log export cancelled; temporary files were removed".into(),
            outcome: OperationOutcome::Submitted,
        });
    }
    write_token_map(
        &prepared.map_path,
        &prepared.destination.display().to_string(),
        &prepared.token_map,
    )
    .map_err(|error| ServiceError::LogExport(error.to_string()))?;
    if let Err(error) = std::fs::rename(&prepared.staging, &prepared.destination) {
        let _ = std::fs::remove_file(&prepared.map_path);
        return Err(ServiceError::LogExport(format!(
            "Cannot write {}: {error}",
            prepared.destination.display()
        )));
    }
    prepared.cleanup = false;
    Ok(OperationResult {
        message: format!(
            "Redacted logs written to {}; {} contains the original-value map and must not be shared",
            prepared.destination.display(),
            prepared.map_path.display()
        ),
        outcome: OperationOutcome::Submitted,
    })
}

fn open_export_source(
    log_dir: Option<&Path>,
    since: i64,
    until: i64,
) -> Result<(Vec<RecordReader>, SourceInfo, Option<std::process::Child>), ServiceError> {
    if let Some(directory) = log_dir.filter(|path| path.is_dir()) {
        let (readers, files) = file_readers(directory, since, until)
            .map_err(|error| ServiceError::LogExport(error.to_string()))?;
        if files.is_empty() {
            return Err(ServiceError::LogExport(format!(
                "No node log files found in {}",
                directory.display()
            )));
        }
        return Ok((readers, SourceInfo::Files(files), None));
    }
    if !cfg!(target_os = "linux") {
        return Err(ServiceError::LogExport(
            "The node has no configured log files; systemd journal export is Linux-only".into(),
        ));
    }
    let mut command = Command::new("journalctl");
    command
        .args(["--utc", "-o", "cat", "--no-pager", "-u", "quilibrium-node"])
        .arg("--since")
        .arg(format!("@{since}"))
        .arg("--until")
        .arg(format!("@{until}"))
        .stdout(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| ServiceError::LogExport(format!("Cannot run journalctl: {error}")))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ServiceError::LogExport("journalctl has no output pipe".into()))?;
    let reader: Box<dyn BufRead> = Box::new(BufReader::new(stdout));
    Ok((
        vec![RecordReader::new(vec![("journal".into(), reader)])],
        SourceInfo::Journal("quilibrium-node".into()),
        Some(child),
    ))
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_owned();
    value.push(suffix);
    PathBuf::from(value)
}

pub(super) async fn read(
    config_dir: &Path,
    core_id: u32,
    lines: usize,
) -> Result<LogSnapshot, ServiceError> {
    let config = quil_config::load_config(config_dir)
        .map_err(|error| ServiceError::Config(error.to_string()))?;
    let config_dir = config_dir.to_path_buf();
    tokio::task::spawn_blocking(move || match config.logger.log_dir(&config_dir) {
        Some(directory) => {
            let directory = absolute_log_dir(&config_dir, directory);
            let path = directory.join(log_file_name(core_id));
            let content = read_tail_lines(&path, lines)?;
            Ok(LogSnapshot {
                core_id,
                path: path.display().to_string(),
                lines: content,
            })
        }
        None if cfg!(target_os = "linux") => read_journal(lines),
        None => {
            let path = Path::new(SYSTEM_LOG_DIR).join("node.err");
            let content = read_tail_lines(&path, lines)?;
            Ok(LogSnapshot {
                core_id,
                path: path.display().to_string(),
                lines: content,
            })
        }
    })
    .await
    .map_err(|error| ServiceError::Logs(format!("Log read task failed: {error}")))?
}

fn absolute_log_dir(config_dir: &Path, directory: PathBuf) -> PathBuf {
    if directory.is_absolute() || directory.starts_with(config_dir) {
        directory
    } else {
        config_dir.join(directory)
    }
}

fn log_file_name(core_id: u32) -> String {
    if core_id == 0 {
        "master.log".into()
    } else {
        format!("worker-{core_id}.log")
    }
}

fn read_tail_lines(path: &Path, limit: usize) -> Result<Vec<String>, ServiceError> {
    let mut file = File::open(path)
        .map_err(|error| ServiceError::Logs(format!("Cannot open {}: {error}", path.display())))?;
    let length = file
        .metadata()
        .map_err(|error| ServiceError::Logs(format!("Cannot read {}: {error}", path.display())))?
        .len();
    let start = length.saturating_sub(MAX_TAIL_BYTES);
    file.seek(SeekFrom::Start(start))
        .map_err(|error| ServiceError::Logs(format!("Cannot seek {}: {error}", path.display())))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| ServiceError::Logs(format!("Cannot read {}: {error}", path.display())))?;
    let text = String::from_utf8_lossy(&bytes);
    let text = if start > 0 {
        text.split_once('\n').map(|(_, rest)| rest).unwrap_or("")
    } else {
        text.as_ref()
    };
    let mut result = text
        .lines()
        .rev()
        .take(limit)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    result.reverse();
    Ok(result)
}

fn read_journal(lines: usize) -> Result<LogSnapshot, ServiceError> {
    let output = Command::new("journalctl")
        .args([
            "--utc",
            "-o",
            "cat",
            "--no-pager",
            "-u",
            "quilibrium-node",
            "-n",
            &lines.to_string(),
        ])
        .output()
        .map_err(|error| ServiceError::Logs(format!("Cannot run journalctl: {error}")))?;
    if !output.status.success() {
        return Err(ServiceError::Logs(format!(
            "journalctl returned {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(LogSnapshot {
        core_id: 0,
        path: "systemd:quilibrium-node".into(),
        lines: String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::to_owned)
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prepared_export(root: &Path) -> PreparedLogExport {
        let destination = root.join("export");
        let staging = root.join("export.partial");
        std::fs::create_dir(&staging).unwrap();
        std::fs::write(staging.join("bundle.log.gz"), b"redacted").unwrap();
        PreparedLogExport {
            map_path: root.join("export.map.txt"),
            destination,
            staging,
            token_map: BTreeMap::from([("secret".into(), "TOKEN-1".into())]),
            cleanup: true,
        }
    }

    #[test]
    fn names_master_and_worker_logs() {
        assert_eq!(log_file_name(0), "master.log");
        assert_eq!(log_file_name(7), "worker-7.log");
    }

    #[test]
    fn reads_only_requested_tail_lines() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("master.log");
        std::fs::write(&path, "one\ntwo\nthree\nfour\n").unwrap();

        assert_eq!(read_tail_lines(&path, 2).unwrap(), vec!["three", "four"]);
    }

    #[test]
    fn cancelling_export_removes_staging_without_writing_private_map() {
        let temp = tempfile::tempdir().unwrap();
        let prepared = prepared_export(temp.path());
        let staging = prepared.staging.clone();
        let map_path = prepared.map_path.clone();

        finish_export(prepared, false).unwrap();

        assert!(!staging.exists());
        assert!(!map_path.exists());
    }

    #[test]
    fn committing_export_renames_bundle_and_writes_private_map() {
        let temp = tempfile::tempdir().unwrap();
        let prepared = prepared_export(temp.path());
        let destination = prepared.destination.clone();
        let staging = prepared.staging.clone();
        let map_path = prepared.map_path.clone();

        finish_export(prepared, true).unwrap();

        assert!(destination.join("bundle.log.gz").exists());
        assert!(!staging.exists());
        let map = std::fs::read_to_string(map_path).unwrap();
        assert!(map.contains("TOKEN-1"));
        assert!(map.contains("secret"));
    }
}
