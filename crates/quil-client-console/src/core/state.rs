use tokio::sync::watch;

use crate::{config::ConfigEntry, domain::NodeSnapshot};

#[derive(Debug, Clone, Default)]
pub struct CoreStateSnapshot {
    pub config_generation: u64,
    pub refresh_generation: u64,
    pub snapshot_revision: u64,
    pub active_config: Option<ConfigEntry>,
    pub node_snapshot: Option<NodeSnapshot>,
}

#[derive(Debug, Clone)]
pub(super) struct RefreshTicket {
    pub(super) active_config: ConfigEntry,
    pub(super) config_generation: u64,
    pub(super) refresh_generation: u64,
}

#[derive(Debug, Clone)]
pub struct CoreStateStore {
    tx: watch::Sender<CoreStateSnapshot>,
}

impl CoreStateStore {
    pub fn new(active_config: Option<ConfigEntry>) -> Self {
        let initial = CoreStateSnapshot {
            active_config,
            ..CoreStateSnapshot::default()
        };
        let (tx, _rx) = watch::channel(initial);
        Self { tx }
    }

    pub fn snapshot(&self) -> CoreStateSnapshot {
        self.tx.borrow().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<CoreStateSnapshot> {
        self.tx.subscribe()
    }

    pub(super) fn begin_refresh(&self) -> Option<RefreshTicket> {
        let mut ticket = None;
        self.tx.send_if_modified(|state| {
            let Some(active_config) = state.active_config.clone() else {
                return false;
            };

            state.refresh_generation = state.refresh_generation.wrapping_add(1);
            ticket = Some(RefreshTicket {
                active_config,
                config_generation: state.config_generation,
                refresh_generation: state.refresh_generation,
            });
            true
        });
        ticket
    }

    pub(super) fn is_refresh_current(
        &self,
        expected_config_generation: u64,
        expected_refresh_generation: u64,
    ) -> bool {
        let state = self.tx.borrow();
        state.config_generation == expected_config_generation
            && state.refresh_generation == expected_refresh_generation
    }

    pub fn replace_config(&self, active_config: ConfigEntry) {
        self.tx.send_modify(|state| {
            state.config_generation = state.config_generation.wrapping_add(1);
            state.active_config = Some(active_config);
            state.node_snapshot = None;
        });
    }

    pub(super) fn replace_config_with_snapshot(
        &self,
        active_config: ConfigEntry,
        node_snapshot: NodeSnapshot,
    ) {
        self.tx.send_modify(|state| {
            state.config_generation = state.config_generation.wrapping_add(1);
            state.snapshot_revision = state.snapshot_revision.wrapping_add(1);
            state.active_config = Some(active_config);
            state.node_snapshot = Some(node_snapshot);
        });
    }

    /// Returns `true` only when the generation matches and the snapshot is published.
    pub fn replace_node_snapshot(
        &self,
        expected_generation: u64,
        node_snapshot: NodeSnapshot,
    ) -> bool {
        self.tx.send_if_modified(|state| {
            if state.config_generation != expected_generation {
                return false;
            }

            state.snapshot_revision = state.snapshot_revision.wrapping_add(1);
            state.node_snapshot = Some(node_snapshot);
            true
        })
    }

    pub(super) fn replace_refreshed_node_snapshot(
        &self,
        expected_config_generation: u64,
        expected_refresh_generation: u64,
        node_snapshot: NodeSnapshot,
    ) -> bool {
        self.tx.send_if_modified(|state| {
            if state.config_generation != expected_config_generation
                || state.refresh_generation != expected_refresh_generation
            {
                return false;
            }

            state.snapshot_revision = state.snapshot_revision.wrapping_add(1);
            state.node_snapshot = Some(node_snapshot);
            true
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn config(name: &str) -> ConfigEntry {
        ConfigEntry {
            name: name.to_owned(),
            path: PathBuf::from(format!("/nodes/{name}/.config")),
            is_default: true,
        }
    }

    #[test]
    fn replacing_config_publishes_one_generation() {
        let store = CoreStateStore::new(None);
        let mut changes = store.subscribe();
        assert!(!changes.has_changed().unwrap());

        store.replace_config(config("mainnet"));
        assert!(changes.has_changed().unwrap());

        let snapshot = changes.borrow_and_update().clone();
        assert_eq!(snapshot.config_generation, 1);
        assert_eq!(snapshot.active_config.unwrap().name, "mainnet");
        assert!(!changes.has_changed().unwrap());
    }

    #[test]
    fn replacing_config_with_snapshot_publishes_consistent_state_atomically() {
        let store = CoreStateStore::new(Some(config("old")));
        let mut changes = store.subscribe();

        store.replace_config_with_snapshot(
            config("new"),
            NodeSnapshot {
                config_name: "new".into(),
                last_received_frame: 2,
                ..Default::default()
            },
        );

        assert!(changes.has_changed().unwrap());
        let snapshot = changes.borrow_and_update().clone();
        assert_eq!(snapshot.config_generation, 1);
        assert_eq!(snapshot.active_config.unwrap().name, "new");
        assert_eq!(snapshot.node_snapshot.unwrap().config_name, "new");
        assert!(!changes.has_changed().unwrap());
    }

    #[test]
    fn snapshot_revision_advances_only_when_a_node_snapshot_is_published() {
        let store = CoreStateStore::new(Some(config("old")));
        let ticket = store.begin_refresh().unwrap();
        let snapshot = NodeSnapshot {
            config_name: "old".into(),
            connected: true,
            ..Default::default()
        };

        assert_eq!(store.snapshot().snapshot_revision, 0);
        assert!(!store.replace_refreshed_node_snapshot(
            ticket.config_generation,
            ticket.refresh_generation.wrapping_add(1),
            snapshot.clone(),
        ));
        assert_eq!(store.snapshot().snapshot_revision, 0);

        assert!(store.replace_refreshed_node_snapshot(
            ticket.config_generation,
            ticket.refresh_generation,
            snapshot.clone(),
        ));
        assert_eq!(store.snapshot().snapshot_revision, 1);

        assert!(store.replace_node_snapshot(ticket.config_generation, snapshot.clone()));
        assert_eq!(store.snapshot().snapshot_revision, 2);

        store.replace_config(config("new"));
        assert_eq!(store.snapshot().snapshot_revision, 2);

        store.replace_config_with_snapshot(config("newer"), snapshot);
        assert_eq!(store.snapshot().snapshot_revision, 3);
    }

    #[tokio::test]
    async fn node_snapshot_is_cleared_when_config_changes() {
        let store = CoreStateStore::new(Some(config("old")));
        let generation = store.snapshot().config_generation;
        assert_eq!(generation, 0);
        assert!(store.replace_node_snapshot(
            generation,
            NodeSnapshot {
                config_name: "old".into(),
                connected: true,
                ..Default::default()
            }
        ));

        store.replace_config(config("new"));

        assert!(store.snapshot().node_snapshot.is_none());
        assert_eq!(store.snapshot().active_config.unwrap().name, "new");
    }

    #[test]
    fn stale_node_snapshot_is_rejected_after_config_changes() {
        let store = CoreStateStore::new(Some(config("old")));
        let stale_generation = store.snapshot().config_generation;

        store.replace_config(config("new"));

        assert!(!store.replace_node_snapshot(
            stale_generation,
            NodeSnapshot {
                config_name: "old".into(),
                connected: true,
                ..Default::default()
            }
        ));
        let snapshot = store.snapshot();
        assert!(snapshot.node_snapshot.is_none());
        assert_eq!(snapshot.active_config.unwrap().name, "new");
    }

    #[test]
    fn newer_refresh_ticket_rejects_an_older_snapshot_for_the_same_config() {
        let store = CoreStateStore::new(Some(config("mainnet")));
        let older = store.begin_refresh().unwrap();
        let newer = store.begin_refresh().unwrap();

        assert!(!store.is_refresh_current(older.config_generation, older.refresh_generation));
        assert!(store.is_refresh_current(newer.config_generation, newer.refresh_generation));
        assert!(store.replace_refreshed_node_snapshot(
            newer.config_generation,
            newer.refresh_generation,
            NodeSnapshot {
                config_name: "mainnet".into(),
                last_received_frame: 2,
                ..Default::default()
            }
        ));
        assert!(!store.replace_refreshed_node_snapshot(
            older.config_generation,
            older.refresh_generation,
            NodeSnapshot {
                config_name: "mainnet".into(),
                last_received_frame: 1,
                ..Default::default()
            }
        ));
        assert_eq!(
            store.snapshot().node_snapshot.unwrap().last_received_frame,
            2
        );
    }

    #[test]
    fn refresh_does_not_begin_without_an_active_config() {
        let store = CoreStateStore::new(None);

        assert!(store.begin_refresh().is_none());
        assert_eq!(store.snapshot().refresh_generation, 0);
    }
}
