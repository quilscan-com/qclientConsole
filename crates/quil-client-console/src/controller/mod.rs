use crate::i18n::UiText;
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::{
    app, config, domain, event, i18n, service,
    service::ConsoleService,
    settings::{self, ConsoleSettings, WebSettings},
    terminal, ui,
    web::{WebServer, WebServerHandle},
};

mod api;
mod configuration;
mod input;

pub use api::*;
use configuration::*;
pub(crate) use input::{map_form_key, map_key};

pub fn run(options: config::RunOptions) -> anyhow::Result<()> {
    run_with_signature_check(options, true)
}

pub fn run_with_signature_check(
    options: config::RunOptions,
    signature_check: bool,
) -> anyhow::Result<()> {
    if options == config::RunOptions::Help {
        println!("{}", config::USAGE);
        return Ok(());
    }

    terminal::install_panic_restore_hook();
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run_async(options, signature_check))
}

async fn run_async(options: config::RunOptions, signature_check: bool) -> anyhow::Result<()> {
    use crossterm::event::{Event, EventStream, KeyEventKind};
    use futures::StreamExt;
    use tokio::sync::mpsc;

    let cwd = std::env::current_dir()?;
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("HOME environment variable is not set"))?;
    let service: Arc<dyn ConsoleService> =
        Arc::new(service::NodeConsoleService::with_home(home.clone()));
    let (mut app, startup_refresh) = initialize_app(options, &cwd, &home);
    let initial_config = startup_refresh.clone();
    if let Some(entry) = initial_config.as_ref() {
        app.queue_read_rpc_load(entry.path.clone());
    }
    let core = crate::core::ConsoleCore::new_with_signature_check(
        service.clone(),
        initial_config,
        signature_check,
    );
    app.capabilities
        .replace_catalog(core.capabilities().iter().cloned().collect());
    let mut web_server = None;
    if app.other.settings.enabled {
        let candidate = app.other.settings;
        let address = web_address(candidate.port);
        if let Err(error) = apply_web_settings(
            &mut web_server,
            WebSettings::default(),
            candidate,
            core.clone(),
            home.clone(),
            address,
        )
        .await
        {
            app.other.error = Some(error);
        }
    }
    let mut core_changes = core.subscribe();
    let mut terminal = terminal::TerminalGuard::enter()?;
    let size = terminal.terminal_mut().size()?;
    app::reduce_input(&mut app, event::InputEvent::Resize(size.width, size.height));
    let mut events = EventStream::new();
    let (tx, mut rx) = mpsc::channel::<event::ServiceEvent>(32);
    let (config_tx, mut config_rx) = mpsc::channel::<ConfigPathWorkerResult>(1);
    let (settings_tx, mut settings_rx) = mpsc::channel::<SettingsWorkerResult>(1);
    let (read_rpc_tx, mut read_rpc_rx) = mpsc::channel::<ReadRpcWorkerResult>(1);
    let mut in_flight = false;
    let mut current_config_activation = None;
    let mut pending_validated_config = None;
    let mut prover_write_admission = None;
    let mut refresh_pending = false;
    let mut operation_poll = tokio::time::interval(std::time::Duration::from_secs(3));
    operation_poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut reward_mint_poll = tokio::time::interval(std::time::Duration::from_secs(15));
    reward_mint_poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut reward_mint_poll_ticks = 1_u64;
    let mut legacy_migration_poll = tokio::time::interval(std::time::Duration::from_secs(15));
    legacy_migration_poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut process_shutdown = false;

    if let Some(entry) = startup_refresh {
        let _ = start_config_refresh(
            &mut app,
            &mut in_flight,
            &mut current_config_activation,
            home.clone(),
            core.clone(),
            entry,
            tx.clone(),
        );
    }

    loop {
        terminal
            .terminal_mut()
            .draw(|frame| ui::draw(frame, &app))?;
        tokio::select! {
            maybe_event = events.next() => match maybe_event {
                Some(Ok(Event::Key(key))) if key.kind != KeyEventKind::Release => {
                    let input = if app.is_text_editing() {
                        map_form_key(key)
                    } else {
                        map_key(key)
                    };
                    if let Some(input) = input {
                        if input == event::InputEvent::Refresh
                            && !page_handles_own_refresh(app.page)
                        {
                            if !app.config_path_in_flight && app.active_config().is_some() {
                                start_refresh(
                                    &mut app,
                                    &mut in_flight,
                                    core.clone(),
                                    tx.clone(),
                                );
                            }
                        } else {
                            app::reduce_input(&mut app, input);
                        }
                    }
                }
                Some(Ok(Event::Resize(width, height))) => {
                    app::reduce_input(&mut app, event::InputEvent::Resize(width, height));
                }
                Some(Err(error)) => return Err(error.into()),
                None => break,
                _ => {}
            },
            Some(message) = rx.recv() => {
                let prover_write_admission_finished = complete_prover_write_admission(
                    &message,
                    &mut prover_write_admission,
                );
                let reward_mint_finished = matches!(
                    &message,
                    event::ServiceEvent::RewardMintPreflightFinished { .. }
                        | event::ServiceEvent::RewardMintStarted { .. }
                        | event::ServiceEvent::RewardMintLoaded { .. }
                        | event::ServiceEvent::RewardMintAdvanced { .. }
                        | event::ServiceEvent::RewardMintRebroadcasted { .. }
                );
                let wallet_execution_finished = matches!(
                    &message,
                    event::ServiceEvent::WalletExecutionFinished { generation, .. }
                        if *generation == app.wallet.generation
                );
                let is_current_activation = complete_node_task(
                    &message,
                    &mut in_flight,
                    &mut current_config_activation,
                );
                if prover_write_admission_finished || wallet_execution_finished {
                    refresh_pending = true;
                }
                reduce_service_with_core_projection(
                    &mut app,
                    &mut core_changes,
                    message,
                    is_current_activation,
                );
                if reward_mint_finished {
                    reward_mint_poll_ticks = app
                        .reward_mint
                        .status
                        .as_ref()
                        .and_then(|status| status.next_poll_in_secs)
                        .unwrap_or(15)
                        .div_ceil(15)
                        .max(1);
                }
            }
            Some(result) = config_rx.recv() => {
                if let Some(entry) = consume_config_path_worker_result(&mut app, result)? {
                    pending_validated_config = Some(entry);
                }
            }
            Some(result) = settings_rx.recv() => {
                reconcile_settings_worker_result(
                    &mut app,
                    &mut web_server,
                    result,
                    core.clone(),
                    home.clone(),
                ).await;
            }
            Some(result) = read_rpc_rx.recv() => consume_read_rpc_worker_result(&mut app, result),
            changed = core_changes.changed(), if current_config_activation.is_none() => {
                if changed.is_err() {
                    break;
                }
            }
            _ = operation_poll.tick(), if app.prover_operations.has_pending() => {
                if !in_flight
                    && !app.config_path_in_flight
                    && app.active_config().is_some()
                {
                    start_refresh(
                        &mut app,
                        &mut in_flight,
                        core.clone(),
                        tx.clone(),
                    );
                }
            }
            _ = reward_mint_poll.tick(), if app.reward_mint_should_poll() => {
                if reward_mint_poll_ticks <= 1 {
                    app.queue_reward_mint_advance();
                    reward_mint_poll_ticks = 1;
                } else {
                    reward_mint_poll_ticks -= 1;
                }
            }
            _ = legacy_migration_poll.tick(), if app.legacy_migration_should_poll() => {
                app.queue_legacy_migration_advance();
            }
            _ = tokio::signal::ctrl_c() => {
                process_shutdown = true;
                break;
            },
        }
        let config_work = take_config_tail_work(
            &mut app,
            &mut core_changes,
            in_flight || current_config_activation.is_some(),
            &mut pending_validated_config,
        );
        if let Some(work) = config_work {
            if dispatch_config_work(
                &mut app,
                &mut in_flight,
                &mut current_config_activation,
                &ConfigWorkResources {
                    home: &home,
                    core: &core,
                    service_tx: &tx,
                    config_tx: &config_tx,
                },
                work,
            ) {
                refresh_pending = false;
            }
        }
        if app.take_node_refresh_request()
            && !in_flight
            && !app.config_path_in_flight
            && current_config_activation.is_none()
            && app.active_config().is_some()
        {
            start_refresh(&mut app, &mut in_flight, core.clone(), tx.clone());
        }
        if should_start_follow_up_refresh(
            in_flight || app.config_path_in_flight,
            &mut refresh_pending,
        ) && app.active_config().is_some()
        {
            start_refresh(&mut app, &mut in_flight, core.clone(), tx.clone());
        }
        dispatch_queued_work(
            &mut app,
            current_config_activation.is_some(),
            &mut prover_write_admission,
            &QueuedWorkResources {
                home: &home,
                core: &core,
                service: &service,
                service_tx: &tx,
                settings_tx: &settings_tx,
                read_rpc_tx: &read_rpc_tx,
            },
        );
        if tui_exit_ready(&app) {
            break;
        }
    }
    drop(terminal);
    if let Some(server) = web_server {
        if !process_shutdown {
            tokio::signal::ctrl_c().await?;
        }
        server.shutdown().await?;
    }
    Ok(())
}

fn tui_exit_ready(app: &app::App) -> bool {
    app.should_quit && !app.other.in_flight && !app.other.read_rpc_in_flight
}

fn web_address(port: u16) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port)
}

async fn apply_web_settings(
    server: &mut Option<WebServerHandle>,
    previous: WebSettings,
    candidate: WebSettings,
    core: crate::core::ConsoleCore,
    home: PathBuf,
    candidate_address: SocketAddr,
) -> Result<(), String> {
    if !candidate.enabled {
        if let Some(running) = server.take() {
            let _ = running.shutdown().await;
        }
        return Ok(());
    }

    if previous.enabled && previous.port == candidate.port && server.is_some() {
        return Ok(());
    }

    let replacement = WebServer::start(candidate_address, core, home)
        .await
        .map_err(|error| format!("Cannot start Web server on {candidate_address}: {error}"))?;
    if let Some(running) = server.replace(replacement) {
        let _ = running.shutdown().await;
    }
    Ok(())
}

async fn reconcile_settings_worker_result(
    app: &mut app::App,
    server: &mut Option<WebServerHandle>,
    result: SettingsWorkerResult,
    core: crate::core::ConsoleCore,
    home: PathBuf,
) {
    if result.result.is_err() || result.generation != app.other.generation || !app.other.in_flight {
        consume_settings_worker_result(app, result);
        return;
    }

    let previous = ConsoleSettings {
        web: app.other.settings,
        language: app.other.previous_locale.unwrap_or(app.locale()),
    };
    let candidate = result.candidate;
    let address = web_address(candidate.web.port);
    match apply_web_settings(
        server,
        previous.web,
        candidate.web,
        core,
        home.clone(),
        address,
    )
    .await
    {
        Ok(()) => app.apply_console_settings_result(result.generation, Ok(candidate)),
        Err(error) => {
            let rollback = tokio::task::spawn_blocking(move || settings::save(&home, previous))
                .await
                .unwrap_or_else(|join_error| {
                    Err(format!(
                        "Console settings rollback task failed: {join_error}"
                    ))
                });
            let error = match rollback {
                Ok(()) => format!("{error}; previous console settings were restored"),
                Err(rollback_error) => {
                    format!("{error}; cannot restore previous console settings: {rollback_error}")
                }
            };
            app.apply_console_settings_result(result.generation, Err(error));
        }
    }
}

fn synchronize_core_projection(
    app: &mut app::App,
    changes: &mut tokio::sync::watch::Receiver<crate::core::CoreStateSnapshot>,
) {
    let state = changes.borrow_and_update().clone();
    app.apply_core_state_projection(&state);
}

fn reduce_service_with_core_projection(
    app: &mut app::App,
    changes: &mut tokio::sync::watch::Receiver<crate::core::CoreStateSnapshot>,
    mut message: event::ServiceEvent,
    is_current_activation: bool,
) {
    let mut queue_reward_mint_load = false;
    let mut queue_read_rpc_load = false;
    if matches!(&message, event::ServiceEvent::RefreshFinished { .. }) {
        synchronize_core_projection(app, changes);
    }
    if let event::ServiceEvent::ConfigActivationFinished {
        generation, result, ..
    } = &mut message
    {
        if !is_current_activation {
            return;
        }
        queue_reward_mint_load = result.is_ok();
        queue_read_rpc_load = result.is_ok();
        synchronize_core_projection(app, changes);
        *generation = app.refresh_generation;
    }
    app::reduce_service(app, message);
    if queue_reward_mint_load {
        app.queue_reward_mint_load();
    }
    if queue_read_rpc_load {
        if let Some(config_dir) = app.active_config().map(|config| config.path.clone()) {
            app.queue_read_rpc_load(config_dir);
        }
    }
}

struct QueuedWorkResources<'a> {
    home: &'a Path,
    core: &'a crate::core::ConsoleCore,
    service: &'a Arc<dyn ConsoleService>,
    service_tx: &'a tokio::sync::mpsc::Sender<event::ServiceEvent>,
    settings_tx: &'a tokio::sync::mpsc::Sender<SettingsWorkerResult>,
    read_rpc_tx: &'a tokio::sync::mpsc::Sender<ReadRpcWorkerResult>,
}

fn dispatch_queued_work(
    app: &mut app::App,
    config_activation_in_flight: bool,
    prover_write_admission: &mut Option<ProverWriteAdmission>,
    resources: &QueuedWorkResources<'_>,
) {
    if !config_activation_in_flight {
        dispatch_queued_commands(
            app,
            resources.core,
            prover_write_admission,
            resources.service,
            resources.service_tx,
        );
        dispatch_queued_reads(app, resources.core, resources.service, resources.service_tx);
    }
    dispatch_console_settings(app, resources.home, resources.settings_tx);
    dispatch_read_rpc(app, resources.read_rpc_tx);
}

fn complete_node_task(
    message: &event::ServiceEvent,
    node_in_flight: &mut bool,
    current_config_activation: &mut Option<u64>,
) -> bool {
    match message {
        event::ServiceEvent::RefreshFinished { .. } => {
            *node_in_flight = false;
            false
        }
        event::ServiceEvent::ConfigActivationFinished { generation, .. }
            if *current_config_activation == Some(*generation) =>
        {
            *node_in_flight = false;
            *current_config_activation = None;
            true
        }
        _ => false,
    }
}

fn page_handles_own_refresh(page: app::Page) -> bool {
    match page {
        app::Page::Home | app::Page::NodeStatus | app::Page::Prover => false,
        app::Page::Section(_)
        | app::Page::Configs
        | app::Page::ConfigPath
        | app::Page::Runtime
        | app::Page::Diagnostics
        | app::Page::DiagnosticsIdentity
        | app::Page::RemoteHealth
        | app::Page::Health
        | app::Page::PeerResolve
        | app::Page::Logs
        | app::Page::LogExport
        | app::Page::Keys
        | app::Page::KeyActions
        | app::Page::KeyCreate
        | app::Page::Aliases
        | app::Page::AliasActions
        | app::Page::AliasEdit
        | app::Page::ProverActions
        | app::Page::Shards
        | app::Page::ArchiveShards
        | app::Page::Rewards
        | app::Page::RewardMintConfirm
        | app::Page::ConfidentialWallet
        | app::Page::PendingEscrows
        | app::Page::WalletActions
        | app::Page::WalletForm
        | app::Page::WalletConfirm
        | app::Page::WalletProgress
        | app::Page::WalletResult
        | app::Page::TokenTransfer
        | app::Page::Capabilities
        | app::Page::CapabilityForm
        | app::Page::CapabilityConfirm
        | app::Page::CapabilityTask
        | app::Page::LegacyAssets
        | app::Page::Hypergraph
        | app::Page::DeployToken
        | app::Page::Confirm
        | app::Page::OperationResult
        | app::Page::Other
        | app::Page::Help => true,
    }
}

fn take_config_tail_work(
    app: &mut app::App,
    changes: &mut tokio::sync::watch::Receiver<crate::core::CoreStateSnapshot>,
    node_in_flight: bool,
    pending_validated_config: &mut Option<config::ConfigEntry>,
) -> Option<PendingConfigWork> {
    synchronize_core_projection(app, changes);
    if node_in_flight {
        return None;
    }
    if let Some(entry) = pending_validated_config.take() {
        return Some(PendingConfigWork::Refresh(entry));
    }
    take_pending_config_work(app, false)
}

struct ConfigWorkResources<'a> {
    home: &'a Path,
    core: &'a crate::core::ConsoleCore,
    service_tx: &'a tokio::sync::mpsc::Sender<event::ServiceEvent>,
    config_tx: &'a tokio::sync::mpsc::Sender<ConfigPathWorkerResult>,
}

fn dispatch_config_work(
    app: &mut app::App,
    node_in_flight: &mut bool,
    current_config_activation: &mut Option<u64>,
    resources: &ConfigWorkResources<'_>,
    work: PendingConfigWork,
) -> bool {
    match work {
        PendingConfigWork::Validate(path) => {
            begin_config_path_validation(app);
            start_config_path_validation(
                resources.home.to_path_buf(),
                PathBuf::from(path),
                app.locale(),
                resources.config_tx.clone(),
            );
            false
        }
        PendingConfigWork::Refresh(entry) => start_config_refresh(
            app,
            node_in_flight,
            current_config_activation,
            resources.home.to_path_buf(),
            resources.core.clone(),
            entry,
            resources.service_tx.clone(),
        ),
    }
}

fn should_start_follow_up_refresh(in_flight: bool, refresh_pending: &mut bool) -> bool {
    if in_flight || !*refresh_pending {
        return false;
    }
    *refresh_pending = false;
    true
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PendingProverWrite {
    Foreground(app::OperationRequest),
    Tracked(app::ProverOperationRequest),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProverWriteAdmission {
    Foreground(u64),
    Tracked(u64),
}

fn take_next_prover_write(
    app: &mut app::App,
    admission: &mut Option<ProverWriteAdmission>,
) -> Option<PendingProverWrite> {
    if admission.is_some() {
        return None;
    }

    let pending = if let Some(request) = app.take_operation() {
        PendingProverWrite::Foreground(request)
    } else {
        PendingProverWrite::Tracked(app.take_prover_operation()?)
    };
    *admission = Some(match &pending {
        PendingProverWrite::Foreground(request) => {
            ProverWriteAdmission::Foreground(request.generation)
        }
        PendingProverWrite::Tracked(request) => ProverWriteAdmission::Tracked(request.id),
    });
    Some(pending)
}

fn complete_prover_write_admission(
    message: &event::ServiceEvent,
    admission: &mut Option<ProverWriteAdmission>,
) -> bool {
    let completed = matches!(
        (admission.as_ref(), message),
        (
            Some(ProverWriteAdmission::Foreground(expected)),
            event::ServiceEvent::OperationFinished { generation, .. },
        ) if expected == generation
    ) || matches!(
        (admission.as_ref(), message),
        (
            Some(ProverWriteAdmission::Tracked(expected)),
            event::ServiceEvent::ProverOperationFinished { id, .. },
        ) if expected == id
    );
    if completed {
        *admission = None;
    }
    completed
}

fn dispatch_queued_commands(
    app: &mut app::App,
    core: &crate::core::ConsoleCore,
    prover_write_admission: &mut Option<ProverWriteAdmission>,
    service: &Arc<dyn ConsoleService>,
    tx: &tokio::sync::mpsc::Sender<event::ServiceEvent>,
) {
    let config_dir = app.active_config().map(|entry| entry.path.clone());
    let missing_config = missing_config_message(app.locale());

    if let Some(request) = app.take_capability_request() {
        start_capability(core.clone(), request, tx.clone());
    }

    if let Some(pending) = take_next_prover_write(app, prover_write_admission) {
        match pending {
            PendingProverWrite::Foreground(request) => {
                start_operation(core.clone(), request, tx.clone());
            }
            PendingProverWrite::Tracked(request) => {
                start_prover_operation(core.clone(), request, tx.clone());
            }
        }
    }
    if let Some(request) = app.take_diagnostics_request() {
        start_diagnostics(service.clone(), request, tx.clone());
    }
    if let Some(request) = app.take_legacy_migration_preflight() {
        match request {
            Ok(request) => start_legacy_migration_preflight(core.clone(), request, tx.clone()),
            Err(error) => {
                app.legacy_migration.in_flight = false;
                app.legacy_migration.error = Some(error);
            }
        }
    }
    if let Some(request) = app.take_legacy_migration_start() {
        start_legacy_migration(core.clone(), request, tx.clone());
    }
    if let Some(request) = app.take_legacy_migration_rebroadcast() {
        start_legacy_migration_rebroadcast(core.clone(), request, tx.clone());
    }
    if let Some(request) = app.take_legacy_migration_file() {
        start_legacy_migration_file(request, tx.clone());
    }
    if let Some(request) = app.take_diagnostics_identity_request() {
        start_diagnostics_identity(service.clone(), request, tx.clone());
    }
    if let Some(request) = app.take_wallet_preflight_request() {
        if let Some(path) = config_dir.clone() {
            start_wallet_preflight(service.clone(), path, request, tx.clone());
        } else {
            app::reduce_service(
                app,
                event::ServiceEvent::WalletPreflightFinished {
                    generation: request.generation,
                    result: Err(missing_config.clone()),
                },
            );
        }
    }
    if let Some(request) = app.take_wallet_execute_request() {
        if let Some(path) = config_dir.clone() {
            start_wallet_execution(service.clone(), path, request, tx.clone());
        } else {
            app::reduce_service(
                app,
                event::ServiceEvent::WalletExecutionFinished {
                    generation: request.generation,
                    result: Err(missing_config.clone()),
                },
            );
        }
    }
    if let Some(request) = app.take_reward_mint_start() {
        if let Some(path) = config_dir.clone() {
            start_reward_mint(service.clone(), path, request, tx.clone());
        } else {
            app::reduce_service(
                app,
                event::ServiceEvent::RewardMintStarted {
                    generation: request.generation,
                    result: Err(missing_config.clone()),
                },
            );
        }
    }
    if let Some(request) = app.take_reward_mint_rebroadcast() {
        if let Some(path) = config_dir.clone() {
            start_reward_mint_rebroadcast(service.clone(), path, request, tx.clone());
        } else {
            app::reduce_service(
                app,
                event::ServiceEvent::RewardMintRebroadcasted {
                    generation: request.generation,
                    result: Err(missing_config.clone()),
                },
            );
        }
    }
    if let Some(request) = app.take_key_mutation() {
        if let Some(path) = config_dir.clone() {
            start_key_mutation(service.clone(), path, request, tx.clone());
        } else {
            app::reduce_service(
                app,
                event::ServiceEvent::KeyMutationFinished {
                    generation: request.generation,
                    result: Err(missing_config.clone()),
                },
            );
        }
    }
    if let Some(request) = app.take_alias_mutation() {
        if let Some(path) = config_dir {
            start_alias_mutation(service.clone(), path, request, tx.clone());
        } else {
            app::reduce_service(
                app,
                event::ServiceEvent::AliasMutationFinished {
                    generation: request.generation,
                    result: Err(missing_config),
                },
            );
        }
    }
}

fn start_capability(
    core: crate::core::ConsoleCore,
    request: app::CapabilityRequest,
    tx: tokio::sync::mpsc::Sender<event::ServiceEvent>,
) {
    tokio::spawn(async move {
        let request_id = request.request_id;
        let capability_id = request.capability_id;
        let (values, redaction_values) = request
            .values
            .into_plain_and_redactions(&request.sensitive_field_ids);
        let redactions = event::SensitiveTaskRedactions::from_zeroizing(redaction_values);
        let result = core
            .enqueue_capability(
                crate::core::OperationOrigin::Tui,
                &capability_id,
                values,
                request.confirmed,
            )
            .map_err(|error| redactions.redact(&error.to_string()));
        let task_id = result.as_ref().ok().map(|task| task.id);
        if tx
            .send(event::ServiceEvent::CapabilityTaskStarted {
                request_id,
                capability_id,
                result,
            })
            .await
            .is_err()
        {
            return;
        }
        let Some(task_id) = task_id else {
            return;
        };
        loop {
            let Some(task) = core.capability_task(task_id) else {
                return;
            };
            if task.state != crate::capability::CapabilityTaskState::Running {
                let _ = tx
                    .send(event::ServiceEvent::CapabilityTaskFinished {
                        request_id,
                        completion: event::CapabilityTaskCompletion::new(task, redactions),
                    })
                    .await;
                return;
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
}

fn dispatch_queued_reads(
    app: &mut app::App,
    core: &crate::core::ConsoleCore,
    service: &Arc<dyn ConsoleService>,
    tx: &tokio::sync::mpsc::Sender<event::ServiceEvent>,
) {
    let config_dir = app.active_config().map(|entry| entry.path.clone());
    let missing_config = missing_config_message(app.locale());

    if let Some(request) = app.take_legacy_migration_load() {
        start_legacy_migration_load(core.clone(), request, tx.clone());
    }
    if let Some(request) = app.take_legacy_migration_advance() {
        match request {
            Ok(request) => start_legacy_migration_advance(core.clone(), request, tx.clone()),
            Err(error) => {
                app.legacy_migration.in_flight = false;
                app.legacy_migration.error = Some(error);
            }
        }
    }

    if let Some(request) = app.take_peer_resolve_request() {
        if let Some(path) = config_dir.clone() {
            start_peer_resolve(service.clone(), path, request, tx.clone());
        } else {
            app::reduce_service(
                app,
                event::ServiceEvent::PeerResolveFinished {
                    generation: request.generation,
                    result: Err(missing_config.clone()),
                },
            );
        }
    }
    if let Some(request) = app.take_health_request() {
        match health_config_path(config_dir.clone(), &request.target, app.locale()) {
            Ok(path) => start_health(service.clone(), path, request, tx.clone()),
            Err(error) => {
                app::reduce_service(
                    app,
                    event::ServiceEvent::HealthFinished {
                        generation: request.generation,
                        result: Err(error),
                    },
                );
            }
        }
    }
    if let Some(request) = app.take_runtime_request() {
        start_runtime(service.clone(), request, tx.clone());
    }

    if let Some(request) = app.take_log_request() {
        if let Some(path) = config_dir.clone() {
            start_logs(service.clone(), path, request, tx.clone());
        } else {
            app.log_in_flight = false;
            app.log_error = Some(missing_config.clone());
        }
    }
    if let Some(request) = app.take_log_export_request() {
        if let Some(path) = config_dir.clone() {
            start_log_export_prepare(service.clone(), path, request, tx.clone());
        } else {
            app::reduce_service(
                app,
                event::ServiceEvent::LogExportPrepared {
                    generation: request.generation,
                    result: Err(missing_config.clone()),
                },
            );
        }
    }
    if let Some(request) = app.take_log_export_finish() {
        start_log_export_finish(service.clone(), request, tx.clone());
    }
    if let Some(request) = app.take_key_request() {
        if let Some(path) = config_dir.clone() {
            start_keys(service.clone(), path, request, tx.clone());
        } else {
            app.key_in_flight = false;
            app.key_error = Some(missing_config.clone());
        }
    }
    if let Some(request) = app.take_alias_request() {
        if let Some(path) = config_dir.clone() {
            start_aliases(service.clone(), path, request, tx.clone());
        } else {
            app.alias_in_flight = false;
            app.alias_error = Some(missing_config.clone());
        }
    }
    if let Some(request) = app.take_reward_request() {
        if let Some(path) = config_dir.clone() {
            start_rewards(service.clone(), path, request, tx.clone());
        } else {
            app.reward_in_flight = false;
            app.reward_error = Some(missing_config.clone());
        }
    }
    if let Some(request) = app.take_reward_mint_preflight() {
        if let Some(path) = config_dir.clone() {
            start_reward_mint_preflight(service.clone(), path, request, tx.clone());
        } else {
            app::reduce_service(
                app,
                event::ServiceEvent::RewardMintPreflightFinished {
                    generation: request.generation,
                    result: Err(missing_config.clone()),
                },
            );
        }
    }
    if let Some(request) = app.take_reward_mint_load() {
        if let Some(path) = config_dir.clone() {
            start_reward_mint_load(service.clone(), path, request, tx.clone());
        } else {
            app::reduce_service(
                app,
                event::ServiceEvent::RewardMintLoaded {
                    generation: request.generation,
                    result: Err(missing_config.clone()),
                },
            );
        }
    }
    if let Some(request) = app.take_reward_mint_advance() {
        if let Some(path) = config_dir.clone() {
            start_reward_mint_advance(service.clone(), path, request, tx.clone());
        } else {
            app::reduce_service(
                app,
                event::ServiceEvent::RewardMintAdvanced {
                    generation: request.generation,
                    result: Err(missing_config.clone()),
                },
            );
        }
    }
    if let Some(request) = app.take_legacy_request() {
        if let Some(path) = config_dir.clone() {
            start_legacy_assets(service.clone(), path, request, tx.clone());
        } else {
            app.legacy_in_flight = false;
            app.legacy_error = Some(missing_config.clone());
        }
    }
    if let Some(request) = app.take_confidential_wallet_request() {
        if let Some(path) = config_dir.clone() {
            start_confidential_wallet(service.clone(), path, request, tx.clone());
        } else {
            app::reduce_service(
                app,
                event::ServiceEvent::ConfidentialWalletFinished {
                    generation: request.generation,
                    result: Err(missing_config.clone()),
                },
            );
        }
    }
    if let Some(request) = app.take_pending_escrows_request() {
        if let Some(path) = config_dir.clone() {
            start_pending_escrows(service.clone(), path, request, tx.clone());
        } else {
            app::reduce_service(
                app,
                event::ServiceEvent::PendingEscrowsFinished {
                    generation: request.generation,
                    result: Err(missing_config.clone()),
                },
            );
        }
    }
    if let Some(request) = app.take_hypergraph_request() {
        if let Some(path) = config_dir.clone() {
            start_hypergraph(service.clone(), path, request, tx.clone());
        } else {
            app.hypergraph.in_flight = false;
            app.hypergraph.error = Some(missing_config.clone());
        }
    }
    if let Some(request) = app.take_shard_request() {
        if let Some(path) = config_dir.clone() {
            start_shards(service.clone(), path, request, tx.clone());
        } else {
            app.shards.in_flight = false;
            app.shards.error = Some(missing_config.clone());
        }
    }
    if let Some(request) = app.take_archive_request() {
        if let Some(path) = config_dir {
            start_archive_shards(service.clone(), path, request, tx.clone());
        } else {
            app::reduce_service(
                app,
                event::ServiceEvent::ArchiveShardsFinished {
                    generation: request.generation,
                    result: Err(missing_config),
                },
            );
        }
    }
}

fn health_config_path(
    active_config: Option<PathBuf>,
    target: &domain::HealthTarget,
    locale: i18n::Locale,
) -> Result<PathBuf, String> {
    if let Some(path) = active_config {
        return Ok(path);
    }
    if matches!(
        target,
        domain::HealthTarget::Peer {
            identity_path: Some(_),
            ..
        }
    ) {
        return Ok(PathBuf::new());
    }
    Err(missing_config_message(locale))
}

fn missing_config_message(locale: i18n::Locale) -> String {
    i18n::t(locale, UiText::SelectANodeConfigurationFirst).into()
}

fn start_operation(
    core: crate::core::ConsoleCore,
    request: app::OperationRequest,
    tx: tokio::sync::mpsc::Sender<event::ServiceEvent>,
) {
    tokio::spawn(async move {
        let result = core
            .execute_prover(crate::core::OperationOrigin::Tui, request.action)
            .await
            .map_err(|error| error.to_string());
        let _ = tx
            .send(event::ServiceEvent::OperationFinished {
                generation: request.generation,
                result,
            })
            .await;
    });
}

fn start_prover_operation(
    core: crate::core::ConsoleCore,
    request: app::ProverOperationRequest,
    tx: tokio::sync::mpsc::Sender<event::ServiceEvent>,
) {
    tokio::spawn(async move {
        let result = core
            .execute_prover(crate::core::OperationOrigin::Tui, request.action)
            .await
            .map_err(|error| error.to_string());
        let _ = tx
            .send(event::ServiceEvent::ProverOperationFinished {
                id: request.id,
                result,
            })
            .await;
    });
}

fn start_diagnostics(
    service: Arc<dyn ConsoleService>,
    request: app::DiagnosticsRequest,
    tx: tokio::sync::mpsc::Sender<event::ServiceEvent>,
) {
    tokio::spawn(async move {
        let result = diagnostics_once(service, request.address)
            .await
            .map_err(|error| error.to_string());
        let _ = tx
            .send(event::ServiceEvent::DiagnosticsFinished {
                generation: request.generation,
                result,
            })
            .await;
    });
}

fn start_diagnostics_identity(
    service: Arc<dyn ConsoleService>,
    request: app::DiagnosticsIdentityRequest,
    tx: tokio::sync::mpsc::Sender<event::ServiceEvent>,
) {
    tokio::spawn(async move {
        let path = PathBuf::from(request.path);
        let result = if request.create {
            create_diagnostics_identity_once(service, path).await
        } else {
            inspect_diagnostics_identity_once(service, path).await
        }
        .map_err(|error| error.to_string());
        let _ = tx
            .send(event::ServiceEvent::DiagnosticsIdentityFinished {
                generation: request.generation,
                result,
            })
            .await;
    });
}

fn start_health(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
    request: app::HealthRequest,
    tx: tokio::sync::mpsc::Sender<event::ServiceEvent>,
) {
    tokio::spawn(async move {
        let result = health_once(service, config_dir, request.target)
            .await
            .map_err(|error| error.to_string());
        let _ = tx
            .send(event::ServiceEvent::HealthFinished {
                generation: request.generation,
                result,
            })
            .await;
    });
}

fn start_runtime(
    service: Arc<dyn ConsoleService>,
    request: app::RuntimeRequest,
    tx: tokio::sync::mpsc::Sender<event::ServiceEvent>,
) {
    tokio::spawn(async move {
        let result = runtime_once(service)
            .await
            .map_err(|error| error.to_string());
        let _ = tx
            .send(event::ServiceEvent::RuntimeFinished {
                generation: request.generation,
                result,
            })
            .await;
    });
}

fn start_logs(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
    request: app::LogRequest,
    tx: tokio::sync::mpsc::Sender<event::ServiceEvent>,
) {
    tokio::spawn(async move {
        let result = logs_once(service, config_dir, request.core_id, request.lines)
            .await
            .map_err(|error| error.to_string());
        let _ = tx
            .send(event::ServiceEvent::LogFinished {
                generation: request.generation,
                result,
            })
            .await;
    });
}

fn start_log_export_prepare(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
    request: app::LogExportRequest,
    tx: tokio::sync::mpsc::Sender<event::ServiceEvent>,
) {
    tokio::spawn(async move {
        let result = prepare_log_export_once(service, config_dir, request.options)
            .await
            .map_err(|error| error.to_string());
        let _ = tx
            .send(event::ServiceEvent::LogExportPrepared {
                generation: request.generation,
                result,
            })
            .await;
    });
}

fn start_log_export_finish(
    service: Arc<dyn ConsoleService>,
    request: app::LogExportFinishRequest,
    tx: tokio::sync::mpsc::Sender<event::ServiceEvent>,
) {
    tokio::spawn(async move {
        let result = finish_log_export_once(service, request.id, request.commit)
            .await
            .map_err(|error| error.to_string());
        let _ = tx
            .send(event::ServiceEvent::LogExportFinished {
                generation: request.generation,
                committed: request.commit,
                result,
            })
            .await;
    });
}

fn start_peer_resolve(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
    request: app::PeerResolveRequest,
    tx: tokio::sync::mpsc::Sender<event::ServiceEvent>,
) {
    tokio::spawn(async move {
        let result = peer_resolve_once(service, config_dir, request.peer_id)
            .await
            .map_err(|error| error.to_string());
        let _ = tx
            .send(event::ServiceEvent::PeerResolveFinished {
                generation: request.generation,
                result,
            })
            .await;
    });
}

fn start_confidential_wallet(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
    request: app::ConfidentialWalletRequest,
    tx: tokio::sync::mpsc::Sender<event::ServiceEvent>,
) {
    tokio::spawn(async move {
        let result = confidential_wallet_once(service, config_dir)
            .await
            .map_err(|error| error.to_string());
        let _ = tx
            .send(event::ServiceEvent::ConfidentialWalletFinished {
                generation: request.generation,
                result,
            })
            .await;
    });
}

fn start_wallet_preflight(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
    request: app::WalletPreflightRequest,
    tx: tokio::sync::mpsc::Sender<event::ServiceEvent>,
) {
    tokio::spawn(async move {
        let result = preflight_wallet_action_once(service, config_dir, request.request)
            .await
            .map_err(|error| error.to_string());
        let _ = tx
            .send(event::ServiceEvent::WalletPreflightFinished {
                generation: request.generation,
                result,
            })
            .await;
    });
}

fn start_wallet_execution(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
    request: app::WalletExecuteRequest,
    tx: tokio::sync::mpsc::Sender<event::ServiceEvent>,
) {
    tokio::spawn(async move {
        let result =
            execute_wallet_action_once(service, config_dir, request.request, request.approved)
                .await
                .map_err(|error| error.to_string());
        let _ = tx
            .send(event::ServiceEvent::WalletExecutionFinished {
                generation: request.generation,
                result,
            })
            .await;
    });
}

fn start_pending_escrows(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
    request: app::PendingEscrowsRequest,
    tx: tokio::sync::mpsc::Sender<event::ServiceEvent>,
) {
    tokio::spawn(async move {
        let result = pending_escrows_once(service, config_dir)
            .await
            .map_err(|error| error.to_string());
        let _ = tx
            .send(event::ServiceEvent::PendingEscrowsFinished {
                generation: request.generation,
                result,
            })
            .await;
    });
}

fn start_keys(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
    request: app::KeyRequest,
    tx: tokio::sync::mpsc::Sender<event::ServiceEvent>,
) {
    tokio::spawn(async move {
        let result = keys_once(service, config_dir)
            .await
            .map_err(|error| error.to_string());
        let _ = tx
            .send(event::ServiceEvent::KeysFinished {
                generation: request.generation,
                result,
            })
            .await;
    });
}

fn start_key_mutation(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
    request: app::KeyMutationRequest,
    tx: tokio::sync::mpsc::Sender<event::ServiceEvent>,
) {
    tokio::spawn(async move {
        let result = mutate_key_once(service, config_dir, request.mutation)
            .await
            .map_err(|error| error.to_string());
        let _ = tx
            .send(event::ServiceEvent::KeyMutationFinished {
                generation: request.generation,
                result,
            })
            .await;
    });
}

fn start_aliases(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
    request: app::AliasRequest,
    tx: tokio::sync::mpsc::Sender<event::ServiceEvent>,
) {
    tokio::spawn(async move {
        let result = aliases_once(service, config_dir)
            .await
            .map_err(|error| error.to_string());
        let _ = tx
            .send(event::ServiceEvent::AliasesFinished {
                generation: request.generation,
                result,
            })
            .await;
    });
}

fn start_alias_mutation(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
    request: app::AliasMutationRequest,
    tx: tokio::sync::mpsc::Sender<event::ServiceEvent>,
) {
    tokio::spawn(async move {
        let result = mutate_alias_once(service, config_dir, request.mutation)
            .await
            .map_err(|error| error.to_string());
        let _ = tx
            .send(event::ServiceEvent::AliasMutationFinished {
                generation: request.generation,
                result,
            })
            .await;
    });
}

fn start_rewards(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
    request: app::RewardRequest,
    tx: tokio::sync::mpsc::Sender<event::ServiceEvent>,
) {
    tokio::spawn(async move {
        let result = rewards_once(service, config_dir)
            .await
            .map_err(|error| error.to_string());
        let _ = tx
            .send(event::ServiceEvent::RewardsFinished {
                generation: request.generation,
                result,
            })
            .await;
    });
}

fn start_reward_mint_preflight(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
    request: app::RewardMintReadRequest,
    tx: tokio::sync::mpsc::Sender<event::ServiceEvent>,
) {
    tokio::spawn(async move {
        let result = reward_mint_preflight_once(service, config_dir)
            .await
            .map_err(|error| error.to_string());
        let _ = tx
            .send(event::ServiceEvent::RewardMintPreflightFinished {
                generation: request.generation,
                result,
            })
            .await;
    });
}

fn start_reward_mint(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
    request: app::RewardMintStartRequest,
    tx: tokio::sync::mpsc::Sender<event::ServiceEvent>,
) {
    tokio::spawn(async move {
        let result = start_reward_mint_once(service, config_dir, request.approved)
            .await
            .map_err(|error| error.to_string());
        let _ = tx
            .send(event::ServiceEvent::RewardMintStarted {
                generation: request.generation,
                result,
            })
            .await;
    });
}

fn start_reward_mint_load(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
    request: app::RewardMintReadRequest,
    tx: tokio::sync::mpsc::Sender<event::ServiceEvent>,
) {
    tokio::spawn(async move {
        let result = load_reward_mint_once(service, config_dir)
            .await
            .map_err(|error| error.to_string());
        let _ = tx
            .send(event::ServiceEvent::RewardMintLoaded {
                generation: request.generation,
                result,
            })
            .await;
    });
}

fn start_reward_mint_advance(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
    request: app::RewardMintReadRequest,
    tx: tokio::sync::mpsc::Sender<event::ServiceEvent>,
) {
    tokio::spawn(async move {
        let result = advance_reward_mint_once(service, config_dir)
            .await
            .map_err(|error| error.to_string());
        let _ = tx
            .send(event::ServiceEvent::RewardMintAdvanced {
                generation: request.generation,
                result,
            })
            .await;
    });
}

fn start_reward_mint_rebroadcast(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
    request: app::RewardMintReadRequest,
    tx: tokio::sync::mpsc::Sender<event::ServiceEvent>,
) {
    tokio::spawn(async move {
        let result = rebroadcast_reward_mint_once(service, config_dir)
            .await
            .map_err(|error| error.to_string());
        let _ = tx
            .send(event::ServiceEvent::RewardMintRebroadcasted {
                generation: request.generation,
                result,
            })
            .await;
    });
}

fn start_legacy_assets(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
    request: app::LegacyAssetsRequest,
    tx: tokio::sync::mpsc::Sender<event::ServiceEvent>,
) {
    tokio::spawn(async move {
        let result = legacy_assets_once(service, config_dir)
            .await
            .map_err(|error| error.to_string());
        let _ = tx
            .send(event::ServiceEvent::LegacyAssetsFinished {
                generation: request.generation,
                result,
            })
            .await;
    });
}

fn start_legacy_migration_preflight(
    core: crate::core::ConsoleCore,
    request: app::LegacyMigrationPreflightWork,
    tx: tokio::sync::mpsc::Sender<event::ServiceEvent>,
) {
    tokio::spawn(async move {
        let result = core
            .preflight_legacy_migration(request.request)
            .await
            .map_err(|error| error.to_string());
        let _ = tx
            .send(event::ServiceEvent::LegacyMigrationPreflightFinished {
                generation: request.generation,
                result,
            })
            .await;
    });
}

fn start_legacy_migration(
    core: crate::core::ConsoleCore,
    request: app::LegacyMigrationStartWork,
    tx: tokio::sync::mpsc::Sender<event::ServiceEvent>,
) {
    tokio::spawn(async move {
        let result = core
            .start_legacy_migration(crate::core::OperationOrigin::Tui, request.approved)
            .await
            .map_err(|error| error.to_string());
        let _ = tx
            .send(event::ServiceEvent::LegacyMigrationStarted {
                generation: request.generation,
                result,
            })
            .await;
    });
}

fn start_legacy_migration_load(
    core: crate::core::ConsoleCore,
    request: app::LegacyMigrationReadWork,
    tx: tokio::sync::mpsc::Sender<event::ServiceEvent>,
) {
    tokio::spawn(async move {
        let result = core
            .load_legacy_migration(&request.fingerprint)
            .await
            .map_err(|error| error.to_string());
        let _ = tx
            .send(event::ServiceEvent::LegacyMigrationLoaded {
                generation: request.generation,
                result,
            })
            .await;
    });
}

fn start_legacy_migration_advance(
    core: crate::core::ConsoleCore,
    request: app::LegacyMigrationAdvanceWork,
    tx: tokio::sync::mpsc::Sender<event::ServiceEvent>,
) {
    tokio::spawn(async move {
        let result = core
            .advance_legacy_migration(
                crate::core::OperationOrigin::Tui,
                &request.fingerprint,
                request.source,
            )
            .await
            .map_err(|error| error.to_string());
        let _ = tx
            .send(event::ServiceEvent::LegacyMigrationAdvanced {
                generation: request.generation,
                result,
            })
            .await;
    });
}

fn start_legacy_migration_rebroadcast(
    core: crate::core::ConsoleCore,
    request: app::LegacyMigrationReadWork,
    tx: tokio::sync::mpsc::Sender<event::ServiceEvent>,
) {
    tokio::spawn(async move {
        let result = core
            .rebroadcast_legacy_migration(
                crate::core::OperationOrigin::Tui,
                &request.fingerprint,
                true,
            )
            .await
            .map_err(|error| error.to_string());
        let _ = tx
            .send(event::ServiceEvent::LegacyMigrationRebroadcasted {
                generation: request.generation,
                result,
            })
            .await;
    });
}

fn start_legacy_migration_file(
    request: app::LegacyMigrationFileWork,
    tx: tokio::sync::mpsc::Sender<event::ServiceEvent>,
) {
    tokio::spawn(async move {
        let kind = request.kind;
        let result = tokio::task::spawn_blocking(move || {
            let directory = std::fs::canonicalize(&request.recipient_config_dir)
                .map_err(|error| format!("Cannot resolve migration directory: {error}"))?;
            let name = match kind {
                app::LegacyMigrationFileKind::Config => "config.yml",
                app::LegacyMigrationFileKind::Keys => "keys.yml",
            };
            let path = directory.join(name);
            let metadata = std::fs::symlink_metadata(&path)
                .map_err(|error| format!("Cannot inspect {name}: {error}"))?;
            if !metadata.file_type().is_file() {
                return Err(format!("{name} is not a regular file"));
            }
            let parent = std::fs::canonicalize(path.parent().unwrap_or(&directory))
                .map_err(|error| format!("Cannot resolve {name}: {error}"))?;
            if parent != directory {
                return Err(format!("{name} is outside the migration directory"));
            }
            std::fs::read_to_string(path)
                .map(event::SensitiveFilePayload::new)
                .map_err(|error| format!("Cannot read {name}: {error}"))
        })
        .await
        .unwrap_or_else(|error| Err(format!("File viewer task failed: {error}")));
        let _ = tx
            .send(event::ServiceEvent::LegacyMigrationFileLoaded {
                generation: request.generation,
                kind,
                result,
            })
            .await;
    });
}

fn start_hypergraph(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
    request: app::HypergraphRequest,
    tx: tokio::sync::mpsc::Sender<event::ServiceEvent>,
) {
    tokio::spawn(async move {
        let result = hypergraph_once(service, config_dir, request.kind, request.address)
            .await
            .map_err(|error| error.to_string());
        let _ = tx
            .send(event::ServiceEvent::HypergraphFinished {
                generation: request.generation,
                result,
            })
            .await;
    });
}

fn start_shards(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
    request: app::ShardRequest,
    tx: tokio::sync::mpsc::Sender<event::ServiceEvent>,
) {
    tokio::spawn(async move {
        let result = shards_once(service, config_dir, request.include_all)
            .await
            .map_err(|error| error.to_string());
        let _ = tx
            .send(event::ServiceEvent::ShardsFinished {
                generation: request.generation,
                result,
            })
            .await;
    });
}

fn start_archive_shards(
    service: Arc<dyn ConsoleService>,
    config_dir: PathBuf,
    request: app::ArchiveShardRequest,
    tx: tokio::sync::mpsc::Sender<event::ServiceEvent>,
) {
    tokio::spawn(async move {
        let result = archive_shards_once(service, config_dir)
            .await
            .map_err(|error| error.to_string());
        let _ = tx
            .send(event::ServiceEvent::ArchiveShardsFinished {
                generation: request.generation,
                result,
            })
            .await;
    });
}

fn start_refresh(
    app: &mut app::App,
    in_flight: &mut bool,
    core: crate::core::ConsoleCore,
    tx: tokio::sync::mpsc::Sender<event::ServiceEvent>,
) {
    let Some(generation) = request_refresh(app, in_flight) else {
        return;
    };
    spawn_core_refresh(core, generation, tx);
}

fn start_config_refresh(
    app: &mut app::App,
    in_flight: &mut bool,
    current_config_activation: &mut Option<u64>,
    home: PathBuf,
    core: crate::core::ConsoleCore,
    entry: config::ConfigEntry,
    tx: tokio::sync::mpsc::Sender<event::ServiceEvent>,
) -> bool {
    let Some(generation) = request_config_refresh(app, in_flight) else {
        return false;
    };
    *current_config_activation = Some(generation);
    tokio::spawn(async move {
        let path = entry.path.clone();
        let (result, persistence_error) = match core
            .prepare_config_switch(crate::core::OperationOrigin::Tui, entry)
            .await
        {
            Ok(prepared) => {
                let persistence_error = match tokio::task::spawn_blocking(move || {
                    config::remember_config_dir(&home, &path)
                })
                .await
                {
                    Ok(Ok(())) => None,
                    Ok(Err(error)) => Some(error),
                    Err(error) => Some(format!("Configuration persistence task failed: {error}")),
                };
                prepared.commit();
                (Ok(()), persistence_error)
            }
            Err(error) => (Err(error.to_string()), None),
        };
        let _ = tx
            .send(event::ServiceEvent::ConfigActivationFinished {
                generation,
                result,
                persistence_error,
            })
            .await;
    });
    true
}

fn spawn_core_refresh(
    core: crate::core::ConsoleCore,
    generation: u64,
    tx: tokio::sync::mpsc::Sender<event::ServiceEvent>,
) {
    tokio::spawn(async move {
        let result = refresh_core_once(core)
            .await
            .map(|_| ())
            .map_err(|error| error.to_string());
        let _ = tx
            .send(event::ServiceEvent::RefreshFinished { generation, result })
            .await;
    });
}

#[cfg(test)]
mod capability_controller_tests {
    use std::{path::PathBuf, sync::Arc, time::Duration};

    use async_trait::async_trait;

    use super::*;
    use crate::{
        capability::{CapabilityInvocation, CapabilityRunResult, CapabilityRunner},
        config::ConfigEntry,
    };

    struct InstantRunner;

    #[async_trait]
    impl CapabilityRunner for InstantRunner {
        async fn run(
            &self,
            _invocation: CapabilityInvocation,
        ) -> Result<CapabilityRunResult, crate::service::ServiceError> {
            Ok(CapabilityRunResult {
                exit_code: 0,
                stdout: "ready".into(),
                stderr: String::new(),
            })
        }
    }

    #[tokio::test]
    async fn tui_capability_dispatch_reports_running_then_completion() {
        let core = crate::core::ConsoleCore::new_with_capability_runner(
            Arc::new(crate::service::NodeConsoleService::default()),
            Some(ConfigEntry {
                name: "test".into(),
                path: PathBuf::from("/tmp/qclient-console-test-config"),
                is_default: true,
            }),
            Arc::new(InstantRunner),
        );
        let (tx, mut rx) = tokio::sync::mpsc::channel(2);
        start_capability(
            core,
            app::CapabilityRequest {
                request_id: 1,
                capability_id: "config.print".into(),
                values: app::CapabilityUiValues::default(),
                sensitive_field_ids: Vec::new(),
                confirmed: false,
            },
            tx,
        );

        let started = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap();
        let task_id = match started {
            event::ServiceEvent::CapabilityTaskStarted {
                request_id: 1,
                result: Ok(task),
                ..
            } => {
                assert_eq!(task.state, crate::capability::CapabilityTaskState::Running);
                task.id
            }
            other => panic!("unexpected event: {other:?}"),
        };
        let finished = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap();
        match finished {
            event::ServiceEvent::CapabilityTaskFinished {
                request_id: 1,
                completion,
            } => {
                let task = completion.into_display_task();
                assert_eq!(task.id, task_id);
                assert_eq!(
                    task.state,
                    crate::capability::CapabilityTaskState::Succeeded
                );
                assert_eq!(task.stdout, "ready");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }
}

#[cfg(test)]
mod web_settings_controller_tests {
    use super::*;
    use crate::settings;
    use crate::settings::{ConsoleSettings, WebSettings};
    use i18n::Locale;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    fn initialize(home: &Path) -> app::App {
        initialize_app(
            config::RunOptions::Run {
                explicit_config: None,
            },
            home,
            home,
        )
        .0
    }

    fn test_core() -> crate::core::ConsoleCore {
        crate::core::ConsoleCore::new(Arc::new(service::NodeConsoleService::default()), None)
    }

    fn loopback(port: u16) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
    }

    #[tokio::test]
    async fn web_settings_reconcile_starts_and_stops_listener() {
        let home = tempfile::tempdir().unwrap();
        let mut server = None;
        let disabled = WebSettings {
            enabled: false,
            port: 8888,
        };
        let enabled = WebSettings {
            enabled: true,
            port: 8888,
        };

        apply_web_settings(
            &mut server,
            disabled,
            enabled,
            test_core(),
            home.path().to_path_buf(),
            loopback(0),
        )
        .await
        .unwrap();
        assert!(server.is_some());

        apply_web_settings(
            &mut server,
            enabled,
            disabled,
            test_core(),
            home.path().to_path_buf(),
            loopback(0),
        )
        .await
        .unwrap();
        assert!(server.is_none());
    }

    #[tokio::test]
    async fn web_settings_reconcile_moves_ports_without_dropping_first() {
        let home = tempfile::tempdir().unwrap();
        let core = test_core();
        let mut server = None;
        let first = WebSettings {
            enabled: true,
            port: 8001,
        };
        let second = WebSettings {
            enabled: true,
            port: 8002,
        };
        apply_web_settings(
            &mut server,
            WebSettings {
                enabled: false,
                port: 8888,
            },
            first,
            core.clone(),
            home.path().to_path_buf(),
            loopback(0),
        )
        .await
        .unwrap();
        let old_address = server.as_ref().unwrap().local_addr();

        apply_web_settings(
            &mut server,
            first,
            second,
            core,
            home.path().to_path_buf(),
            loopback(0),
        )
        .await
        .unwrap();

        let new_address = server.as_ref().unwrap().local_addr();
        assert_ne!(new_address, old_address);
        assert!(tokio::net::TcpStream::connect(old_address).await.is_err());
        assert!(tokio::net::TcpStream::connect(new_address).await.is_ok());
    }

    #[tokio::test]
    async fn web_settings_reconcile_keeps_old_listener_when_new_bind_fails() {
        let home = tempfile::tempdir().unwrap();
        let core = test_core();
        let mut server = None;
        let first = WebSettings {
            enabled: true,
            port: 8001,
        };
        let second = WebSettings {
            enabled: true,
            port: 8002,
        };
        apply_web_settings(
            &mut server,
            WebSettings {
                enabled: false,
                port: 8888,
            },
            first,
            core.clone(),
            home.path().to_path_buf(),
            loopback(0),
        )
        .await
        .unwrap();
        let old_address = server.as_ref().unwrap().local_addr();
        let occupied = tokio::net::TcpListener::bind(loopback(0)).await.unwrap();
        let occupied_address = occupied.local_addr().unwrap();

        let error = apply_web_settings(
            &mut server,
            first,
            second,
            core,
            home.path().to_path_buf(),
            occupied_address,
        )
        .await
        .unwrap_err();

        assert!(error.contains("Cannot start Web server"), "{error}");
        assert_eq!(server.as_ref().unwrap().local_addr(), old_address);
        assert!(tokio::net::TcpStream::connect(old_address).await.is_ok());
    }

    #[tokio::test]
    async fn failed_saved_candidate_restores_app_and_settings_file() {
        let home = tempfile::tempdir().unwrap();
        let occupied = tokio::net::TcpListener::bind("0.0.0.0:0").await.unwrap();
        let port = occupied.local_addr().unwrap().port();
        let previous = ConsoleSettings {
            web: WebSettings {
                enabled: false,
                port,
            },
            language: Locale::English,
        };
        settings::save(home.path(), previous).unwrap();
        let mut app = app::App::default();
        app.set_console_settings(previous);
        app.open(app::Page::Other);
        app::reduce_input(&mut app, event::InputEvent::Enter);
        let request = app.take_console_settings_request().unwrap();
        let candidate = ConsoleSettings {
            web: WebSettings {
                enabled: true,
                port,
            },
            language: request.settings.language,
        };
        settings::save(home.path(), candidate).unwrap();
        let mut server = None;

        reconcile_settings_worker_result(
            &mut app,
            &mut server,
            SettingsWorkerResult {
                generation: request.generation,
                candidate,
                result: Ok(()),
            },
            test_core(),
            home.path().to_path_buf(),
        )
        .await;

        assert!(server.is_none());
        assert_eq!(app.other.settings, previous.web);
        assert_eq!(settings::load(home.path()), Ok(previous));
        assert!(app
            .other
            .error
            .as_deref()
            .unwrap()
            .contains("previous console settings were restored"));
    }

    #[test]
    fn tui_quit_waits_for_all_in_flight_settings() {
        let mut app = app::App {
            should_quit: true,
            ..Default::default()
        };
        app.other.in_flight = true;

        assert!(!tui_exit_ready(&app));

        app.other.in_flight = false;
        app.other.read_rpc_in_flight = true;
        assert!(!tui_exit_ready(&app));

        app.other.read_rpc_in_flight = false;
        assert!(tui_exit_ready(&app));
    }

    #[test]
    fn startup_loads_saved_enabled_console_settings() {
        let home = tempfile::tempdir().unwrap();
        let saved = ConsoleSettings {
            web: WebSettings {
                enabled: true,
                port: 9000,
            },
            language: Locale::TraditionalChinese,
        };
        settings::save(home.path(), saved).unwrap();

        let app = initialize(home.path());

        assert_eq!(app.other.settings.port, 9000);
        assert!(app.other.settings.enabled);
        assert_eq!(app.locale(), Locale::TraditionalChinese);
        assert_eq!(app.other.port_input, "9000");
        assert!(app.other.error.is_none());
    }

    #[test]
    fn startup_reports_invalid_web_settings_without_replacing_the_file() {
        let home = tempfile::tempdir().unwrap();
        let path = settings::settings_file(home.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let invalid = "enabled: [broken";
        std::fs::write(&path, invalid).unwrap();

        let mut app = initialize(home.path());

        assert_eq!(app.other.settings, WebSettings::default());
        assert_eq!(app.other.port_input, "8888");
        assert!(app
            .other
            .error
            .as_ref()
            .unwrap()
            .contains("Cannot parse console settings"));
        assert!(app.take_console_settings_request().is_none());
        assert_eq!(std::fs::read_to_string(path).unwrap(), invalid);
    }

    #[test]
    fn startup_without_web_settings_uses_defaults_without_saving() {
        let home = tempfile::tempdir().unwrap();
        let mut app = initialize(home.path());

        assert_eq!(app.other.settings, WebSettings::default());
        assert!(app.other.error.is_none());
        assert!(app.take_console_settings_request().is_none());
        assert!(!settings::settings_file(home.path()).exists());
    }

    fn queued_language_change(settings: WebSettings) -> app::App {
        let mut app = app::App::default();
        app.set_console_settings(ConsoleSettings {
            web: settings,
            language: Locale::English,
        });
        app.open(app::Page::Other);
        app::reduce_input(&mut app, event::InputEvent::Down);
        app::reduce_input(&mut app, event::InputEvent::Down);
        app::reduce_input(&mut app, event::InputEvent::Down);
        app::reduce_input(&mut app, event::InputEvent::Enter);
        app
    }

    async fn receive(
        rx: &mut tokio::sync::mpsc::Receiver<SettingsWorkerResult>,
    ) -> SettingsWorkerResult {
        tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("settings worker must finish")
            .expect("settings worker must return a result")
    }

    #[tokio::test]
    async fn settings_worker_takes_request_once_and_commits_only_when_consumed() {
        let home = tempfile::tempdir().unwrap();
        let mut app = queued_language_change(WebSettings::default());
        let generation = app.other.generation;
        let candidate = ConsoleSettings {
            web: WebSettings::default(),
            language: Locale::TraditionalChinese,
        };
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);

        dispatch_console_settings(&mut app, home.path(), &tx);
        assert!(app.take_console_settings_request().is_none());
        app::reduce_input(&mut app, event::InputEvent::Enter);
        dispatch_console_settings(&mut app, home.path(), &tx);
        assert!(app.other.in_flight);
        assert_eq!(app.other.settings, WebSettings::default());

        let result = receive(&mut rx).await;
        assert_eq!(result.generation, generation);
        assert_eq!(result.candidate, candidate);
        assert_eq!(result.result, Ok(()));
        assert_eq!(settings::load(home.path()), Ok(candidate));
        assert_eq!(app.other.settings, WebSettings::default());
        assert!(rx.try_recv().is_err());

        consume_settings_worker_result(&mut app, result);
        assert_eq!(app.other.settings, candidate.web);
        assert_eq!(app.locale(), candidate.language);
        assert!(!app.other.in_flight);
        assert!(app.other.error.is_none());
    }

    #[tokio::test]
    async fn settings_worker_preserves_committed_values_on_save_failure() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home-is-a-file");
        std::fs::write(&home, "keep this file").unwrap();
        let committed = ConsoleSettings {
            web: WebSettings {
                enabled: false,
                port: 9000,
            },
            language: Locale::English,
        };
        let mut app = queued_language_change(committed.web);
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);

        dispatch_console_settings(&mut app, &home, &tx);
        let result = receive(&mut rx).await;
        assert!(result.result.is_err());
        assert_eq!(app.other.settings, committed.web);
        assert!(app.other.in_flight);

        consume_settings_worker_result(&mut app, result);
        assert_eq!(app.other.settings, committed.web);
        assert_eq!(app.other.port_input, "9000");
        assert_eq!(app.locale(), committed.language);
        assert!(!app.other.in_flight);
        assert!(app.other.error.is_some());
        assert_eq!(std::fs::read_to_string(home).unwrap(), "keep this file");
    }

    #[tokio::test]
    async fn settings_worker_ignores_old_success_and_failure_while_new_request_is_pending() {
        let home = tempfile::tempdir().unwrap();
        let mut app = queued_language_change(WebSettings::default());
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        dispatch_console_settings(&mut app, home.path(), &tx);
        let old_result = receive(&mut rx).await;
        let old_generation = old_result.generation;
        let old_candidate = old_result.candidate;
        let committed = ConsoleSettings {
            web: WebSettings {
                enabled: false,
                port: 9000,
            },
            language: Locale::English,
        };
        app.set_console_settings(committed);
        app::reduce_input(&mut app, event::InputEvent::Enter);
        dispatch_console_settings(&mut app, home.path(), &tx);

        consume_settings_worker_result(&mut app, old_result);
        consume_settings_worker_result(
            &mut app,
            SettingsWorkerResult {
                generation: old_generation,
                candidate: old_candidate,
                result: Err("stale failure".into()),
            },
        );
        assert_eq!(app.other.settings, committed.web);
        assert!(app.other.in_flight);
        assert!(app.other.error.is_none());

        let result = receive(&mut rx).await;
        let candidate = result.candidate;
        consume_settings_worker_result(&mut app, result);
        assert_eq!(app.other.settings, candidate.web);
        assert_eq!(app.other.settings.port, 9000);
        assert!(!app.other.settings.enabled);
        assert!(!app.other.in_flight);
    }
}

#[cfg(test)]
mod dependency_tests {
    use std::path::{Path, PathBuf};

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use crate::{
        app::{App, Page},
        config::{self, ConfigEntry, RunOptions},
        core::CoreStateSnapshot,
        domain::NodeSnapshot,
        event::{InputEvent, ServiceEvent},
    };

    use super::{ConfigPathWorkerResult, PendingConfigWork};

    #[test]
    fn page_keys_map_to_navigation_events_and_are_ignored_by_forms() {
        for (code, name) in [(KeyCode::PageUp, "PageUp"), (KeyCode::PageDown, "PageDown")] {
            let key = KeyEvent::new(code, KeyModifiers::NONE);
            let event = super::map_key(key).expect("page key must map to navigation");
            assert_eq!(format!("{event:?}"), name);
            assert_eq!(super::map_form_key(key), None);
        }
    }

    fn create_config_dir(parent: &Path, name: &str) -> PathBuf {
        let path = parent.join(name);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("config.yml"), "listenGrpcMultiaddr: test\n").unwrap();
        std::fs::write(path.join("keys.yml"), "null:\n").unwrap();
        path
    }

    #[test]
    fn manifest_embeds_the_shared_quil_client_crate() {
        let manifest = include_str!("../../Cargo.toml");
        assert!(manifest.lines().any(|line| {
            let dependency = line.trim().split_once('=').map(|(name, _)| name.trim());
            dependency == Some("quil-client")
        }));
    }

    #[test]
    fn control_c_requests_a_clean_exit_in_raw_mode() {
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);

        assert_eq!(super::map_key(key), Some(InputEvent::Quit));
    }

    #[test]
    fn legacy_file_viewer_keys_reach_navigation_pages() {
        for character in ['c', 'C', 'k', 'K'] {
            let key = KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE);

            assert_eq!(super::map_key(key), Some(InputEvent::Character(character)));
        }
    }

    #[test]
    fn direction_keys_remain_available_while_editing_a_form() {
        for (code, expected) in [
            (KeyCode::Up, InputEvent::Up),
            (KeyCode::Down, InputEvent::Down),
            (KeyCode::Left, InputEvent::Left),
            (KeyCode::Right, InputEvent::Right),
        ] {
            let key = KeyEvent::new(code, KeyModifiers::NONE);

            assert_eq!(super::map_form_key(key), Some(expected));
        }
    }

    #[test]
    fn operation_refresh_waits_for_the_current_refresh() {
        let mut pending = true;

        assert!(!super::should_start_follow_up_refresh(true, &mut pending));
        assert!(pending);
        assert!(super::should_start_follow_up_refresh(false, &mut pending));
        assert!(!pending);
    }

    #[test]
    fn core_projection_switches_app_without_activation_event() {
        let old = ConfigEntry {
            name: "old".into(),
            path: "/tmp/old".into(),
            is_default: true,
        };
        let new = ConfigEntry {
            name: "new".into(),
            path: "/tmp/new".into(),
            is_default: false,
        };
        let mut app = App::default();
        app.set_configs(vec![old]);
        let state = CoreStateSnapshot {
            active_config: Some(new),
            node_snapshot: Some(NodeSnapshot {
                config_name: "new".into(),
                peer_id: "new-peer".into(),
                connected: true,
                ..Default::default()
            }),
            ..Default::default()
        };

        app.apply_core_state_projection(&state);

        assert_eq!(app.active_config().unwrap().name, "new");
        assert_eq!(app.snapshot.as_ref().unwrap().peer_id, "new-peer");
        assert!(app.take_reward_mint_load().is_some());
        let generation = app.refresh_generation;

        app.apply_core_state_projection(&state);

        assert_eq!(app.refresh_generation, generation);
        assert!(app.take_reward_mint_load().is_none());
    }

    #[test]
    fn refresh_completion_cannot_replay_noncanonical_snapshot() {
        let config = ConfigEntry {
            name: "mainnet".into(),
            path: "/tmp/mainnet".into(),
            is_default: true,
        };
        let filter = vec![0xaa; 32];
        let mut app = App {
            refresh_generation: 7,
            snapshot: Some(NodeSnapshot {
                peer_id: "noncanonical".into(),
                allocations: vec![crate::domain::AllocationSnapshot {
                    filter_hex: hex::encode(&filter),
                    status: crate::domain::AllocationStatus::Active,
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        app.set_configs(vec![config.clone()]);
        app.prover_operations
            .enqueue(
                crate::domain::ProverAction::Join {
                    filters: vec![filter.clone()],
                    delegate: Vec::new(),
                    worker_ids: vec![7],
                },
                Some(7),
                1,
            )
            .unwrap();

        let store = crate::core::CoreStateStore::new(Some(config));
        let mut changes = store.subscribe();
        assert!(store.replace_node_snapshot(
            0,
            NodeSnapshot {
                peer_id: "canonical".into(),
                allocations: vec![crate::domain::AllocationSnapshot {
                    filter_hex: hex::encode(&filter),
                    status: crate::domain::AllocationStatus::Joining,
                    ..Default::default()
                }],
                ..Default::default()
            }
        ));

        super::reduce_service_with_core_projection(
            &mut app,
            &mut changes,
            ServiceEvent::RefreshFinished {
                generation: 7,
                result: Ok(()),
            },
            false,
        );

        assert_eq!(
            app.prover_operations.items[0].phase,
            crate::app::ProverOperationPhase::Submitting
        );
        assert!(app.prover_operations.conflicts_filter(&filter));
        assert!(app.prover_operations.conflicts_core(7));
        assert_eq!(app.snapshot.as_ref().unwrap().peer_id, "canonical");
        assert_eq!(
            app.snapshot.as_ref().unwrap().allocations[0].status,
            crate::domain::AllocationStatus::Joining
        );
    }

    #[test]
    fn same_snapshot_value_with_new_revision_reconciles_updated_prover_phase() {
        let config = ConfigEntry {
            name: "mainnet".into(),
            path: "/tmp/mainnet".into(),
            is_default: true,
        };
        let filter = vec![0xbb; 32];
        let mut app = App::default();
        app.set_configs(vec![config.clone()]);
        let id = app
            .prover_operations
            .enqueue(
                crate::domain::ProverAction::Leave {
                    filters: vec![filter.clone()],
                },
                Some(7),
                1,
            )
            .unwrap();
        let mut state = CoreStateSnapshot {
            snapshot_revision: 1,
            active_config: Some(config),
            node_snapshot: Some(NodeSnapshot::default()),
            ..Default::default()
        };

        app.apply_core_state_projection(&state);
        assert_eq!(
            app.prover_operations.items[0].phase,
            crate::app::ProverOperationPhase::Submitting
        );

        app.prover_operations.finish_submission(
            id,
            Ok(crate::domain::OperationResult {
                message: "accepted without a definitive response".into(),
                outcome: crate::domain::OperationOutcome::Unknown,
            }),
        );
        assert_eq!(
            app.prover_operations.items[0].phase,
            crate::app::ProverOperationPhase::Unknown
        );

        state.snapshot_revision = 2;
        app.apply_core_state_projection(&state);

        assert_eq!(
            app.prover_operations.items[0].phase,
            crate::app::ProverOperationPhase::Complete
        );
        assert!(!app.prover_operations.conflicts_filter(&filter));
        assert!(!app.prover_operations.conflicts_core(7));
    }

    #[test]
    fn same_path_metadata_is_projected_without_reset() {
        let path = PathBuf::from("/tmp/shared");
        let old = ConfigEntry {
            name: "old-name".into(),
            path: path.clone(),
            is_default: false,
        };
        let new = ConfigEntry {
            name: "new-name".into(),
            path,
            is_default: true,
        };
        let snapshot = NodeSnapshot {
            config_name: "shared".into(),
            peer_id: "same-peer".into(),
            ..Default::default()
        };
        let mut app = App {
            snapshot: Some(snapshot.clone()),
            refresh_generation: 7,
            ..Default::default()
        };
        app.set_configs(vec![old]);
        let state = CoreStateSnapshot {
            active_config: Some(new.clone()),
            node_snapshot: Some(snapshot.clone()),
            ..Default::default()
        };

        app.apply_core_state_projection(&state);

        assert_eq!(app.active_config(), Some(&new));
        assert_eq!(app.refresh_generation, 7);
        assert_eq!(app.snapshot.as_ref(), Some(&snapshot));
        assert!(app.take_reward_mint_load().is_none());

        app.apply_core_state_projection(&state);

        assert_eq!(app.active_config(), Some(&new));
        assert_eq!(app.refresh_generation, 7);
        assert_eq!(app.snapshot.as_ref(), Some(&snapshot));
        assert!(app.take_reward_mint_load().is_none());
    }

    #[test]
    fn config_activation_event_cannot_override_newer_core_projection() {
        let old = ConfigEntry {
            name: "old".into(),
            path: "/tmp/old".into(),
            is_default: true,
        };
        let new = ConfigEntry {
            name: "new".into(),
            path: "/tmp/new".into(),
            is_default: false,
        };
        let snapshot = NodeSnapshot {
            config_name: "new".into(),
            peer_id: "new-peer".into(),
            ..Default::default()
        };
        let mut app = App::default();
        app.set_configs(vec![old]);

        app.apply_core_state_projection(&CoreStateSnapshot {
            active_config: Some(new),
            node_snapshot: Some(snapshot),
            ..Default::default()
        });
        let generation = app.refresh_generation;

        crate::app::reduce_service(
            &mut app,
            ServiceEvent::ConfigActivationFinished {
                generation,
                result: Ok(()),
                persistence_error: None,
            },
        );

        assert_eq!(app.active_config().unwrap().name, "new");
        assert_eq!(app.snapshot.as_ref().unwrap().peer_id, "new-peer");
    }

    #[test]
    fn startup_config_activation_queues_reward_load_after_core_projection() {
        let config = ConfigEntry {
            name: "startup".into(),
            path: "/tmp/startup".into(),
            is_default: true,
        };
        let mut app = App::default();
        app.set_configs(vec![config.clone()]);
        app.begin_node_refresh();
        let generation = app.refresh_generation;
        let store = crate::core::CoreStateStore::new(Some(config));
        let mut changes = store.subscribe();
        assert!(store.replace_node_snapshot(
            0,
            NodeSnapshot {
                config_name: "startup".into(),
                connected: true,
                ..Default::default()
            }
        ));

        super::reduce_service_with_core_projection(
            &mut app,
            &mut changes,
            ServiceEvent::ConfigActivationFinished {
                generation,
                result: Ok(()),
                persistence_error: None,
            },
            true,
        );

        assert!(app.snapshot.as_ref().unwrap().connected);
        assert!(app.take_reward_mint_load().is_some());
    }

    #[test]
    fn synchronize_before_dispatch_clears_work_queued_for_the_old_config() {
        let old = ConfigEntry {
            name: "old".into(),
            path: "/tmp/old".into(),
            is_default: true,
        };
        let new = ConfigEntry {
            name: "new".into(),
            path: "/tmp/new".into(),
            is_default: false,
        };
        let mut app = App::default();
        app.set_configs(vec![old.clone()]);
        app.log_in_flight = true;
        app.queued_log = Some(crate::app::LogRequest {
            generation: 1,
            core_id: 0,
            lines: 500,
        });
        let store = crate::core::CoreStateStore::new(Some(old));
        let mut changes = store.subscribe();
        store.replace_config(new);

        super::synchronize_core_projection(&mut app, &mut changes);

        assert_eq!(app.active_config().unwrap().name, "new");
        assert!(app.take_log_request().is_none());
    }

    #[test]
    fn activation_completion_after_projection_uses_controller_identity() {
        let old = ConfigEntry {
            name: "old".into(),
            path: "/tmp/old".into(),
            is_default: true,
        };
        let new = ConfigEntry {
            name: "new".into(),
            path: "/tmp/new".into(),
            is_default: false,
        };
        let mut app = App::default();
        app.set_configs(vec![old.clone()]);
        app.begin_node_refresh();
        let activation_id = app.refresh_generation;
        let store = crate::core::CoreStateStore::new(Some(old));
        let mut changes = store.subscribe();
        store.replace_config(new);
        super::synchronize_core_projection(&mut app, &mut changes);
        assert_ne!(app.refresh_generation, activation_id);
        let mut node_in_flight = true;
        let mut current_config_activation = Some(activation_id);
        let message = ServiceEvent::ConfigActivationFinished {
            generation: activation_id,
            result: Ok(()),
            persistence_error: Some("cannot remember config".into()),
        };

        let is_current_activation = super::complete_node_task(
            &message,
            &mut node_in_flight,
            &mut current_config_activation,
        );
        super::reduce_service_with_core_projection(
            &mut app,
            &mut changes,
            message,
            is_current_activation,
        );

        assert!(!node_in_flight);
        assert_eq!(current_config_activation, None);
        assert_eq!(app.active_config().unwrap().name, "new");
        assert!(!app.loading);
        assert_eq!(app.error.as_deref(), Some("cannot remember config"));
    }

    #[test]
    fn stale_config_activation_completion_does_not_release_new_identity() {
        let config = ConfigEntry {
            name: "current".into(),
            path: "/tmp/current".into(),
            is_default: true,
        };
        let mut app = App::default();
        app.set_configs(vec![config.clone()]);
        app.loading = true;
        app.error = Some("keep".into());
        let store = crate::core::CoreStateStore::new(Some(config));
        let mut changes = store.subscribe();
        let message = ServiceEvent::ConfigActivationFinished {
            generation: 1,
            result: Ok(()),
            persistence_error: Some("stale".into()),
        };
        let mut node_in_flight = true;
        let mut current_config_activation = Some(2);

        let is_current_activation = super::complete_node_task(
            &message,
            &mut node_in_flight,
            &mut current_config_activation,
        );
        super::reduce_service_with_core_projection(
            &mut app,
            &mut changes,
            message,
            is_current_activation,
        );

        assert!(!is_current_activation);
        assert!(node_in_flight);
        assert_eq!(current_config_activation, Some(2));
        assert!(app.loading);
        assert_eq!(app.error.as_deref(), Some("keep"));
    }

    #[test]
    fn config_tail_projects_before_taking_old_menu_candidate() {
        let old = ConfigEntry {
            name: "old".into(),
            path: "/tmp/old".into(),
            is_default: true,
        };
        let stale_candidate = ConfigEntry {
            name: "stale-candidate".into(),
            path: "/tmp/stale-candidate".into(),
            is_default: false,
        };
        let canonical = ConfigEntry {
            name: "canonical".into(),
            path: "/tmp/canonical".into(),
            is_default: false,
        };
        let mut app = App::default();
        app.set_configs(vec![old.clone(), stale_candidate]);
        app.queued_config_index = Some(1);
        let store = crate::core::CoreStateStore::new(Some(old));
        let mut changes = store.subscribe();
        store.replace_config(canonical);
        let mut pending_validated_config = None;

        let work = super::take_config_tail_work(
            &mut app,
            &mut changes,
            false,
            &mut pending_validated_config,
        );

        assert!(work.is_none());
        assert_eq!(app.active_config().unwrap().name, "canonical");
        assert!(app.take_config_selection().is_none());
    }

    #[test]
    fn config_activation_gate_keeps_config_dependent_work_queued() {
        let mut app = App::default();
        app.runtime.generation = 1;
        app.runtime.in_flight = true;
        app.runtime.queued = Some(crate::app::RuntimeRequest { generation: 1 });
        let service: std::sync::Arc<dyn crate::service::ConsoleService> =
            std::sync::Arc::new(crate::service::NodeConsoleService::default());
        let core = crate::core::ConsoleCore::new(service.clone(), None);
        let mut prover_write_admission = None;
        let (service_tx, _service_rx) = tokio::sync::mpsc::channel(1);
        let (settings_tx, _settings_rx) = tokio::sync::mpsc::channel(1);
        let (read_rpc_tx, _read_rpc_rx) = tokio::sync::mpsc::channel(1);
        let home = tempfile::tempdir().unwrap();

        super::dispatch_queued_work(
            &mut app,
            true,
            &mut prover_write_admission,
            &super::QueuedWorkResources {
                home: home.path(),
                core: &core,
                service: &service,
                service_tx: &service_tx,
                settings_tx: &settings_tx,
                read_rpc_tx: &read_rpc_tx,
            },
        );

        assert_eq!(app.take_runtime_request().unwrap().generation, 1);
    }

    #[test]
    fn prover_write_admission_is_foreground_first_fifo_and_completion_matched() {
        let foreground_action = crate::domain::ProverAction::Pause {
            filter: vec![0x11; 32],
        };
        let first_tracked_action = crate::domain::ProverAction::Pause {
            filter: vec![0x22; 32],
        };
        let second_tracked_action = crate::domain::ProverAction::Pause {
            filter: vec![0x33; 32],
        };
        let mut app = App::default();
        app.queued_operation = Some(crate::app::OperationRequest {
            generation: 7,
            action: foreground_action.clone(),
        });
        app.operation_generation = 7;
        let first_id = app
            .prover_operations
            .enqueue(first_tracked_action.clone(), None, 1)
            .unwrap();
        let second_id = app
            .prover_operations
            .enqueue(second_tracked_action.clone(), None, 2)
            .unwrap();
        let mut admission = None;

        let first = super::take_next_prover_write(&mut app, &mut admission).unwrap();
        assert_eq!(
            first,
            super::PendingProverWrite::Foreground(crate::app::OperationRequest {
                generation: 7,
                action: foreground_action,
            })
        );
        assert_eq!(admission, Some(super::ProverWriteAdmission::Foreground(7)));
        assert!(super::take_next_prover_write(&mut app, &mut admission).is_none());
        assert!(!super::complete_prover_write_admission(
            &ServiceEvent::ProverOperationFinished {
                id: first_id,
                result: Err("not admitted".into()),
            },
            &mut admission,
        ));

        assert!(super::complete_prover_write_admission(
            &ServiceEvent::OperationFinished {
                generation: 7,
                result: Err("done".into()),
            },
            &mut admission,
        ));
        let second = super::take_next_prover_write(&mut app, &mut admission).unwrap();
        assert_eq!(
            second,
            super::PendingProverWrite::Tracked(crate::app::ProverOperationRequest {
                id: first_id,
                action: first_tracked_action,
            })
        );
        assert_eq!(
            admission,
            Some(super::ProverWriteAdmission::Tracked(first_id))
        );
        assert!(!super::complete_prover_write_admission(
            &ServiceEvent::ProverOperationFinished {
                id: second_id,
                result: Err("not admitted".into()),
            },
            &mut admission,
        ));

        assert!(super::complete_prover_write_admission(
            &ServiceEvent::ProverOperationFinished {
                id: first_id,
                result: Err("done".into()),
            },
            &mut admission,
        ));
        let third = super::take_next_prover_write(&mut app, &mut admission).unwrap();
        assert_eq!(
            third,
            super::PendingProverWrite::Tracked(crate::app::ProverOperationRequest {
                id: second_id,
                action: second_tracked_action,
            })
        );
        assert_eq!(
            admission,
            Some(super::ProverWriteAdmission::Tracked(second_id))
        );
        assert!(super::complete_prover_write_admission(
            &ServiceEvent::ProverOperationFinished {
                id: second_id,
                result: Err("done".into()),
            },
            &mut admission,
        ));
        assert_eq!(admission, None);
    }

    #[test]
    fn stale_refresh_completion_releases_controller_in_flight() {
        let mut app = App {
            refresh_generation: 2,
            snapshot: Some(NodeSnapshot {
                config_name: "current".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let message = ServiceEvent::RefreshFinished {
            generation: 1,
            result: Ok(()),
        };
        let mut node_in_flight = true;
        let mut current_config_activation = None;

        let is_current_activation = super::complete_node_task(
            &message,
            &mut node_in_flight,
            &mut current_config_activation,
        );
        crate::app::reduce_service(&mut app, message);

        assert!(!is_current_activation);
        assert!(!node_in_flight);
        assert_eq!(current_config_activation, None);
        assert_eq!(app.snapshot.as_ref().unwrap().config_name, "current");
    }

    #[test]
    fn health_refresh_is_dispatched_to_the_health_page() {
        assert!(super::page_handles_own_refresh(Page::Health));
        assert!(!super::page_handles_own_refresh(Page::Home));
    }

    #[test]
    fn only_node_snapshot_pages_route_refresh_to_the_node_service() {
        for page in [Page::Home, Page::NodeStatus, Page::Prover] {
            assert!(!super::page_handles_own_refresh(page), "{page:?}");
        }

        for page in [
            Page::Section(crate::app::Section::Node),
            Page::Section(crate::app::Section::Prover),
            Page::Section(crate::app::Section::Wallet),
            Page::Section(crate::app::Section::Identity),
            Page::Section(crate::app::Section::Data),
            Page::Section(crate::app::Section::Diagnostics),
            Page::Configs,
            Page::ConfigPath,
            Page::Runtime,
            Page::Diagnostics,
            Page::DiagnosticsIdentity,
            Page::RemoteHealth,
            Page::Health,
            Page::PeerResolve,
            Page::Logs,
            Page::LogExport,
            Page::Keys,
            Page::KeyActions,
            Page::KeyCreate,
            Page::Aliases,
            Page::AliasActions,
            Page::AliasEdit,
            Page::ProverActions,
            Page::Shards,
            Page::ArchiveShards,
            Page::Rewards,
            Page::ConfidentialWallet,
            Page::PendingEscrows,
            Page::LegacyAssets,
            Page::Hypergraph,
            Page::Confirm,
            Page::OperationResult,
            Page::Help,
        ] {
            assert!(super::page_handles_own_refresh(page), "{page:?}");
        }
    }

    #[test]
    fn independent_health_dispatch_does_not_require_an_active_config() {
        let target = crate::domain::HealthTarget::Peer {
            peer_id: "QmTarget".into(),
            address: "node.example:8336".into(),
            identity_path: Some("/tmp/diagnostics.key".into()),
        };

        assert_eq!(
            super::health_config_path(None, &target, crate::i18n::Locale::English).unwrap(),
            PathBuf::new()
        );
        assert!(super::health_config_path(
            None,
            &crate::domain::HealthTarget::SelfNode,
            crate::i18n::Locale::English
        )
        .is_err());
    }

    #[test]
    fn config_refresh_keeps_the_previous_snapshot_during_candidate_preflight() {
        let mut app = crate::app::App {
            snapshot: Some(crate::domain::NodeSnapshot {
                config_name: "old".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut in_flight = false;

        let generation = super::request_config_refresh(&mut app, &mut in_flight);

        assert_eq!(generation, Some(1));
        assert_eq!(app.snapshot.as_ref().unwrap().config_name, "old");
        assert!(app.loading);
        assert!(in_flight);
    }

    #[test]
    fn explicit_config_refresh_starts_a_node_read_from_the_configs_page() {
        let mut app = crate::app::App {
            page: Page::Configs,
            refresh_generation: 7,
            error: Some("old error".into()),
            ..Default::default()
        };
        let mut in_flight = false;

        let generation = super::request_config_refresh(&mut app, &mut in_flight);

        assert_eq!(generation, Some(8));
        assert!(in_flight);
        assert!(app.loading);
        assert!(app.error.is_none());
    }

    #[test]
    fn q_returns_to_the_previous_page() {
        let key = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);

        assert_eq!(super::map_key(key), Some(InputEvent::Back));
    }

    #[test]
    fn q_and_escape_return_from_other_without_quitting() {
        for code in [KeyCode::Char('q'), KeyCode::Char('Q'), KeyCode::Esc] {
            let mut app = App::default();
            app.open(Page::Other);
            let input = super::map_key(KeyEvent::new(code, KeyModifiers::NONE)).unwrap();
            crate::app::reduce_input(&mut app, input);
            assert_eq!(app.page(), Page::Home);
            assert!(!app.should_quit);
        }
    }

    #[test]
    fn other_port_form_ignores_q_and_escape_cancels_before_returning() {
        let mut app = App::default();
        app.open(Page::Other);
        crate::app::reduce_input(&mut app, InputEvent::Down);
        crate::app::reduce_input(&mut app, InputEvent::Enter);
        assert!(app.is_text_editing());
        for code in [KeyCode::Char('q'), KeyCode::Char('Q')] {
            let input = super::map_form_key(KeyEvent::new(code, KeyModifiers::NONE)).unwrap();
            crate::app::reduce_input(&mut app, input);
        }
        assert_eq!(app.other.port_input, "8888");
        assert_eq!(app.page(), Page::Other);
        let escape = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        crate::app::reduce_input(&mut app, super::map_form_key(escape).unwrap());
        assert!(!app.is_text_editing());
        assert_eq!(app.page(), Page::Other);
        crate::app::reduce_input(&mut app, super::map_key(escape).unwrap());
        assert_eq!(app.page(), Page::Home);
    }

    #[test]
    fn a_maps_to_context_action_in_navigation_mode() {
        let key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);

        assert_eq!(super::map_key(key), Some(InputEvent::Character('a')));
    }

    #[test]
    fn n_opens_the_config_path_page_from_normal_pages() {
        for character in ['n', 'N'] {
            let key = KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE);

            assert_eq!(super::map_key(key), Some(InputEvent::OpenConfigPath));
        }
    }

    #[test]
    fn n_and_q_are_available_to_form_pages() {
        for character in ['n', 'N', 'q', 'Q'] {
            let key = KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE);

            assert_eq!(
                super::map_form_key(key),
                Some(InputEvent::Character(character))
            );
        }
    }

    #[test]
    fn prepare_config_path_does_not_remember_candidate_before_core_preflight() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let old = create_config_dir(temp.path(), "old");
        let candidate = create_config_dir(temp.path(), "candidate");
        config::remember_config_dir(&home, &old).unwrap();

        let entry = super::prepare_config_path(&home, &candidate).unwrap();

        assert_eq!(entry.name, "candidate");
        assert_eq!(
            std::fs::read_to_string(config::settings_file(&home)).unwrap(),
            std::fs::canonicalize(old).unwrap().to_string_lossy()
        );
    }

    #[test]
    fn initialize_app_activates_candidate_without_remembering_before_preflight() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let cwd = temp.path().join("cwd");
        std::fs::create_dir_all(&cwd).unwrap();
        let config_dir = create_config_dir(temp.path(), "direct-config");

        let (app, refresh) = super::initialize_app(
            RunOptions::Run {
                explicit_config: Some(config_dir),
            },
            &cwd,
            &home,
        );

        assert_eq!(app.page(), Page::Home);
        assert_eq!(app.active_config(), refresh.as_ref());
        assert!(!config::settings_file(&home).exists());
    }

    #[test]
    fn initialize_app_requires_a_path_when_no_config_is_available() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("cwd");
        let home = temp.path().join("home");
        std::fs::create_dir_all(&cwd).unwrap();

        let (app, refresh) = super::initialize_app(
            RunOptions::Run {
                explicit_config: None,
            },
            &cwd,
            &home,
        );

        assert_eq!(app.page(), Page::ConfigPath);
        assert!(app.config_path_required);
        assert!(app.config_path_error.is_some());
        assert!(refresh.is_none());
    }

    #[test]
    fn initialize_app_keeps_candidate_when_memory_is_not_writable() {
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().join("cwd");
        let home = temp.path().join("home-is-a-file");
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::write(&home, "not a directory").unwrap();
        let config_dir = create_config_dir(temp.path(), "direct-config");

        let (app, refresh) = super::initialize_app(
            RunOptions::Run {
                explicit_config: Some(config_dir.clone()),
            },
            &cwd,
            &home,
        );

        assert_eq!(app.page(), Page::Home);
        assert_eq!(app.active_config(), refresh.as_ref());
        assert!(refresh.is_some());
    }

    #[test]
    fn queued_config_path_waits_for_node_refresh_then_starts_while_locked() {
        let temp = tempfile::tempdir().unwrap();
        let config_dir = create_config_dir(temp.path(), "next-config");
        let mut app = App::default();
        app.require_config_path(config_dir.to_string_lossy().into_owned(), None);
        crate::app::reduce_input(&mut app, InputEvent::Enter);

        assert!(app.config_path_in_flight);
        assert!(super::take_pending_config_work(&mut app, true).is_none());
        assert!(app.queued_config_path.is_some());

        let work = super::take_pending_config_work(&mut app, false).unwrap();

        assert_eq!(
            work,
            PendingConfigWork::Validate(config_dir.to_string_lossy().into_owned())
        );
        assert!(app.queued_config_path.is_none());
        assert!(app.config_path_in_flight);
        assert!(super::take_pending_config_work(&mut app, false).is_none());
    }

    #[test]
    fn failed_path_validation_preserves_and_then_releases_a_queued_selection() {
        let mut app = App::default();
        app.set_configs(vec![
            ConfigEntry {
                name: "current".into(),
                path: "/tmp/current".into(),
                is_default: true,
            },
            ConfigEntry {
                name: "backup".into(),
                path: "/tmp/backup".into(),
                is_default: false,
            },
        ]);
        app.open(Page::Configs);
        crate::app::reduce_input(&mut app, InputEvent::Down);
        crate::app::reduce_input(&mut app, InputEvent::Enter);
        app.require_config_path("/missing/config".into(), None);
        crate::app::reduce_input(&mut app, InputEvent::Enter);

        assert_eq!(
            super::take_pending_config_work(&mut app, false),
            Some(PendingConfigWork::Validate("/missing/config".into()))
        );
        assert_eq!(app.queued_config_index, Some(1));

        app.config_path_in_flight = true;
        super::apply_config_path_result(&mut app, Err("invalid path".into()));
        assert!(!app.config_path_in_flight);

        assert_eq!(
            super::take_pending_config_work(&mut app, false),
            Some(PendingConfigWork::Refresh(ConfigEntry {
                name: "backup".into(),
                path: "/tmp/backup".into(),
                is_default: false,
            }))
        );
    }

    #[test]
    fn delayed_optional_validation_cannot_leave_the_locked_config_path() {
        let mut app = App::default();
        app.set_configs(vec![ConfigEntry {
            name: "current".into(),
            path: "/tmp/current".into(),
            is_default: true,
        }]);
        app.open(Page::Configs);
        crate::app::reduce_input(&mut app, InputEvent::OpenConfigPath);
        app.config_path_input = "/tmp/next".into();
        crate::app::reduce_input(&mut app, InputEvent::Enter);

        assert!(super::take_pending_config_work(&mut app, true).is_none());
        crate::app::reduce_input(&mut app, InputEvent::Back);
        assert_eq!(app.page(), Page::ConfigPath);
        assert_eq!(app.config_path_input, "/tmp/next");
        assert!(app.config_path_in_flight);

        assert_eq!(
            super::take_pending_config_work(&mut app, false),
            Some(PendingConfigWork::Validate("/tmp/next".into()))
        );
        super::begin_config_path_validation(&mut app);

        assert_eq!(app.page(), Page::ConfigPath);
        assert_eq!(app.route_stack.last(), Some(&Page::ConfigPath));
        assert!(app.config_path_in_flight);
    }

    #[tokio::test]
    async fn blocking_config_worker_returns_candidate_without_applying_it() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let config_dir = create_config_dir(temp.path(), "next-config");
        let mut app = App::default();
        app.set_configs(vec![ConfigEntry {
            name: "old".into(),
            path: "/tmp/old".into(),
            is_default: true,
        }]);
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);

        app.config_path_in_flight = true;
        super::start_config_path_validation(home, config_dir, crate::i18n::Locale::English, tx);
        assert_eq!(app.active_config().unwrap().name, "old");

        let result = rx.recv().await.unwrap();
        assert_eq!(app.active_config().unwrap().name, "old");
        assert!(matches!(&result, ConfigPathWorkerResult::Finished(Ok(_))));
        let entry = super::consume_config_path_worker_result(&mut app, result)
            .unwrap()
            .unwrap();

        assert!(!app.config_path_in_flight);
        assert_eq!(entry.name, "next-config");
        assert_eq!(app.active_config().unwrap().name, "old");
    }

    #[tokio::test]
    async fn blocking_config_worker_applies_failure_only_when_result_is_consumed() {
        let temp = tempfile::tempdir().unwrap();
        let mut app = App::default();
        app.set_configs(vec![ConfigEntry {
            name: "old".into(),
            path: "/tmp/old".into(),
            is_default: true,
        }]);
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);

        app.config_path_in_flight = true;
        super::start_config_path_validation(
            temp.path().join("home"),
            temp.path().join("missing"),
            crate::i18n::Locale::English,
            tx,
        );
        assert!(app.config_path_error.is_none());

        let result = rx.recv().await.unwrap();
        assert!(app.config_path_error.is_none());
        assert!(matches!(&result, ConfigPathWorkerResult::Finished(Err(_))));
        assert!(super::consume_config_path_worker_result(&mut app, result)
            .unwrap()
            .is_none());

        assert!(!app.config_path_in_flight);
        assert_eq!(app.active_config().unwrap().name, "old");
        assert!(app.config_path_error.is_some());
        assert_eq!(app.page(), Page::ConfigPath);
    }

    #[tokio::test]
    async fn panicking_config_worker_returns_a_fatal_result() {
        let mut app = App::default();
        app.require_config_path("/tmp/node".into(), None);
        super::begin_config_path_validation(&mut app);
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);

        super::spawn_config_task(
            || -> Result<ConfigEntry, String> { panic!("boom") },
            crate::i18n::Locale::English,
            tx,
        );

        let result = rx.recv().await.expect("panic must still send a result");
        assert!(matches!(
            &result,
            ConfigPathWorkerResult::Fatal(message)
                if message.contains("Configuration validation task failed")
        ));

        let error = super::consume_config_path_worker_result(&mut app, result).unwrap_err();
        assert!(error
            .to_string()
            .contains("Configuration validation task failed"));
    }
}
