use std::{
    fs::{self, OpenOptions},
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::i18n::Locale;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WebSettings {
    pub enabled: bool,
    pub port: u16,
}

impl Default for WebSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            port: 8888,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConsoleSettings {
    #[serde(flatten)]
    pub web: WebSettings,
    pub language: Locale,
}

impl Default for ConsoleSettings {
    fn default() -> Self {
        Self {
            web: WebSettings::default(),
            language: Locale::English,
        }
    }
}

pub fn settings_file(home: &Path) -> PathBuf {
    home.join(".qclient-console").join("settings.yml")
}

pub fn parse_port(input: &str) -> Result<u16, String> {
    input
        .parse::<u16>()
        .ok()
        .filter(|port| *port != 0)
        .ok_or_else(|| "Web port must be an integer from 1 to 65535".to_owned())
}

pub fn load(home: &Path) -> Result<ConsoleSettings, String> {
    let path = settings_file(home);
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(ConsoleSettings::default()),
        Err(error) => {
            return Err(format!(
                "Cannot read console settings {}: {error}",
                path.display()
            ))
        }
    };
    let settings: ConsoleSettings = serde_yaml::from_str(&contents)
        .map_err(|error| format!("Cannot parse console settings {}: {error}", path.display()))?;
    parse_port(&settings.web.port.to_string())?;
    Ok(settings)
}

pub fn save(home: &Path, settings: ConsoleSettings) -> Result<(), String> {
    parse_port(&settings.web.port.to_string())?;
    let contents = serde_yaml::to_string(&settings)
        .map_err(|error| format!("Cannot serialize console settings: {error}"))?;
    let path = settings_file(home);
    let parent = path
        .parent()
        .ok_or_else(|| "Cannot determine the console settings parent directory".to_owned())?;
    fs::create_dir_all(parent).map_err(|error| {
        format!(
            "Cannot create console settings directory {}: {error}",
            parent.display()
        )
    })?;
    let temporary = path.with_file_name(format!("settings.yml.{}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| {
            format!(
                "Cannot create temporary console settings {}: {error}",
                temporary.display()
            )
        })?;
    let result = (|| {
        file.write_all(contents.as_bytes()).map_err(|error| {
            format!(
                "Cannot write temporary console settings {}: {error}",
                temporary.display()
            )
        })?;
        file.sync_all().map_err(|error| {
            format!(
                "Cannot sync temporary console settings {}: {error}",
                temporary.display()
            )
        })?;
        drop(file);
        fs::rename(&temporary, &path).map_err(|error| {
            format!(
                "Cannot replace console settings {}: {error}",
                path.display()
            )
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn enabled_setting_loads_and_defaults_to_english() {
        let home = tempfile::tempdir().unwrap();
        let path = settings_file(home.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "enabled: true\nport: 9000\n").unwrap();

        let settings = load(home.path()).unwrap();

        assert!(settings.web.enabled);
        assert_eq!(settings.web.port, 9000);
        assert_eq!(settings.language, Locale::English);
    }

    #[test]
    fn defaults_to_enabled_port_8888() {
        assert_eq!(
            ConsoleSettings::default(),
            ConsoleSettings {
                web: WebSettings {
                    enabled: true,
                    port: 8888,
                },
                language: Locale::English,
            }
        );
    }

    #[test]
    fn all_supported_languages_round_trip_through_yaml() {
        for (locale, encoded) in [
            (Locale::English, "en"),
            (Locale::TraditionalChinese, "zh-TW"),
            (Locale::Japanese, "ja"),
            (Locale::French, "fr"),
        ] {
            let settings = ConsoleSettings {
                language: locale,
                ..ConsoleSettings::default()
            };
            let yaml = serde_yaml::to_string(&settings).unwrap();
            assert!(yaml.contains(&format!("language: {encoded}")));
            assert_eq!(
                serde_yaml::from_str::<ConsoleSettings>(&yaml).unwrap(),
                settings
            );
        }
    }

    #[test]
    fn validates_port_boundaries() {
        assert_eq!(parse_port("1"), Ok(1));
        assert_eq!(parse_port("65535"), Ok(65535));
        for value in ["", "0", "65536", "abc"] {
            assert!(parse_port(value).is_err(), "accepted {value:?}");
        }
    }

    #[test]
    fn missing_file_returns_defaults_without_creating_it() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(
            settings_file(home.path()),
            home.path().join(".qclient-console/settings.yml")
        );
        assert_eq!(load(home.path()).unwrap(), ConsoleSettings::default());
        assert!(!settings_file(home.path()).exists());
    }

    #[test]
    fn invalid_settings_are_reported_without_overwriting_the_file() {
        let home = tempfile::tempdir().unwrap();
        let path = settings_file(home.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        for contents in [
            "enabled: [broken",
            "enabled: true\nport: 9000\nunknown: 1\n",
            "port: 0\n",
            "port: 65536\n",
        ] {
            fs::write(&path, contents).unwrap();
            assert!(load(home.path()).is_err(), "accepted {contents:?}");
            assert_eq!(fs::read_to_string(&path).unwrap(), contents);
        }
    }

    #[test]
    fn missing_yaml_fields_use_defaults() {
        let home = tempfile::tempdir().unwrap();
        let path = settings_file(home.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "enabled: true\n").unwrap();
        assert_eq!(
            load(home.path()).unwrap(),
            ConsoleSettings {
                web: WebSettings {
                    enabled: true,
                    port: 8888,
                },
                language: Locale::English,
            }
        );
    }

    #[test]
    fn save_creates_parent_and_atomically_overwrites_settings_without_temporary_files() {
        let home = tempfile::tempdir().unwrap();
        for settings in [
            ConsoleSettings {
                web: WebSettings {
                    enabled: true,
                    port: 1,
                },
                language: Locale::TraditionalChinese,
            },
            ConsoleSettings {
                web: WebSettings {
                    enabled: false,
                    port: 65535,
                },
                language: Locale::English,
            },
        ] {
            save(home.path(), settings).unwrap();
            assert_eq!(load(home.path()).unwrap(), settings);
            let entries: Vec<_> = fs::read_dir(settings_file(home.path()).parent().unwrap())
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect();
            assert_eq!(entries, vec![std::ffi::OsString::from("settings.yml")]);
        }
    }

    #[test]
    fn failed_rename_cleans_up_the_temporary_file() {
        let home = tempfile::tempdir().unwrap();
        let path = settings_file(home.path());
        fs::create_dir_all(&path).unwrap();
        assert!(save(home.path(), ConsoleSettings::default()).is_err());
        assert!(path.is_dir());
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    }

    #[test]
    fn invalid_port_is_not_saved_over_existing_settings() {
        let home = tempfile::tempdir().unwrap();
        save(home.path(), ConsoleSettings::default()).unwrap();
        let path = settings_file(home.path());
        let original = fs::read(&path).unwrap();
        assert!(save(
            home.path(),
            ConsoleSettings {
                web: WebSettings {
                    enabled: true,
                    port: 0,
                },
                language: Locale::English,
            }
        )
        .is_err());
        assert_eq!(fs::read(path).unwrap(), original);
    }
}
