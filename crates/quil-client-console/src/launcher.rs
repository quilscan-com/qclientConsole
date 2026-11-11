//! Select Console mode before starting any terminal, core, or Web services.

use std::ffi::OsString;
use std::path::PathBuf;

use clap::error::ErrorKind;
use clap::Parser;

use crate::config::{self, RunOptions};

#[derive(Debug, Parser)]
#[command(
    name = "qclient-console",
    args_override_self = true,
    disable_help_flag = true,
    disable_version_flag = true
)]
struct ConsoleArgs {
    #[arg(short = 'c', long = "config-dir", value_name = "PATH")]
    config_dir: Option<PathBuf>,

    #[arg(
        long,
        env = "QUILIBRIUM_SIGNATURE_CHECK",
        default_value_t = true,
        action = clap::ArgAction::Set
    )]
    signature_check: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchMode {
    Console {
        options: RunOptions,
        signature_check: bool,
    },
    CombinedHelp,
    Qclient(Vec<OsString>),
}

/// Route a complete argv, preserving every value passed to the shared CLI.
pub fn route_args<I, T>(args: I) -> Result<LaunchMode, String>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let args: Vec<OsString> = args.into_iter().map(Into::into).collect();
    let user_args = args.get(1..).unwrap_or_default();
    if user_args.len() == 1 && (user_args[0] == "--help" || user_args[0] == "-h") {
        return Ok(LaunchMode::CombinedHelp);
    }

    // Once a real qclient command is present, qclient owns the complete argv,
    // including malformed shared options. This preserves Clap's qclient error
    // kind and exit code instead of turning it into a Console parse error.
    if contains_qclient_subcommand(user_args) {
        return Ok(LaunchMode::Qclient(args));
    }

    match ConsoleArgs::try_parse_from(args.iter().cloned()) {
        Ok(console) => {
            if console
                .config_dir
                .as_ref()
                .is_some_and(|path| path.as_os_str().is_empty())
            {
                return Err(format!(
                    "Argument --config-dir requires a path\n\n{}",
                    config::USAGE
                ));
            }
            Ok(LaunchMode::Console {
                options: RunOptions::Run {
                    explicit_config: console.config_dir,
                },
                signature_check: console.signature_check,
            })
        }
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::UnknownArgument | ErrorKind::InvalidSubcommand
            ) =>
        {
            Ok(LaunchMode::Qclient(args))
        }
        Err(error) => Err(console_parse_error(user_args, error)),
    }
}

fn contains_qclient_subcommand(user_args: &[OsString]) -> bool {
    let commands: Vec<_> = crate::qclient::command()
        .get_subcommands()
        .map(|command| command.get_name().to_owned())
        .collect();
    let mut config_value = false;

    for argument in user_args {
        if config_value {
            config_value = false;
            continue;
        }
        if argument == "-c" || argument == "--config-dir" {
            config_value = true;
            continue;
        }
        if argument
            .to_str()
            .is_some_and(|argument| argument.starts_with("--config-dir="))
        {
            continue;
        }
        if argument
            .to_str()
            .is_some_and(|argument| commands.iter().any(|command| command == argument))
        {
            return true;
        }
    }

    false
}

fn console_parse_error(user_args: &[OsString], error: clap::Error) -> String {
    let missing_config_path = user_args.last().is_some_and(|argument| {
        argument == "-c"
            || argument == "--config-dir"
            || argument
                .to_str()
                .is_some_and(|argument| argument == "--config-dir=")
    });
    if missing_config_path {
        return format!("Argument --config-dir requires a path\n\n{}", config::USAGE);
    }
    error.to_string()
}

/// Run either entry point without terminating the calling process.
pub fn run_process_from<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let args: Vec<OsString> = args.into_iter().map(Into::into).collect();
    let args =
        match crate::capability::protocol::restore_process_args(args, std::io::stdin().lock()) {
            Ok(args) => args,
            Err(error) => {
                eprintln!("Error: {error}");
                return 1;
            }
        };
    match route_args(args.iter().cloned()) {
        Ok(LaunchMode::Qclient(args)) => crate::qclient::run_process_from(args),
        Ok(LaunchMode::CombinedHelp) => {
            let program = args
                .first()
                .map(|name| name.to_string_lossy())
                .unwrap_or_else(|| "qclient-console".into());
            println!("{}", combined_help(&program));
            0
        }
        Ok(LaunchMode::Console {
            options,
            signature_check,
        }) => {
            if let Err(error) = crate::console_release::verify_current_executable(signature_check) {
                eprintln!("Error: {error:#}");
                return 1;
            }
            match crate::controller::run_with_signature_check(options, signature_check) {
                Ok(()) => 0,
                Err(error) => {
                    eprintln!("Error: {error:#}");
                    1
                }
            }
        }
        Err(error) => {
            eprintln!("Error: {error}");
            1
        }
    }
}

fn combined_help(program: &str) -> String {
    let usage = config::USAGE.replace("qclient-console", program);
    let shared_help = crate::qclient::command()
        .bin_name(program)
        .render_long_help();
    format!(
        "{usage}\n\nConsole mode: no arguments or Console configuration flags start the TUI and configured Web listener.\n\n{shared_help}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RunOptions;
    use std::{ffi::OsString, path::PathBuf};

    #[test]
    fn no_args_and_console_config_flags_open_console() {
        for (args, path) in [
            (vec!["console"], None),
            (
                vec!["console", "-c", "/tmp/node one"],
                Some("/tmp/node one"),
            ),
            (vec!["console", "--config-dir", "node"], Some("node")),
            (vec!["console", "--config-dir=node"], Some("node")),
            (
                vec!["console", "-c", "first", "--config-dir=last"],
                Some("last"),
            ),
        ] {
            assert_eq!(
                route_args(args).unwrap(),
                LaunchMode::Console {
                    options: RunOptions::Run {
                        explicit_config: path.map(PathBuf::from),
                    },
                    signature_check: true,
                }
            );
        }
    }

    #[test]
    fn every_qclient_command_routes_without_starting_console() {
        for command in [
            "token",
            "config",
            "node",
            "alias",
            "hypergraph",
            "key",
            "message",
            "deploy",
            "compute",
            "update",
            "download-signatures",
            "link",
            "diagnostics",
        ] {
            let args: Vec<OsString> = ["console", command].into_iter().map(Into::into).collect();
            assert_eq!(route_args(args.clone()).unwrap(), LaunchMode::Qclient(args));
        }
    }

    #[test]
    fn root_help_is_combined_but_subcommand_help_stays_qclient() {
        for flag in ["-h", "--help"] {
            assert_eq!(
                route_args(["console", flag]).unwrap(),
                LaunchMode::CombinedHelp
            );
            let args: Vec<OsString> = ["console", "token", flag]
                .into_iter()
                .map(Into::into)
                .collect();
            assert_eq!(route_args(args.clone()).unwrap(), LaunchMode::Qclient(args));
        }
    }

    #[test]
    fn malformed_console_config_flag_is_a_console_error() {
        for args in [
            vec!["console", "-c"],
            vec!["console", "--config-dir"],
            vec!["console", "--config-dir="],
        ] {
            let error = route_args(args).unwrap_err();
            assert!(error.contains("requires a path"), "{error}");
            assert!(error.contains(crate::config::USAGE), "{error}");
        }
    }

    #[test]
    fn unknown_first_argument_and_mixed_modes_are_forwarded_unchanged() {
        for args in [
            vec!["console", "not-a-command"],
            vec!["console", "--wat"],
            vec!["console", "--network", "mainnet", "token", "--help"],
            vec!["console", "-c", "node", "token"],
            vec!["console", "--config-dir=node", "--help"],
            vec!["console", "--help", "token"],
        ] {
            let args: Vec<OsString> = args.into_iter().map(Into::into).collect();
            assert_eq!(route_args(args.clone()).unwrap(), LaunchMode::Qclient(args));
        }
    }

    #[cfg(unix)]
    #[test]
    fn forwarding_preserves_non_utf8_arguments_including_argv_zero() {
        use std::os::unix::ffi::OsStringExt;
        let args = vec![
            OsString::from_vec(b"custom-\xff-console".to_vec()),
            OsString::from("token"),
            OsString::from_vec(b"value-\xfe".to_vec()),
            OsString::new(),
        ];
        assert_eq!(route_args(args.clone()).unwrap(), LaunchMode::Qclient(args));
    }

    #[cfg(unix)]
    #[test]
    fn console_config_path_preserves_non_utf8() {
        use std::os::unix::ffi::OsStringExt;
        let path = OsString::from_vec(b"node-\xff".to_vec());
        assert_eq!(
            route_args([
                OsString::from("console"),
                OsString::from("-c"),
                path.clone()
            ])
            .unwrap(),
            LaunchMode::Console {
                options: RunOptions::Run {
                    explicit_config: Some(PathBuf::from(path)),
                },
                signature_check: true,
            }
        );
    }

    #[test]
    fn combined_help_includes_console_modes_then_shared_commands_and_program_name() {
        let help = combined_help("custom-console");
        assert!(
            help.contains("Usage: custom-console [--config-dir"),
            "{help}"
        );
        assert!(help.contains("TUI") && help.contains("Web"), "{help}");
        let commands = help.find("Commands:").expect("shared command help");
        assert!(help.find("--config-dir").unwrap() < commands);
        for command in ["token", "node", "hypergraph", "diagnostics"] {
            assert!(help[commands..].contains(command), "{help}");
        }
        assert!(!help.contains("Usage: qclient "), "{help}");
    }

    #[test]
    fn process_adapter_returns_console_error_and_qclient_help_and_parse_codes() {
        assert_eq!(run_process_from(["console", "-c"]), 1);
        assert_eq!(run_process_from(["console", "--help"]), 0);
        assert_eq!(run_process_from(["console", "token", "--help"]), 0);
        assert_eq!(run_process_from(["console", "not-a-command"]), 2);
    }
}
