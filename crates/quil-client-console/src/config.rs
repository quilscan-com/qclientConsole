use std::{
    ffi::OsString,
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

pub const USAGE: &str =
    "Usage: qclient-console [--config-dir <PATH> | -c <PATH>]\n       qclient-console --help";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunOptions {
    Run { explicit_config: Option<PathBuf> },
    Help,
}

pub fn parse_args<I, S>(args: I) -> Result<RunOptions, String>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let mut args = args.into_iter().map(Into::into);
    let mut explicit_config = None;

    while let Some(argument) = args.next() {
        if argument == "--help" || argument == "-h" {
            return Ok(RunOptions::Help);
        }

        if argument == "--config-dir" || argument == "-c" {
            let value = args.next().ok_or_else(|| {
                format!(
                    "Argument {} requires a path\n\n{USAGE}",
                    argument.to_string_lossy()
                )
            })?;
            if value.is_empty() {
                return Err(format!(
                    "Argument {} requires a path\n\n{USAGE}",
                    argument.to_string_lossy()
                ));
            }
            explicit_config = Some(PathBuf::from(value));
            continue;
        }

        if let Some(value) = argument
            .to_str()
            .and_then(|argument| argument.strip_prefix("--config-dir="))
        {
            if value.is_empty() {
                return Err(format!("Argument --config-dir requires a path\n\n{USAGE}"));
            }
            explicit_config = Some(PathBuf::from(value));
            continue;
        }

        return Err(format!(
            "Unknown argument: {}\n\n{USAGE}",
            argument.to_string_lossy()
        ));
    }

    Ok(RunOptions::Run { explicit_config })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupConfig {
    pub entry: Option<ConfigEntry>,
    pub input: String,
    pub error: Option<String>,
}

pub fn validate_config_dir(path: &Path) -> Result<ConfigEntry, String> {
    let canonical = std::fs::canonicalize(path).map_err(|error| {
        format!(
            "Cannot access configuration directory {}: {error}",
            path.to_string_lossy()
        )
    })?;

    if !canonical.is_dir() {
        return Err(format!(
            "Configuration path is not a directory: {}",
            canonical.to_string_lossy()
        ));
    }

    for file_name in ["config.yml", "keys.yml"] {
        let file_path = canonical.join(file_name);
        if !file_path.is_file() {
            return Err(format!(
                "Configuration directory is missing required file {file_name}"
            ));
        }
        File::open(&file_path).map_err(|error| {
            format!(
                "Cannot read configuration file {}: {error}",
                file_path.to_string_lossy()
            )
        })?;
    }

    let name = canonical
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("node")
        .to_owned();
    Ok(ConfigEntry {
        name,
        path: canonical,
        is_default: true,
    })
}

pub fn settings_file(home: &Path) -> PathBuf {
    home.join(".qclient-console").join("config-dir")
}

pub fn remember_config_dir(home: &Path, path: &Path) -> Result<(), String> {
    let entry = validate_config_dir(path)?;
    let remembered = entry
        .path
        .to_str()
        .ok_or_else(|| "Configuration directory path is not valid UTF-8".to_owned())?;
    let settings = settings_file(home);
    let parent = settings.parent().ok_or_else(|| {
        "Cannot determine the saved configuration path parent directory".to_owned()
    })?;
    std::fs::create_dir_all(parent).map_err(|error| {
        format!(
            "Cannot create saved configuration directory {}: {error}",
            parent.display()
        )
    })?;
    let temporary = settings.with_file_name(format!("config-dir.{}.tmp", std::process::id()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temporary)
            .map_err(|error| {
                format!(
                    "Cannot create temporary saved configuration file {}: {error}",
                    temporary.display()
                )
            })?;
        file.write_all(remembered.as_bytes()).map_err(|error| {
            format!(
                "Cannot write temporary saved configuration file {}: {error}",
                temporary.display()
            )
        })?;
        file.sync_all().map_err(|error| {
            format!(
                "Cannot sync temporary saved configuration file {}: {error}",
                temporary.display()
            )
        })?;
        drop(file);
        std::fs::rename(&temporary, &settings).map_err(|error| {
            format!(
                "Cannot replace saved configuration file {}: {error}",
                settings.display()
            )
        })
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

pub fn read_remembered_config_dir(home: &Path) -> Result<PathBuf, String> {
    let settings = settings_file(home);
    let contents = std::fs::read_to_string(&settings).map_err(|error| {
        format!(
            "Cannot read saved configuration file {}: {error}",
            settings.display()
        )
    })?;
    if contents.is_empty() {
        return Err("Saved configuration path file is empty".to_owned());
    }
    Ok(PathBuf::from(contents))
}

pub fn select_startup_config(explicit: Option<PathBuf>, cwd: &Path, home: &Path) -> StartupConfig {
    if let Some(path) = explicit {
        let input = path.to_string_lossy().into_owned();
        return match validate_config_dir(&path) {
            Ok(entry) => StartupConfig {
                entry: Some(entry),
                input,
                error: None,
            },
            Err(error) => StartupConfig {
                entry: None,
                input,
                error: Some(error),
            },
        };
    }

    let local = cwd.join(".config");
    let mut fallback_input = String::new();
    let mut fallback_error = None;
    if local.exists() {
        match validate_config_dir(&local) {
            Ok(entry) => {
                return StartupConfig {
                    input: local.to_string_lossy().into_owned(),
                    entry: Some(entry),
                    error: None,
                };
            }
            Err(error) => {
                fallback_input = local.to_string_lossy().into_owned();
                fallback_error = Some(error);
            }
        }
    }

    match read_remembered_config_dir(home) {
        Ok(path) => {
            let input = path.to_string_lossy().into_owned();
            match validate_config_dir(&path) {
                Ok(entry) => StartupConfig {
                    entry: Some(entry),
                    input,
                    error: None,
                },
                Err(error) => StartupConfig {
                    entry: None,
                    input,
                    error: Some(error),
                },
            }
        }
        Err(memory_error) => StartupConfig {
            entry: None,
            input: fallback_input,
            error: Some(fallback_error.unwrap_or_else(|| {
                format!("No valid configuration directory found; enter one ({memory_error})")
            })),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_config_dir(parent: &Path, name: &str) -> PathBuf {
        let path = parent.join(name);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("config.yml"), "listenGrpcMultiaddr: test\n").unwrap();
        std::fs::write(path.join("keys.yml"), "null:\n").unwrap();
        path
    }

    #[test]
    fn parses_supported_arguments_and_help() {
        assert_eq!(
            parse_args(["--config-dir", "/tmp/node one"]).unwrap(),
            RunOptions::Run {
                explicit_config: Some(PathBuf::from("/tmp/node one"))
            }
        );
        assert_eq!(
            parse_args(["--config-dir=/tmp/node two"]).unwrap(),
            RunOptions::Run {
                explicit_config: Some(PathBuf::from("/tmp/node two"))
            }
        );
        assert_eq!(
            parse_args(["-c", "/tmp/node three"]).unwrap(),
            RunOptions::Run {
                explicit_config: Some(PathBuf::from("/tmp/node three"))
            }
        );
        assert_eq!(parse_args(["--help"]).unwrap(), RunOptions::Help);
        assert_eq!(parse_args(["-h"]).unwrap(), RunOptions::Help);
    }

    #[test]
    fn repeated_config_argument_uses_last_value() {
        assert_eq!(
            parse_args(["-c", "first", "--config-dir=second"]).unwrap(),
            RunOptions::Run {
                explicit_config: Some(PathBuf::from("second"))
            }
        );
    }

    #[test]
    fn rejects_missing_values_and_unknown_arguments_with_usage() {
        for args in [vec!["--config-dir"], vec!["-c"], vec!["--wat"]] {
            let error = parse_args(args).unwrap_err();
            assert!(error.contains(USAGE), "unexpected error: {error}");
        }
    }

    #[test]
    fn validates_complete_directory_and_preserves_spaces() {
        let temp = tempfile::tempdir().unwrap();
        let path = create_config_dir(temp.path(), "node with spaces");

        let entry = validate_config_dir(&path).unwrap();

        assert_eq!(entry.name, "node with spaces");
        assert_eq!(entry.path, path.canonicalize().unwrap());
        assert!(entry.is_default);
    }

    #[test]
    fn rejects_missing_directory_and_each_missing_required_file() {
        let temp = tempfile::tempdir().unwrap();
        assert!(validate_config_dir(&temp.path().join("missing")).is_err());

        let missing_config = temp.path().join("missing-config");
        std::fs::create_dir(&missing_config).unwrap();
        std::fs::write(missing_config.join("keys.yml"), "null:\n").unwrap();
        assert!(validate_config_dir(&missing_config)
            .unwrap_err()
            .contains("config.yml"));

        let missing_keys = temp.path().join("missing-keys");
        std::fs::create_dir(&missing_keys).unwrap();
        std::fs::write(missing_keys.join("config.yml"), "test\n").unwrap();
        assert!(validate_config_dir(&missing_keys)
            .unwrap_err()
            .contains("keys.yml"));
    }

    #[test]
    fn remembers_and_replaces_a_valid_config_directory() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let first = create_config_dir(temp.path(), "first node");
        let second = create_config_dir(temp.path(), "second node");

        remember_config_dir(&home, &first).unwrap();
        assert_eq!(
            read_remembered_config_dir(&home).unwrap(),
            first.canonicalize().unwrap()
        );
        remember_config_dir(&home, &second).unwrap();
        assert_eq!(
            read_remembered_config_dir(&home).unwrap(),
            second.canonicalize().unwrap()
        );
        assert_eq!(
            settings_file(&home),
            home.join(".qclient-console/config-dir")
        );
    }

    #[test]
    fn remembered_path_preserves_spaces_at_both_ends_of_directory_name() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let path = create_config_dir(temp.path(), " node ");

        remember_config_dir(&home, &path).unwrap();

        assert_eq!(
            read_remembered_config_dir(&home).unwrap(),
            path.canonicalize().unwrap()
        );
    }

    #[test]
    fn invalid_config_never_overwrites_previous_memory() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let valid = create_config_dir(temp.path(), "valid");
        remember_config_dir(&home, &valid).unwrap();

        assert!(remember_config_dir(&home, &temp.path().join("missing")).is_err());
        assert_eq!(
            read_remembered_config_dir(&home).unwrap(),
            valid.canonicalize().unwrap()
        );
    }

    #[test]
    fn failed_atomic_replacement_preserves_previous_memory() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let first = create_config_dir(temp.path(), "first");
        let second = create_config_dir(temp.path(), "second");
        remember_config_dir(&home, &first).unwrap();
        std::fs::create_dir(
            settings_file(&home).with_file_name(format!("config-dir.{}.tmp", std::process::id())),
        )
        .unwrap();

        assert!(remember_config_dir(&home, &second).is_err());
        assert_eq!(
            read_remembered_config_dir(&home).unwrap(),
            first.canonicalize().unwrap()
        );
    }

    #[test]
    fn failed_replacement_cleans_up_its_process_specific_temporary_file() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let config = create_config_dir(temp.path(), "node");
        let settings = settings_file(&home);
        std::fs::create_dir_all(&settings).unwrap();
        let temporary = settings.with_file_name(format!("config-dir.{}.tmp", std::process::id()));

        assert!(remember_config_dir(&home, &config).is_err());
        assert!(!temporary.exists());
    }

    #[test]
    fn startup_priority_is_explicit_then_local_then_remembered() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("cwd");
        let home = temp.path().join("home");
        std::fs::create_dir_all(&cwd).unwrap();
        let explicit = create_config_dir(temp.path(), "explicit");
        let local = create_config_dir(&cwd, ".config");
        let remembered = create_config_dir(temp.path(), "remembered");
        remember_config_dir(&home, &remembered).unwrap();

        let selected = select_startup_config(Some(explicit.clone()), &cwd, &home);
        assert_eq!(
            selected.entry.unwrap().path,
            explicit.canonicalize().unwrap()
        );

        let selected = select_startup_config(None, &cwd, &home);
        assert_eq!(selected.entry.unwrap().path, local.canonicalize().unwrap());

        std::fs::remove_file(local.join("keys.yml")).unwrap();
        let selected = select_startup_config(None, &cwd, &home);
        assert_eq!(
            selected.entry.unwrap().path,
            remembered.canonicalize().unwrap()
        );
    }

    #[test]
    fn invalid_explicit_config_does_not_fall_back() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("cwd");
        let home = temp.path().join("home");
        std::fs::create_dir_all(&cwd).unwrap();
        create_config_dir(&cwd, ".config");
        let missing = temp.path().join("missing explicit");

        let selected = select_startup_config(Some(missing.clone()), &cwd, &home);

        assert_eq!(selected.entry, None);
        assert_eq!(selected.input, missing.to_string_lossy());
        assert!(selected.error.is_some());
    }

    #[test]
    fn empty_or_broken_memory_enters_input_state() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("cwd");
        let home = temp.path().join("home");
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::create_dir_all(settings_file(&home).parent().unwrap()).unwrap();

        std::fs::write(settings_file(&home), "").unwrap();
        let empty = select_startup_config(None, &cwd, &home);
        assert_eq!(empty.entry, None);
        assert!(empty.input.is_empty());
        assert!(empty.error.unwrap().contains("configuration"));

        let broken = temp.path().join("broken remembered");
        std::fs::write(settings_file(&home), broken.to_string_lossy().as_bytes()).unwrap();
        let selected = select_startup_config(None, &cwd, &home);
        assert_eq!(selected.entry, None);
        assert_eq!(selected.input, broken.to_string_lossy());
        assert!(selected.error.unwrap().contains("configuration"));
    }
}
