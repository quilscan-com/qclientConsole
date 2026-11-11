use std::fmt::{Display, Formatter};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};

use tokio::sync::watch;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationOrigin {
    Tui,
    Web,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveOperation {
    pub id: u64,
    pub origin: OperationOrigin,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationBusy {
    pub active: ActiveOperation,
}

impl Display for OperationBusy {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} is already running from {:?}",
            self.active.label, self.active.origin
        )
    }
}

impl std::error::Error for OperationBusy {}

#[derive(Debug)]
struct Shared {
    next_id: AtomicU64,
    active: Mutex<Option<ActiveOperation>>,
    tx: watch::Sender<Option<ActiveOperation>>,
}

impl Shared {
    fn lock_active(&self) -> std::sync::MutexGuard<'_, Option<ActiveOperation>> {
        self.active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[derive(Debug, Clone)]
pub struct OperationCoordinator {
    shared: Arc<Shared>,
}

impl Default for OperationCoordinator {
    fn default() -> Self {
        let (tx, _) = watch::channel(None);
        Self {
            shared: Arc::new(Shared {
                next_id: AtomicU64::new(1),
                active: Mutex::new(None),
                tx,
            }),
        }
    }
}

impl OperationCoordinator {
    pub fn active(&self) -> Option<ActiveOperation> {
        self.shared.lock_active().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<Option<ActiveOperation>> {
        self.shared.tx.subscribe()
    }

    pub fn try_begin(
        &self,
        origin: OperationOrigin,
        label: impl Into<String>,
    ) -> Result<OperationLease, OperationBusy> {
        let label = label.into();
        let operation = {
            let mut active = self.shared.lock_active();
            if let Some(operation) = active.as_ref() {
                return Err(OperationBusy {
                    active: operation.clone(),
                });
            }

            let operation = ActiveOperation {
                id: self.shared.next_id.fetch_add(1, Ordering::Relaxed),
                origin,
                label,
            };
            *active = Some(operation.clone());
            operation
        };
        self.shared.tx.send_replace(Some(operation.clone()));

        Ok(OperationLease {
            coordinator: self.clone(),
            operation,
        })
    }
}

#[derive(Debug)]
pub struct OperationLease {
    coordinator: OperationCoordinator,
    operation: ActiveOperation,
}

impl OperationLease {
    pub fn operation(&self) -> &ActiveOperation {
        &self.operation
    }
}

impl Drop for OperationLease {
    fn drop(&mut self) {
        let released = {
            let mut active = self.coordinator.shared.lock_active();
            if active.as_ref().map(|operation| operation.id) == Some(self.operation.id) {
                *active = None;
                true
            } else {
                false
            }
        };

        if released {
            let operation_id = self.operation.id;
            self.coordinator.shared.tx.send_if_modified(|active| {
                if active.as_ref().map(|operation| operation.id) == Some(operation_id) {
                    *active = None;
                    true
                } else {
                    // A replacement published first is preserved; if it publishes later,
                    // its Some value naturally follows this lease's None notification.
                    false
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use super::*;

    struct BlockingLabel {
        conversion_started: mpsc::Sender<()>,
        continue_conversion: mpsc::Receiver<()>,
    }

    impl From<BlockingLabel> for String {
        fn from(value: BlockingLabel) -> Self {
            value.conversion_started.send(()).unwrap();
            value.continue_conversion.recv().unwrap();
            "blocked-operation".to_owned()
        }
    }

    #[test]
    fn only_one_write_operation_can_hold_the_lease() {
        let coordinator = OperationCoordinator::default();
        let first = coordinator
            .try_begin(OperationOrigin::Tui, "reward-mint")
            .unwrap();

        let error = coordinator
            .try_begin(OperationOrigin::Web, "transfer")
            .unwrap_err();

        assert_eq!(error.active.id, first.operation().id);
        assert_eq!(error.active.label, "reward-mint");
        assert_eq!(error.active.origin, OperationOrigin::Tui);
    }

    #[test]
    fn dropping_the_lease_releases_the_writer_and_notifies_observers() {
        let coordinator = OperationCoordinator::default();
        let mut changes = coordinator.subscribe();
        let lease = coordinator
            .try_begin(OperationOrigin::Web, "pause-prover")
            .unwrap();
        let first_id = lease.operation().id;

        assert!(changes.has_changed().unwrap());
        {
            let active = changes.borrow_and_update();
            assert_eq!(active.as_ref().unwrap().label, "pause-prover");
        }
        assert!(!changes.has_changed().unwrap());

        drop(lease);

        assert!(changes.has_changed().unwrap());
        assert!(changes.borrow_and_update().is_none());
        assert!(coordinator.active().is_none());

        let next = coordinator
            .try_begin(OperationOrigin::Tui, "reward-mint")
            .unwrap();
        assert_ne!(next.operation().id, first_id);
    }

    #[test]
    fn observer_does_not_block_readers_while_a_write_is_published() {
        let coordinator = OperationCoordinator::default();
        let changes = coordinator.subscribe();
        let held_value = changes.borrow();
        let (conversion_started_tx, conversion_started_rx) = mpsc::channel();
        let (continue_conversion_tx, continue_conversion_rx) = mpsc::channel();

        let begin_coordinator = coordinator.clone();
        let begin = thread::spawn(move || {
            begin_coordinator
                .try_begin(
                    OperationOrigin::Web,
                    BlockingLabel {
                        conversion_started: conversion_started_tx,
                        continue_conversion: continue_conversion_rx,
                    },
                )
                .unwrap()
        });

        conversion_started_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        continue_conversion_tx.send(()).unwrap();

        let reader_coordinator = coordinator.clone();
        let (reader_finished_tx, reader_finished_rx) = mpsc::channel();
        let reader = thread::spawn(move || {
            let active = reader_coordinator.active();
            reader_finished_tx.send(active).unwrap();
        });
        let reader_finished_timely = reader_finished_rx
            .recv_timeout(Duration::from_secs(1))
            .is_ok();

        drop(held_value);
        let lease = begin.join().unwrap();
        reader.join().unwrap();
        drop(lease);

        assert!(
            reader_finished_timely,
            "active() was blocked while an observer held the watch value"
        );
    }

    #[test]
    fn poisoned_writer_state_is_recovered() {
        let coordinator = OperationCoordinator::default();
        let poison_coordinator = coordinator.clone();

        assert!(thread::spawn(move || {
            let _active = poison_coordinator.shared.active.lock().unwrap();
            panic!("poison active operation state");
        })
        .join()
        .is_err());

        assert!(coordinator.active().is_none());
        let lease = coordinator
            .try_begin(OperationOrigin::Tui, "reward-mint")
            .unwrap();
        assert_eq!(coordinator.active().unwrap().id, lease.operation().id);

        drop(lease);

        assert!(coordinator.active().is_none());
    }
}
