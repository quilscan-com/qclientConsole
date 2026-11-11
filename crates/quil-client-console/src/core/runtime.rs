use std::{path::PathBuf, sync::Arc, time::Duration};

use tokio::{
    sync::{watch, Semaphore},
    task::JoinHandle,
};

use crate::{
    capability::{
        build_invocation, CapabilityCatalog, CapabilityRunner, CapabilityTarget, CapabilityTask,
        CapabilityTaskStore, CapabilityValues, CurrentExeRunner, MAX_CONCURRENT_CAPABILITY_TASKS,
    },
    config::ConfigEntry,
    domain::{NodeSnapshot, OperationResult, ProverAction},
    service::{ConsoleService, ServiceError},
};

pub const CAPABILITY_TASK_TIMEOUT: Duration = Duration::from_secs(30 * 60);

use super::{
    CoreStateSnapshot, CoreStateStore, OperationCoordinator, OperationLease, OperationOrigin,
};

pub struct PreparedConfigSwitch {
    state: CoreStateStore,
    candidate: ConfigEntry,
    snapshot: NodeSnapshot,
    _lease: OperationLease,
}

impl PreparedConfigSwitch {
    pub fn commit(self) -> NodeSnapshot {
        self.state
            .replace_config_with_snapshot(self.candidate, self.snapshot.clone());
        self.snapshot
    }
}

#[derive(Clone)]
pub struct ConsoleCore {
    service: Arc<dyn ConsoleService>,
    state: CoreStateStore,
    operations: OperationCoordinator,
    catalog: Arc<CapabilityCatalog>,
    capability_initialization_error: Option<Arc<str>>,
    capability_runner: Arc<dyn CapabilityRunner>,
    capability_tasks: CapabilityTaskStore,
    capability_slots: Arc<Semaphore>,
    capability_timeout: Duration,
}

impl ConsoleCore {
    pub fn new(service: Arc<dyn ConsoleService>, active_config: Option<ConfigEntry>) -> Self {
        Self::new_with_signature_check(service, active_config, true)
    }

    pub fn new_with_signature_check(
        service: Arc<dyn ConsoleService>,
        active_config: Option<ConfigEntry>,
        signature_check: bool,
    ) -> Self {
        Self::new_with_capability_dependencies(
            service,
            active_config,
            Arc::new(CurrentExeRunner::current_with_signature_check(
                signature_check,
            )),
            CapabilityCatalog::discover(),
            CAPABILITY_TASK_TIMEOUT,
        )
    }

    #[doc(hidden)]
    pub fn new_with_capability_runner(
        service: Arc<dyn ConsoleService>,
        active_config: Option<ConfigEntry>,
        capability_runner: Arc<dyn CapabilityRunner>,
    ) -> Self {
        Self::new_with_capability_dependencies(
            service,
            active_config,
            capability_runner,
            CapabilityCatalog::discover(),
            CAPABILITY_TASK_TIMEOUT,
        )
    }

    #[doc(hidden)]
    pub fn new_with_capability_runner_and_timeout(
        service: Arc<dyn ConsoleService>,
        active_config: Option<ConfigEntry>,
        capability_runner: Arc<dyn CapabilityRunner>,
        capability_timeout: Duration,
    ) -> Self {
        Self::new_with_capability_dependencies(
            service,
            active_config,
            capability_runner,
            CapabilityCatalog::discover(),
            capability_timeout,
        )
    }

    fn new_with_capability_dependencies(
        service: Arc<dyn ConsoleService>,
        active_config: Option<ConfigEntry>,
        capability_runner: Arc<dyn CapabilityRunner>,
        catalog: anyhow::Result<CapabilityCatalog>,
        capability_timeout: Duration,
    ) -> Self {
        let (catalog, capability_initialization_error) = match catalog {
            Ok(catalog) => (catalog, None),
            Err(error) => (
                CapabilityCatalog::empty(),
                Some(Arc::<str>::from(format!(
                    "Cannot initialize qclient capabilities: {error:#}"
                ))),
            ),
        };
        Self {
            service,
            state: CoreStateStore::new(active_config),
            operations: OperationCoordinator::default(),
            catalog: Arc::new(catalog),
            capability_initialization_error,
            capability_runner,
            capability_tasks: CapabilityTaskStore::default(),
            capability_slots: Arc::new(Semaphore::new(MAX_CONCURRENT_CAPABILITY_TASKS)),
            capability_timeout: capability_timeout.min(CAPABILITY_TASK_TIMEOUT),
        }
    }

    pub fn capabilities(&self) -> &CapabilityCatalog {
        &self.catalog
    }

    pub fn capability_task(&self, id: u64) -> Option<CapabilityTask> {
        self.capability_tasks.get(id)
    }

    pub async fn start_capability(
        &self,
        origin: OperationOrigin,
        id: &str,
        values: CapabilityValues,
        confirmed: bool,
    ) -> Result<CapabilityTask, ServiceError> {
        let (running, supervisor) = self.spawn_capability(origin, id, values, confirmed)?;
        let task_id = running.id;
        match supervisor.await {
            Ok(result) => result,
            Err(join_error) => {
                let error = ServiceError::Operation(format!(
                    "Capability supervisor task failed: {join_error}"
                ));
                finish_with_error(&self.capability_tasks, task_id, error)
            }
        }
    }

    pub fn enqueue_capability(
        &self,
        origin: OperationOrigin,
        id: &str,
        values: CapabilityValues,
        confirmed: bool,
    ) -> Result<CapabilityTask, ServiceError> {
        let (running, _supervisor) = self.spawn_capability(origin, id, values, confirmed)?;
        Ok(running)
    }

    fn spawn_capability(
        &self,
        origin: OperationOrigin,
        id: &str,
        values: CapabilityValues,
        confirmed: bool,
    ) -> Result<
        (
            CapabilityTask,
            JoinHandle<Result<CapabilityTask, ServiceError>>,
        ),
        ServiceError,
    > {
        if let Some(error) = &self.capability_initialization_error {
            return Err(ServiceError::Operation(error.to_string()));
        }
        let capability = self
            .catalog
            .get(id)
            .cloned()
            .ok_or_else(|| ServiceError::InvalidInput(format!("Unknown capability {id}")))?;
        if let CapabilityTarget::Native { route } = &capability.target {
            return Err(ServiceError::InvalidInput(format!(
                "Capability {id} is available through native route {route}"
            )));
        }
        if !capability.read_only && !confirmed {
            return Err(ServiceError::InvalidInput(format!(
                "Confirmation is required for capability {id}"
            )));
        }

        let active_config = self.active_config_path()?;
        let redactions = capability
            .fields
            .iter()
            .filter(|field| field.sensitive)
            .filter_map(|field| values.get(&field.id))
            .flatten()
            .filter(|value| !value.is_empty())
            .cloned()
            .collect();
        let invocation = build_invocation(&capability, &active_config, &values, confirmed)
            .map_err(|error| ServiceError::InvalidInput(error.to_string()))?;
        let permit = self
            .capability_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| {
                ServiceError::Conflict(format!(
                    "At most {MAX_CONCURRENT_CAPABILITY_TASKS} capability tasks may run at once"
                ))
            })?;
        let lease = if invocation.read_only {
            None
        } else {
            Some(
                self.operations
                    .try_begin(origin, format!("capability {id}"))
                    .map_err(|error| ServiceError::Conflict(error.to_string()))?,
            )
        };
        let running = self
            .capability_tasks
            .try_start_with_redactions(id, redactions)
            .map_err(|error| ServiceError::Conflict(error.to_string()))?;

        let runner = self.capability_runner.clone();
        let runner_task = tokio::spawn(async move { runner.run(invocation).await });
        let tasks = self.capability_tasks.clone();
        let task_id = running.id;
        let capability_id = id.to_owned();
        let capability_timeout = self.capability_timeout;
        let supervisor = tokio::spawn(async move {
            let _lease = lease;
            let _permit = permit;
            let mut runner_task = runner_task;
            match tokio::time::timeout(capability_timeout, &mut runner_task).await {
                Ok(result) => finish_runner_task(&tasks, task_id, result),
                Err(_) => {
                    runner_task.abort();
                    let _ = runner_task.await;
                    finish_with_error(
                        &tasks,
                        task_id,
                        ServiceError::Operation(format!(
                            "Capability {capability_id} timed out after {} seconds",
                            capability_timeout.as_secs()
                        )),
                    )
                }
            }
        });
        Ok((running, supervisor))
    }

    pub fn state(&self) -> CoreStateSnapshot {
        self.state.snapshot()
    }

    pub fn subscribe(&self) -> watch::Receiver<CoreStateSnapshot> {
        self.state.subscribe()
    }

    pub fn operations(&self) -> &OperationCoordinator {
        &self.operations
    }

    pub(super) fn service(&self) -> &Arc<dyn ConsoleService> {
        &self.service
    }

    pub(super) fn active_config_path(&self) -> Result<PathBuf, ServiceError> {
        self.state()
            .active_config
            .map(|entry| entry.path)
            .ok_or_else(|| ServiceError::InvalidInput("Select a node configuration first".into()))
    }

    pub async fn execute_prover(
        &self,
        origin: OperationOrigin,
        action: ProverAction,
    ) -> Result<OperationResult, ServiceError> {
        self.state.snapshot().active_config.ok_or_else(|| {
            ServiceError::InvalidInput("Select a node configuration first".into())
        })?;
        let label = action.label().to_owned();
        let _lease = self
            .operations
            .try_begin(origin, label)
            .map_err(|error| ServiceError::Conflict(error.to_string()))?;
        let entry = self.state.snapshot().active_config.ok_or_else(|| {
            ServiceError::InvalidInput("Select a node configuration first".into())
        })?;

        self.service.execute_prover(&entry.path, action).await
    }

    pub async fn switch_config(
        &self,
        candidate: ConfigEntry,
    ) -> Result<NodeSnapshot, ServiceError> {
        self.switch_config_from(OperationOrigin::Tui, candidate)
            .await
    }

    pub async fn switch_config_from(
        &self,
        origin: OperationOrigin,
        candidate: ConfigEntry,
    ) -> Result<NodeSnapshot, ServiceError> {
        Ok(self
            .prepare_config_switch(origin, candidate)
            .await?
            .commit())
    }

    pub async fn prepare_config_switch(
        &self,
        origin: OperationOrigin,
        candidate: ConfigEntry,
    ) -> Result<PreparedConfigSwitch, ServiceError> {
        let lease = self
            .operations
            .try_begin(origin, "switch-config")
            .map_err(|error| ServiceError::Conflict(error.to_string()))?;
        let snapshot = self
            .service
            .snapshot(&candidate.name, &candidate.path)
            .await?;

        Ok(PreparedConfigSwitch {
            state: self.state.clone(),
            candidate,
            snapshot,
            _lease: lease,
        })
    }

    pub async fn refresh(&self) -> Result<NodeSnapshot, ServiceError> {
        let ticket = self.state.begin_refresh().ok_or_else(|| {
            ServiceError::InvalidInput("Select a node configuration first".into())
        })?;

        let result = self
            .service
            .snapshot(&ticket.active_config.name, &ticket.active_config.path)
            .await;

        match result {
            Ok(snapshot) => {
                if !self.state.replace_refreshed_node_snapshot(
                    ticket.config_generation,
                    ticket.refresh_generation,
                    snapshot.clone(),
                ) {
                    return Err(ServiceError::Conflict("Node refresh was superseded".into()));
                }
                Ok(snapshot)
            }
            Err(error) => {
                if !self
                    .state
                    .is_refresh_current(ticket.config_generation, ticket.refresh_generation)
                {
                    return Err(ServiceError::Conflict("Node refresh was superseded".into()));
                }
                Err(error)
            }
        }
    }
}

fn finish_runner_task(
    tasks: &CapabilityTaskStore,
    task_id: u64,
    result: Result<
        Result<crate::capability::CapabilityRunResult, ServiceError>,
        tokio::task::JoinError,
    >,
) -> Result<CapabilityTask, ServiceError> {
    match result {
        Ok(Ok(result)) => tasks
            .finish(
                task_id,
                Some(result.exit_code),
                result.stdout,
                result.stderr,
            )
            .map_err(|error| ServiceError::Operation(error.to_string())),
        Ok(Err(error)) => finish_with_error(tasks, task_id, error),
        Err(join_error) => finish_with_error(
            tasks,
            task_id,
            ServiceError::Operation(format!("Capability runner task failed: {join_error}")),
        ),
    }
}

fn finish_with_error(
    tasks: &CapabilityTaskStore,
    task_id: u64,
    error: ServiceError,
) -> Result<CapabilityTask, ServiceError> {
    match tasks.finish(task_id, None, "", error.to_string()) {
        Ok(_) => Err(error),
        Err(finish_error) => Err(ServiceError::Operation(format!(
            "{error}; cannot finish capability task {task_id}: {finish_error}"
        ))),
    }
}

#[cfg(test)]
mod capability_initialization_tests {
    use super::*;
    use crate::service::NodeConsoleService;

    #[tokio::test]
    async fn catalog_discovery_error_does_not_panic_core_construction() {
        let core = ConsoleCore::new_with_capability_dependencies(
            Arc::new(NodeConsoleService::default()),
            None,
            Arc::new(CurrentExeRunner::new("/not-run")),
            Err(anyhow::anyhow!("catalog discovery failed")),
            CAPABILITY_TASK_TIMEOUT,
        );

        assert_eq!(core.capabilities().iter().count(), 0);
        let error = core
            .start_capability(
                OperationOrigin::Web,
                "config.print",
                CapabilityValues::new(),
                false,
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("catalog discovery failed"));
    }

    #[test]
    fn capability_timeout_cannot_exceed_the_runtime_limit() {
        let core = ConsoleCore::new_with_capability_dependencies(
            Arc::new(NodeConsoleService::default()),
            None,
            Arc::new(CurrentExeRunner::new("/not-run")),
            Ok(CapabilityCatalog::empty()),
            CAPABILITY_TASK_TIMEOUT + Duration::from_secs(1),
        );

        assert_eq!(core.capability_timeout, CAPABILITY_TASK_TIMEOUT);
    }
}
