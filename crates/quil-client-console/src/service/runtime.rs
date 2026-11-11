use std::process::{Command, Output};

use super::ServiceError;
use crate::domain::RuntimeSnapshot;

const SERVICE_NAME: &str = "quilibrium-node";
const LAUNCHD_LABEL: &str = "com.quilibrium.quilibrium-node";
const LAUNCHD_PLIST: &str = "/Library/LaunchDaemons/com.quilibrium.quilibrium-node.plist";
const SYSTEMD_UNIT: &str = "/etc/systemd/system/quilibrium-node.service";
const AUTO_UPDATE_MARKER: &str = "# quilibrium-auto-update";

pub(super) async fn read() -> Result<RuntimeSnapshot, ServiceError> {
    tokio::task::spawn_blocking(read_platform)
        .await
        .map_err(|error| {
            ServiceError::Runtime(format!("Runtime inspection task failed: {error}"))
        })?
}

fn read_platform() -> Result<RuntimeSnapshot, ServiceError> {
    match std::env::consts::OS {
        "macos" => read_launchd(),
        "linux" => read_systemd(),
        platform => Ok(RuntimeSnapshot {
            platform: platform.into(),
            manager: "unsupported".into(),
            service_name: SERVICE_NAME.into(),
            state: "unknown".into(),
            note: Some(
                "No supported service-manager inspection is available on this platform".into(),
            ),
            ..Default::default()
        }),
    }
}

fn read_launchd() -> Result<RuntimeSnapshot, ServiceError> {
    let target = format!("system/{LAUNCHD_LABEL}");
    let output = command_output("launchctl", &["print", &target])?;
    let installed = std::path::Path::new(LAUNCHD_PLIST).exists();
    let text = String::from_utf8_lossy(&output.stdout);
    let state = assignment(&text, "state").unwrap_or_else(|| {
        if installed {
            "not running".into()
        } else {
            "not installed".into()
        }
    });
    let pid = assignment(&text, "pid").and_then(|value| value.parse().ok());
    let enabled = launchd_enabled();
    Ok(RuntimeSnapshot {
        platform: "macOS".into(),
        manager: "launchd".into(),
        service_name: LAUNCHD_LABEL.into(),
        installed,
        running: output.status.success() && (state == "running" || pid.is_some()),
        enabled,
        state,
        pid,
        definition_path: LAUNCHD_PLIST.into(),
        auto_update_enabled: crontab_auto_update(),
        note: (!output.status.success() && installed).then(|| clean_stderr(&output)),
    })
}

fn launchd_enabled() -> Option<bool> {
    let output = Command::new("launchctl")
        .args(["print-disabled", "system"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let Some(line) = text.lines().find(|line| line.contains(LAUNCHD_LABEL)) else {
        return Some(true);
    };
    if line.contains("=> true") {
        Some(false)
    } else if line.contains("=> false") {
        Some(true)
    } else {
        None
    }
}

fn read_systemd() -> Result<RuntimeSnapshot, ServiceError> {
    let output = command_output(
        "systemctl",
        &[
            "show",
            SERVICE_NAME,
            "--no-pager",
            "--property=LoadState,ActiveState,SubState,UnitFileState,MainPID,FragmentPath",
        ],
    )?;
    let text = String::from_utf8_lossy(&output.stdout);
    let load = property(&text, "LoadState").unwrap_or_default();
    let active = property(&text, "ActiveState").unwrap_or_default();
    let sub = property(&text, "SubState").unwrap_or_default();
    let unit_state = property(&text, "UnitFileState").unwrap_or_default();
    let pid = property(&text, "MainPID")
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0);
    let mut fragment = property(&text, "FragmentPath").unwrap_or_default();
    if fragment.is_empty() {
        fragment = SYSTEMD_UNIT.into();
    }
    let installed =
        (!load.is_empty() && load != "not-found") || std::path::Path::new(SYSTEMD_UNIT).exists();
    let state = match (active.as_str(), sub.as_str()) {
        ("", "") if !installed => "not installed".into(),
        (active, "") => active.into(),
        (active, sub) => format!("{active}/{sub}"),
    };
    Ok(RuntimeSnapshot {
        platform: "Linux".into(),
        manager: "systemd".into(),
        service_name: SERVICE_NAME.into(),
        installed,
        running: active == "active" && sub == "running",
        enabled: match unit_state.as_str() {
            "enabled" | "enabled-runtime" => Some(true),
            "disabled" | "masked" | "masked-runtime" => Some(false),
            _ => None,
        },
        state,
        pid,
        definition_path: fragment,
        auto_update_enabled: crontab_auto_update(),
        note: (!output.status.success()).then(|| clean_stderr(&output)),
    })
}

fn command_output(program: &str, args: &[&str]) -> Result<Output, ServiceError> {
    Command::new(program)
        .args(args)
        .output()
        .map_err(|error| ServiceError::Runtime(format!("Cannot run {program}: {error}")))
}

fn assignment(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (name, value) = line.trim().split_once(" = ")?;
        (name == key).then(|| value.trim_matches('"').to_string())
    })
}

fn property(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (name, value) = line.trim().split_once('=')?;
        (name == key).then(|| value.to_string())
    })
}

fn crontab_auto_update() -> Option<bool> {
    let output = Command::new("crontab").arg("-l").output().ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).contains(AUTO_UPDATE_MARKER))
    } else if output.status.code() == Some(1) {
        Some(false)
    } else {
        None
    }
}

fn clean_stderr(output: &Output) -> String {
    let value = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if value.is_empty() {
        format!("Service manager returned {}", output.status)
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_launchd_assignments_without_matching_similar_keys() {
        let text = "state = running\nlast exit code = 0\npid = 123\n";

        assert_eq!(assignment(text, "state").as_deref(), Some("running"));
        assert_eq!(assignment(text, "pid").as_deref(), Some("123"));
        assert_eq!(assignment(text, "exit code"), None);
    }

    #[test]
    fn parses_systemd_properties_exactly() {
        let text = "ActiveState=active\nSubState=running\nMainPID=42\n";

        assert_eq!(property(text, "ActiveState").as_deref(), Some("active"));
        assert_eq!(property(text, "MainPID").as_deref(), Some("42"));
        assert_eq!(property(text, "PID"), None);
    }
}
