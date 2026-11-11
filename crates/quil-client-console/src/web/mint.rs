use axum::{extract::State, http::HeaderMap, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use crate::{
    core::{OperationOrigin, RewardMintRun},
    domain::{RewardMintPhase, RewardMintStatus},
    service::ServiceError,
};

use super::{require_session, ApiError, AppState};

const MAX_RETAINED_POLL_ERRORS: usize = 32;

#[derive(Default)]
struct MintTaskState {
    active: HashSet<PathBuf>,
    errors: HashMap<PathBuf, String>,
    error_order: VecDeque<PathBuf>,
}

impl MintTaskState {
    fn remember_error(&mut self, key: PathBuf, error: String) {
        self.error_order.retain(|stored| stored != &key);
        self.error_order.push_back(key.clone());
        self.errors.insert(key, error);
        while self.errors.len() > MAX_RETAINED_POLL_ERRORS {
            let Some(oldest) = self.error_order.pop_front() else {
                break;
            };
            self.errors.remove(&oldest);
        }
    }

    fn forget_error(&mut self, key: &Path) {
        self.errors.remove(key);
        self.error_order.retain(|stored| stored != key);
    }
}

#[derive(Clone, Default)]
pub(super) struct ActiveMintTasks(Arc<Mutex<MintTaskState>>);

impl ActiveMintTasks {
    fn try_start(&self, config_dir: &Path) -> bool {
        let key = mint_task_key(config_dir);
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let started = state.active.insert(key.clone());
        if started {
            state.forget_error(&key);
        }
        started
    }

    fn finish(&self, config_dir: &Path) {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .active
            .remove(&mint_task_key(config_dir));
    }

    fn fail(&self, config_dir: &Path, error: String) {
        let key = mint_task_key(config_dir);
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.active.remove(&key);
        state.remember_error(key, error);
    }

    fn retain_error(&self, config_dir: &Path, error: String) {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remember_error(mint_task_key(config_dir), error);
    }

    pub(super) fn poll_error(&self, config_dir: &Path) -> Option<String> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .errors
            .get(&mint_task_key(config_dir))
            .cloned()
    }

    fn clear_error(&self, config_dir: &Path) {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .forget_error(&mint_task_key(config_dir));
    }
}

fn mint_task_key(config_dir: &Path) -> PathBuf {
    std::fs::canonicalize(config_dir).unwrap_or_else(|_| config_dir.to_path_buf())
}

#[derive(Serialize)]
pub(super) struct MintResponse {
    message: &'static str,
    receipt: String,
    recipient: String,
    phase: RewardMintPhase,
    detail: Option<String>,
    next_poll_in_secs: Option<u64>,
    can_rebroadcast: bool,
}

#[derive(Deserialize)]
pub(super) struct RebroadcastRequest {
    confirmed: bool,
}

pub(super) async fn start(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<MintResponse>, ApiError> {
    require_session(&headers, &state.auth)?;
    let (run, should_continue) = admit_mint_run(
        &state.mint_tasks,
        state.core.mint_rewards(OperationOrigin::Web).await,
    )
    .map_err(mint_error)?;
    let response = MintResponse::from(run.status.clone());
    if should_continue {
        continue_mint(state, run);
    }
    Ok(Json(response))
}

pub(super) async fn rebroadcast(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<RebroadcastRequest>,
) -> Result<Json<MintResponse>, ApiError> {
    require_session(&headers, &state.auth)?;
    if !request.confirmed {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Explicit confirmation is required before rebroadcasting the saved Mint claim",
        ));
    }
    let (run, should_continue) = admit_mint_run(
        &state.mint_tasks,
        state
            .core
            .rebroadcast_reward_mint(OperationOrigin::Web, true)
            .await,
    )
    .map_err(mint_error)?;
    let response = MintResponse::from(run.status.clone());
    if should_continue {
        continue_mint(state, run);
    }
    Ok(Json(response))
}

pub(super) fn resume(state: AppState) {
    tokio::spawn(async move {
        if let Ok(Some(run)) = state.core.load_reward_mint_run().await {
            if let Ok((run, true)) = admit_mint_run(&state.mint_tasks, Ok(run)) {
                continue_mint(state, run);
            }
        }
    });
}

fn continue_mint(state: AppState, run: RewardMintRun) {
    if run.status.next_poll_in_secs.is_none() {
        return;
    }
    tokio::spawn(async move {
        let config_dir = run.config_dir;
        let mut next_poll_in_secs = run.status.next_poll_in_secs;
        let mut transient_failures = 0;
        while let Some(delay) = next_poll_in_secs {
            tokio::time::sleep(std::time::Duration::from_secs(delay.max(1))).await;
            match apply_poll_result(
                &state.mint_tasks,
                &config_dir,
                state.core.advance_reward_mint_at(&config_dir).await,
                &mut transient_failures,
            ) {
                MintPollStep::Advanced(status) => {
                    next_poll_in_secs = status.next_poll_in_secs;
                }
                MintPollStep::RetryAfter(delay) => next_poll_in_secs = Some(delay),
                MintPollStep::Stop => return,
            }
        }
        state.mint_tasks.finish(&config_dir);
    });
}

fn admit_mint_run(
    tasks: &ActiveMintTasks,
    result: Result<RewardMintRun, ServiceError>,
) -> Result<(RewardMintRun, bool), ServiceError> {
    let run = result?;
    let should_continue = if run.status.next_poll_in_secs.is_some() {
        tasks.try_start(&run.config_dir)
    } else {
        tasks.clear_error(&run.config_dir);
        false
    };
    Ok((run, should_continue))
}

#[derive(Debug, PartialEq, Eq)]
enum MintPollStep {
    Advanced(RewardMintStatus),
    RetryAfter(u64),
    Stop,
}

fn apply_poll_result(
    tasks: &ActiveMintTasks,
    config_dir: &Path,
    result: Result<RewardMintStatus, ServiceError>,
    transient_failures: &mut u32,
) -> MintPollStep {
    match result {
        Ok(status) => {
            *transient_failures = 0;
            tasks.clear_error(config_dir);
            MintPollStep::Advanced(status)
        }
        Err(error) if mint_is_finished(&error) => {
            tasks.clear_error(config_dir);
            tasks.finish(config_dir);
            MintPollStep::Stop
        }
        Err(error) if mint_error_is_transient(&error) => {
            *transient_failures = transient_failures.saturating_add(1);
            tasks.retain_error(config_dir, error.to_string());
            MintPollStep::RetryAfter(transient_retry_delay(*transient_failures))
        }
        Err(error) => {
            tasks.fail(config_dir, error.to_string());
            MintPollStep::Stop
        }
    }
}

fn mint_error_is_transient(error: &ServiceError) -> bool {
    matches!(
        error,
        ServiceError::Connect(_) | ServiceError::Rpc(_) | ServiceError::Timeout
    )
}

fn transient_retry_delay(failures: u32) -> u64 {
    match failures {
        0 | 1 => 15,
        2 => 30,
        _ => 60,
    }
}

fn mint_error(error: ServiceError) -> ApiError {
    let status = match &error {
        ServiceError::InvalidInput(_) => StatusCode::BAD_REQUEST,
        ServiceError::Conflict(_) => StatusCode::CONFLICT,
        ServiceError::Wallet(message) if wallet_conflict(message) => StatusCode::CONFLICT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    ApiError::new(status, error.to_string())
}

fn wallet_conflict(message: &str) -> bool {
    message.contains("unfinished Reward Mint") || message.contains("Another Console")
}

fn mint_is_finished(error: &ServiceError) -> bool {
    matches!(error, ServiceError::Wallet(message) if message.contains("No unfinished Reward Mint"))
}

impl From<RewardMintStatus> for MintResponse {
    fn from(status: RewardMintStatus) -> Self {
        Self {
            message: "Reward Mint started",
            receipt: status.receipt,
            recipient: status.recipient,
            phase: status.phase,
            detail: status.detail,
            next_poll_in_secs: status.next_poll_in_secs,
            can_rebroadcast: status.can_rebroadcast,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{admit_mint_run, apply_poll_result, ActiveMintTasks, MintPollStep};
    use crate::{
        core::RewardMintRun,
        domain::{RewardMintPhase, RewardMintStatus},
        service::ServiceError,
    };
    use std::path::{Path, PathBuf};

    fn status(phase: RewardMintPhase) -> RewardMintStatus {
        RewardMintStatus {
            receipt: "receipt".into(),
            recipient: "recipient".into(),
            phase,
            resume_phase: None,
            detail: None,
            next_poll_in_secs: Some(15),
            can_rebroadcast: false,
        }
    }

    #[test]
    fn active_mint_tasks_deduplicate_one_config_until_finished() {
        let tasks = ActiveMintTasks::default();
        let config = Path::new("/node/.config");

        assert!(tasks.try_start(config));
        assert!(!tasks.try_start(config));
        tasks.finish(config);
        assert!(tasks.try_start(config));
    }

    #[test]
    fn mint_run_admission_retains_failed_and_duplicate_start_errors() {
        let tasks = ActiveMintTasks::default();
        let selected = Path::new("/node/selected");
        let other = Path::new("/node/other");
        tasks.retain_error(selected, "previous selected error".into());
        tasks.retain_error(other, "previous other error".into());

        let failed = admit_mint_run(
            &tasks,
            Err(ServiceError::Conflict("Mint is already running".into())),
        );
        assert!(failed.is_err());
        assert_eq!(
            tasks.poll_error(selected).as_deref(),
            Some("previous selected error"),
            "a failed or duplicate start must retain the prior poll error"
        );

        assert!(tasks.try_start(selected));
        tasks.retain_error(selected, "active poll error".into());
        let (duplicate, should_continue) = admit_mint_run(
            &tasks,
            Ok(RewardMintRun {
                config_dir: PathBuf::from(selected),
                status: status(RewardMintPhase::ClaimSubmitted),
            }),
        )
        .unwrap();
        assert_eq!(duplicate.config_dir, selected);
        assert!(!should_continue);
        assert_eq!(
            tasks.poll_error(selected).as_deref(),
            Some("active poll error")
        );

        tasks.finish(selected);
        let (admitted, should_continue) = admit_mint_run(
            &tasks,
            Ok(RewardMintRun {
                config_dir: PathBuf::from(selected),
                status: status(RewardMintPhase::ClaimSubmitted),
            }),
        )
        .unwrap();
        assert_eq!(admitted.config_dir, selected);
        assert!(should_continue);
        assert_eq!(tasks.poll_error(selected), None);
        assert_eq!(
            tasks.poll_error(other).as_deref(),
            Some("previous other error"),
            "success must clear the run's config, not whichever config is currently selected"
        );
    }

    #[test]
    fn retained_poll_errors_are_bounded_and_evict_the_oldest_config() {
        const EXPECTED_LIMIT: usize = 32;
        let tasks = ActiveMintTasks::default();
        let configs = (0..EXPECTED_LIMIT)
            .map(|index| PathBuf::from(format!("/node/config-{index:02}")))
            .collect::<Vec<_>>();
        for config in &configs {
            tasks.retain_error(config, format!("error for {}", config.display()));
        }

        tasks.retain_error(&configs[0], "refreshed oldest error".into());
        let newest = PathBuf::from("/node/config-32");
        tasks.retain_error(&newest, "newest error".into());

        let retained_count = tasks
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .errors
            .len();
        assert_eq!(retained_count, EXPECTED_LIMIT);
        assert_eq!(
            tasks.poll_error(&configs[0]).as_deref(),
            Some("refreshed oldest error"),
            "updating an error should make it the newest retained entry"
        );
        assert_eq!(tasks.poll_error(&configs[1]), None);
        assert_eq!(tasks.poll_error(&newest).as_deref(), Some("newest error"));
    }

    #[test]
    fn deterministic_poll_error_stops_releases_and_remains_visible_until_restart() {
        let tasks = ActiveMintTasks::default();
        let config = Path::new("/node/.config");
        assert!(tasks.try_start(config));
        let mut failures = 0;

        let next = apply_poll_result(
            &tasks,
            config,
            Err(ServiceError::Wallet("corrupt Mint journal".into())),
            &mut failures,
        );

        assert_eq!(next, MintPollStep::Stop);
        assert_eq!(
            tasks.poll_error(config).as_deref(),
            Some("Cannot read Confidential wallet: corrupt Mint journal")
        );
        tasks.clear_error(config);
        assert_eq!(
            tasks.poll_error(config),
            None,
            "a new start clears the error"
        );
        assert!(
            tasks.try_start(config),
            "the active marker must be released"
        );
    }

    #[test]
    fn journaled_recoverable_status_keeps_background_poll_active() {
        let tasks = ActiveMintTasks::default();
        let config = Path::new("/node/.config");
        assert!(tasks.try_start(config));
        let recoverable = status(RewardMintPhase::RecoverableError);
        let mut failures = 0;

        let next = apply_poll_result(&tasks, config, Ok(recoverable.clone()), &mut failures);

        assert_eq!(next, MintPollStep::Advanced(recoverable));
        assert_eq!(tasks.poll_error(config), None);
        assert!(!tasks.try_start(config), "the existing poll remains active");
    }

    #[test]
    fn transient_poll_errors_back_off_and_eventual_success_clears_the_error() {
        let tasks = ActiveMintTasks::default();
        let config = Path::new("/node/.config");
        assert!(tasks.try_start(config));
        let mut failures = 0;

        for (error, expected_delay) in [
            (ServiceError::Connect("offline".into()), 15),
            (ServiceError::Rpc("read failed".into()), 30),
            (ServiceError::Timeout, 60),
            (ServiceError::Timeout, 60),
        ] {
            assert_eq!(
                apply_poll_result(&tasks, config, Err(error), &mut failures),
                MintPollStep::RetryAfter(expected_delay)
            );
            assert!(tasks.poll_error(config).is_some());
            assert!(
                !tasks.try_start(config),
                "transient failures keep the poll active"
            );
        }

        let recovered = status(RewardMintPhase::ClaimSubmitted);
        assert_eq!(
            apply_poll_result(&tasks, config, Ok(recovered.clone()), &mut failures),
            MintPollStep::Advanced(recovered)
        );
        assert_eq!(failures, 0);
        assert_eq!(tasks.poll_error(config), None);
        assert!(
            !tasks.try_start(config),
            "success continues the existing poll"
        );
    }
}
