use crate::i18n::UiText;
use std::path::{Path, PathBuf};

use crate::{app, config, i18n, service::read_rpc, settings};

pub(super) fn prepare_config_path(
    _home: &Path,
    path: &Path,
) -> Result<config::ConfigEntry, String> {
    config::validate_config_dir(path)
}

pub(super) fn initialize_app(
    options: config::RunOptions,
    cwd: &Path,
    home: &Path,
) -> (app::App, Option<config::ConfigEntry>) {
    let mut app = app::App::default();
    app.set_diagnostics_identity_path(home);
    match settings::load(home) {
        Ok(settings) => app.set_console_settings(settings),
        Err(error) => app.set_console_settings_load_error(error),
    }

    let explicit_config = match options {
        config::RunOptions::Run { explicit_config } => explicit_config,
        config::RunOptions::Help => return (app, None),
    };
    let startup = config::select_startup_config(explicit_config, cwd, home);

    match startup.entry {
        Some(entry) => match prepare_config_path(home, &entry.path) {
            Ok(entry) => {
                app.set_configs(vec![entry.clone()]);
                (app, Some(entry))
            }
            Err(error) => {
                app.require_config_path(startup.input, Some(error));
                (app, None)
            }
        },
        None => {
            app.require_config_path(startup.input, startup.error);
            (app, None)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PendingConfigWork {
    Validate(String),
    Refresh(config::ConfigEntry),
}

#[derive(Debug)]
pub(super) enum ConfigPathWorkerResult {
    Finished(Result<config::ConfigEntry, String>),
    Fatal(String),
}

#[derive(Debug)]
pub(super) struct SettingsWorkerResult {
    pub generation: u64,
    pub candidate: settings::ConsoleSettings,
    pub result: Result<(), String>,
}

#[derive(Debug)]
pub(super) struct ReadRpcWorkerResult {
    pub generation: u64,
    pub result: Result<app::ReadRpcSelection, String>,
}

pub(super) fn dispatch_console_settings(
    app: &mut app::App,
    home: &Path,
    tx: &tokio::sync::mpsc::Sender<SettingsWorkerResult>,
) {
    let Some(request) = app.take_console_settings_request() else {
        return;
    };
    let home = home.to_path_buf();
    let tx = tx.clone();
    tokio::spawn(async move {
        let candidate = request.settings;
        let result = tokio::task::spawn_blocking(move || settings::save(&home, candidate))
            .await
            .unwrap_or_else(|error| Err(format!("Console settings save task failed: {error}")));
        let _ = tx
            .send(SettingsWorkerResult {
                generation: request.generation,
                candidate,
                result,
            })
            .await;
    });
}

pub(super) fn consume_settings_worker_result(app: &mut app::App, result: SettingsWorkerResult) {
    app.apply_console_settings_result(result.generation, result.result.map(|()| result.candidate));
}

pub(super) fn dispatch_read_rpc(
    app: &mut app::App,
    tx: &tokio::sync::mpsc::Sender<ReadRpcWorkerResult>,
) {
    let Some(request) = app.take_read_rpc_request() else {
        return;
    };
    let tx = tx.clone();
    tokio::spawn(async move {
        let generation = request.generation;
        let result = tokio::task::spawn_blocking(move || {
            let config =
                quil_config::load_config(&request.config_dir).map_err(|error| error.to_string())?;
            if let Some(value) = request.value {
                read_rpc::save(&request.config_dir, value.as_deref())
                    .map_err(|error| error.to_string())?;
            }
            let override_address =
                read_rpc::load_override(&request.config_dir).map_err(|error| error.to_string())?;
            let effective_address = override_address
                .clone()
                .unwrap_or_else(|| config.effective_listen_grpc_multiaddr().into());
            Ok(app::ReadRpcSelection {
                override_address,
                effective_address,
            })
        })
        .await
        .unwrap_or_else(|error| Err(format!("Read RPC settings task failed: {error}")));
        let _ = tx.send(ReadRpcWorkerResult { generation, result }).await;
    });
}

pub(super) fn consume_read_rpc_worker_result(app: &mut app::App, result: ReadRpcWorkerResult) {
    app.apply_read_rpc_result(result.generation, result.result);
}

pub(super) fn take_pending_config_work(
    app: &mut app::App,
    node_in_flight: bool,
) -> Option<PendingConfigWork> {
    if node_in_flight {
        return None;
    }
    if let Some(request) = app.take_config_path_request() {
        return Some(PendingConfigWork::Validate(request.path));
    }
    if app.config_path_in_flight {
        return None;
    }
    app.take_config_selection().map(PendingConfigWork::Refresh)
}

pub(super) fn start_config_path_validation(
    home: PathBuf,
    path: PathBuf,
    locale: i18n::Locale,
    tx: tokio::sync::mpsc::Sender<ConfigPathWorkerResult>,
) {
    spawn_config_task(move || prepare_config_path(&home, &path), locale, tx);
}

pub(super) fn spawn_config_task<F>(
    work: F,
    locale: i18n::Locale,
    tx: tokio::sync::mpsc::Sender<ConfigPathWorkerResult>,
) where
    F: FnOnce() -> Result<config::ConfigEntry, String> + Send + 'static,
{
    tokio::spawn(async move {
        let result = match tokio::task::spawn_blocking(work).await {
            Ok(result) => ConfigPathWorkerResult::Finished(result),
            Err(error) => ConfigPathWorkerResult::Fatal(format!(
                "{}: {error}",
                i18n::t(locale, UiText::ConfigurationValidationTaskFailed)
            )),
        };
        let _ = tx.send(result).await;
    });
}

pub(super) fn begin_config_path_validation(app: &mut app::App) {
    if app.page != app::Page::ConfigPath {
        app.open(app::Page::ConfigPath);
    } else if app.route_stack.last() != Some(&app::Page::ConfigPath) {
        app.route_stack.push(app::Page::ConfigPath);
    }
    app.config_path_in_flight = true;
}

pub(super) fn apply_config_path_result(
    app: &mut app::App,
    result: Result<config::ConfigEntry, String>,
) -> Option<config::ConfigEntry> {
    app.config_path_in_flight = false;
    match result {
        Ok(entry) => Some(entry),
        Err(error) => {
            app.apply_config_path_error(error);
            None
        }
    }
}

pub(super) fn consume_config_path_worker_result(
    app: &mut app::App,
    result: ConfigPathWorkerResult,
) -> anyhow::Result<Option<config::ConfigEntry>> {
    match result {
        ConfigPathWorkerResult::Finished(result) => Ok(apply_config_path_result(app, result)),
        ConfigPathWorkerResult::Fatal(message) => Err(anyhow::Error::msg(message)),
    }
}
