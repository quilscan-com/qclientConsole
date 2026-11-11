//! qclient-compatible command-line entry point owned by the Console binary.
//!
//! Command implementations remain in the `quil-client` dependency. This
//! module only owns the root parser because the dependency exposes its command
//! groups, but not the parser from its binary target.

use std::ffi::OsString;

use clap::{Args, CommandFactory, Parser, Subcommand};
use quil_client::{commands, context::GlobalArgs};

#[derive(Debug, Args)]
pub struct GlobalFlags {
    /// Also print the derivation details used for diagnostics and bug reports.
    #[arg(short = 'v', long = "verbose", global = true, default_value_t = false)]
    verbose: bool,

    #[arg(long, global = true, env = "QUILIBRIUM_NETWORK")]
    network: Option<String>,

    /// Verify this executable against its signed digest before running.
    #[arg(
        long,
        global = true,
        env = "QUILIBRIUM_SIGNATURE_CHECK",
        default_value_t = true,
        action = clap::ArgAction::Set,
    )]
    signature_check: bool,

    #[arg(short = 'y', long = "yes", global = true, default_value_t = false)]
    yes: bool,
}

impl From<&GlobalFlags> for GlobalArgs {
    fn from(flags: &GlobalFlags) -> Self {
        Self {
            network: flags.network.clone(),
            yes: flags.yes,
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "qclient-console",
    about = "Quilibrium client",
    long_about = "Quilibrium client is a command-line tool for managing Quilibrium nodes.\n\
                  It provides commands for installing, updating, and managing Quilibrium nodes.",
    version = quil_config::display_version()
)]
pub struct Cli {
    #[command(flatten)]
    global: GlobalFlags,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Performs a token operation.
    Token(commands::token::TokenArgs),
    /// Performs a QClient configuration operation.
    Config {
        #[command(subcommand)]
        command: commands::config::ConfigCommand,
    },
    /// Node management (prover status/lifecycle, install/service, …).
    Node {
        #[command(subcommand)]
        command: commands::node::NodeCommand,
    },
    /// Manage address aliases.
    Alias {
        #[command(subcommand)]
        command: commands::alias::AliasCommand,
    },
    /// Hypergraph operations.
    Hypergraph {
        #[command(subcommand)]
        command: commands::hypergraph::HypergraphCommand,
    },
    /// Key management operations.
    Key {
        #[command(subcommand)]
        command: commands::key::KeyCommand,
    },
    /// Messaging operations.
    Message(commands::message::MessageArgs),
    /// Deploy schemas, tokens, and compute intrinsics.
    Deploy {
        #[command(subcommand)]
        command: commands::deploy::DeployCommand,
    },
    /// Compute operations.
    Compute(commands::compute::ComputeArgs),
    /// Download and install the latest (or a specific) qclient-console release.
    Update {
        version: Option<String>,
        #[arg(long)]
        allow_downgrade: bool,
    },
    /// Download the release signature files for the qclient-console binary.
    DownloadSignatures {
        #[arg(long)]
        version: Option<String>,
    },
    /// Create /usr/local/bin/qclient-console (requires sudo).
    Link,
    /// Diagnostics identities: ask other nodes for diagnostics without revealing this node's identity.
    Diagnostics {
        #[command(subcommand)]
        command: commands::node::diagnostics_logs::IdentityCommand,
    },
}

pub fn command() -> clap::Command {
    Cli::command()
}

pub fn try_parse_from<I, T>(args: I) -> Result<Cli, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    Cli::try_parse_from(args)
}

pub fn execute(cli: Cli) -> anyhow::Result<()> {
    if let Some(message) = quil_lattice_ct::native_cpu::unsupported_cpu_message("qclient-console") {
        anyhow::bail!(message);
    }
    quil_client::init_logging();
    quil_client::verbosity::set(cli.global.verbose);
    signature_check(&cli)?;
    let global = GlobalArgs::from(&cli.global);

    match &cli.command {
        Commands::Config { command } => commands::config::run(command),
        Commands::Alias { command } => commands::alias::run(global, command),
        Commands::Key { command } => commands::key::run(global, command),
        Commands::Link => crate::console_release::link(),
        Commands::Diagnostics { command } => {
            commands::node::diagnostics_logs::run_identity(command)
        }
        Commands::Token(args) => runtime()?.block_on(commands::token::run(global, args)),
        Commands::Node { command } => runtime()?.block_on(commands::node::run(global, command)),
        Commands::Hypergraph { command } => {
            runtime()?.block_on(commands::hypergraph::run(global, command))
        }
        Commands::Message(args) => runtime()?.block_on(commands::message::run(global, args)),
        Commands::Deploy { command } => runtime()?.block_on(commands::deploy::run(global, command)),
        Commands::Compute(args) => runtime()?.block_on(commands::compute::run(global, args)),
        Commands::Update {
            version,
            allow_downgrade,
        } => runtime()?.block_on(crate::console_release::update(
            version.as_deref(),
            *allow_downgrade,
        )),
        Commands::DownloadSignatures { version } => runtime()?.block_on(
            crate::console_release::download_signatures(version.as_deref()),
        ),
    }
}

/// Every command checks the running Console release unless explicitly
/// disabled; `download-signatures` fetches the files needed by that check.
pub fn wants_signature_check(cli: &Cli) -> bool {
    cli.global.signature_check && !matches!(cli.command, Commands::DownloadSignatures { .. })
}

fn signature_check(cli: &Cli) -> anyhow::Result<()> {
    crate::console_release::verify_current_executable(wants_signature_check(cli))
}

pub fn run_process_from<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = match try_parse_from(args) {
        Ok(cli) => cli,
        Err(error) => {
            let exit_code = error.exit_code();
            let _ = error.print();
            return exit_code;
        }
    };
    match execute(cli) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("Error: {error:#}");
            1
        }
    }
}

fn runtime() -> anyhow::Result<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?)
}
