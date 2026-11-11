use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, RwLock,
    },
};

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

const MAX_TERMINAL_TASKS: usize = 100;
pub const MAX_CONCURRENT_CAPABILITY_TASKS: usize = 4;
pub(super) const MAX_TASK_OUTPUT_BYTES: usize = 256 * 1024;
pub(super) const OUTPUT_TRUNCATED_MARKER: &str = "\n...[output truncated]...\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityTaskState {
    Running,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityTask {
    pub id: u64,
    pub capability_id: String,
    pub state: CapabilityTaskState,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CapabilityTaskFinishError {
    #[error("Capability task {0} does not exist")]
    NotFound(u64),
    #[error("Capability task {0} is already finished")]
    AlreadyFinished(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CapabilityTaskStartError {
    #[error("At most {0} capability tasks may run at once")]
    AtCapacity(usize),
}

#[derive(Debug)]
struct CapabilityTaskStoreInner {
    next_id: AtomicU64,
    tasks: RwLock<BTreeMap<u64, CapabilityTask>>,
    redactions: Mutex<BTreeMap<u64, Vec<Zeroizing<String>>>>,
    completion_order: Mutex<VecDeque<u64>>,
}

#[derive(Debug, Clone)]
pub struct CapabilityTaskStore {
    inner: Arc<CapabilityTaskStoreInner>,
}

impl Default for CapabilityTaskStore {
    fn default() -> Self {
        Self {
            inner: Arc::new(CapabilityTaskStoreInner {
                next_id: AtomicU64::new(1),
                tasks: RwLock::new(BTreeMap::new()),
                redactions: Mutex::new(BTreeMap::new()),
                completion_order: Mutex::new(VecDeque::new()),
            }),
        }
    }
}

impl CapabilityTaskStore {
    pub fn try_start(
        &self,
        capability_id: impl Into<String>,
    ) -> Result<CapabilityTask, CapabilityTaskStartError> {
        self.try_start_with_redactions(capability_id, Vec::new())
    }

    pub fn try_start_with_redactions(
        &self,
        capability_id: impl Into<String>,
        redactions: Vec<String>,
    ) -> Result<CapabilityTask, CapabilityTaskStartError> {
        let mut tasks = self.write_tasks();
        if tasks
            .values()
            .filter(|task| task.state == CapabilityTaskState::Running)
            .count()
            >= MAX_CONCURRENT_CAPABILITY_TASKS
        {
            return Err(CapabilityTaskStartError::AtCapacity(
                MAX_CONCURRENT_CAPABILITY_TASKS,
            ));
        }
        let task = CapabilityTask {
            id: self.inner.next_id.fetch_add(1, Ordering::Relaxed),
            capability_id: capability_id.into(),
            state: CapabilityTaskState::Running,
            stdout: String::new(),
            stderr: String::new(),
            exit_code: None,
        };
        let redactions = normalize_redactions(redactions);
        if !redactions.is_empty() {
            self.write_redactions().insert(task.id, redactions);
        }
        tasks.insert(task.id, task.clone());
        Ok(task)
    }

    pub fn finish(
        &self,
        id: u64,
        exit_code: Option<i32>,
        stdout: impl Into<String>,
        stderr: impl Into<String>,
    ) -> Result<CapabilityTask, CapabilityTaskFinishError> {
        let mut tasks = self.write_tasks();
        let completed = {
            let task = tasks
                .get_mut(&id)
                .ok_or(CapabilityTaskFinishError::NotFound(id))?;
            if task.state != CapabilityTaskState::Running {
                return Err(CapabilityTaskFinishError::AlreadyFinished(id));
            }
            task.state = if exit_code == Some(0) {
                CapabilityTaskState::Succeeded
            } else {
                CapabilityTaskState::Failed
            };
            let redactions = self.write_redactions().remove(&id).unwrap_or_default();
            task.stdout = limit_output(redact(stdout.into(), &redactions), false);
            task.stderr = limit_output(redact(stderr.into(), &redactions), false);
            task.exit_code = exit_code;
            task.clone()
        };
        let mut completion_order = self
            .inner
            .completion_order
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        completion_order.push_back(id);
        prune_terminal_tasks(&mut tasks, &mut completion_order);
        Ok(completed)
    }

    pub fn get(&self, id: u64) -> Option<CapabilityTask> {
        self.read_tasks().get(&id).cloned()
    }

    pub fn list(&self) -> Vec<CapabilityTask> {
        self.read_tasks().values().cloned().collect()
    }

    fn read_tasks(&self) -> std::sync::RwLockReadGuard<'_, BTreeMap<u64, CapabilityTask>> {
        self.inner
            .tasks
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn write_tasks(&self) -> std::sync::RwLockWriteGuard<'_, BTreeMap<u64, CapabilityTask>> {
        self.inner
            .tasks
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn write_redactions(&self) -> std::sync::MutexGuard<'_, BTreeMap<u64, Vec<Zeroizing<String>>>> {
        self.inner
            .redactions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn normalize_redactions(mut values: Vec<String>) -> Vec<Zeroizing<String>> {
    values.retain(|value| !value.is_empty());
    values.sort_by_key(|value| std::cmp::Reverse(value.len()));
    values.dedup();
    values.into_iter().map(Zeroizing::new).collect()
}

fn redact(mut output: String, redactions: &[Zeroizing<String>]) -> String {
    let truncated = output.ends_with(OUTPUT_TRUNCATED_MARKER);
    if truncated {
        output.truncate(output.len() - OUTPUT_TRUNCATED_MARKER.len());
    }
    let truncated_suffix = truncated
        .then(|| truncated_secret_suffix_bytes(&output, redactions))
        .flatten();
    if let Some(suffix_bytes) = truncated_suffix {
        output.truncate(output.len().saturating_sub(suffix_bytes));
    }
    for secret in redactions {
        output = output.replace(secret.as_str(), "[redacted]");
    }
    if truncated {
        if truncated_suffix.is_some() {
            let maximum_before_redaction = MAX_TASK_OUTPUT_BYTES
                .saturating_sub(OUTPUT_TRUNCATED_MARKER.len())
                .saturating_sub("[redacted]".len());
            truncate_at_char_boundary(&mut output, maximum_before_redaction);
            output.push_str("[redacted]");
        } else {
            let maximum_payload =
                MAX_TASK_OUTPUT_BYTES.saturating_sub(OUTPUT_TRUNCATED_MARKER.len());
            truncate_at_char_boundary(&mut output, maximum_payload);
        }
        output.push_str(OUTPUT_TRUNCATED_MARKER);
    }
    output
}

fn truncated_secret_suffix_bytes(payload: &str, redactions: &[Zeroizing<String>]) -> Option<usize> {
    let (candidate, replacement_bytes) = payload
        .strip_suffix('\u{fffd}')
        .map_or((payload, 0), |candidate| (candidate, '\u{fffd}'.len_utf8()));
    let mut best = None;
    for secret in redactions {
        for boundary in secret
            .char_indices()
            .map(|(index, _)| index)
            .skip(1)
            .chain(std::iter::once(secret.len()))
        {
            if candidate.ends_with(&secret[..boundary])
                && best.is_none_or(|current| boundary > current)
            {
                best = Some(boundary);
            }
        }
    }
    best.map(|bytes| bytes + replacement_bytes)
}

fn truncate_at_char_boundary(value: &mut String, maximum_bytes: usize) {
    let mut boundary = value.len().min(maximum_bytes);
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value.truncate(boundary);
}

fn prune_terminal_tasks(
    tasks: &mut BTreeMap<u64, CapabilityTask>,
    completion_order: &mut VecDeque<u64>,
) {
    while completion_order.len() > MAX_TERMINAL_TASKS {
        if let Some(id) = completion_order.pop_front() {
            tasks.remove(&id);
        }
    }
}

pub(super) fn limit_output(output: String, truncated: bool) -> String {
    let marker_bytes = OUTPUT_TRUNCATED_MARKER.len();
    let needs_marker = truncated || output.len() > MAX_TASK_OUTPUT_BYTES;
    let maximum_payload = if needs_marker {
        MAX_TASK_OUTPUT_BYTES.saturating_sub(marker_bytes)
    } else {
        MAX_TASK_OUTPUT_BYTES
    };
    let mut boundary = output.len().min(maximum_payload);
    while !output.is_char_boundary(boundary) {
        boundary -= 1;
    }

    let final_length = boundary + if needs_marker { marker_bytes } else { 0 };
    let mut limited = String::with_capacity(final_length);
    limited.push_str(&output[..boundary]);
    if needs_marker {
        limited.push_str(OUTPUT_TRUNCATED_MARKER);
    }
    limited.into_boxed_str().into_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_output_lengths_and_allocations_are_bounded() {
        let store = CapabilityTaskStore::default();
        let task = store.try_start("config.print").unwrap();
        let mut short_overallocated = String::with_capacity(4 * 1024 * 1024);
        short_overallocated.push_str("short output");
        let oversized = "x".repeat(300 * 1024);

        store
            .finish(task.id, Some(1), short_overallocated, oversized)
            .unwrap();

        let tasks = store.read_tasks();
        let stored = tasks.get(&task.id).unwrap();
        assert_eq!(stored.stdout, "short output");
        assert!(stored.stdout.len() <= MAX_TASK_OUTPUT_BYTES);
        assert!(stored.stdout.capacity() <= MAX_TASK_OUTPUT_BYTES);
        assert!(stored.stderr.len() <= MAX_TASK_OUTPUT_BYTES);
        assert!(stored.stderr.capacity() <= MAX_TASK_OUTPUT_BYTES);
        assert!(stored.stderr.ends_with(OUTPUT_TRUNCATED_MARKER));
    }

    #[test]
    fn shared_store_redacts_each_task_before_results_are_observable() {
        let store = CapabilityTaskStore::default();
        let first = store
            .try_start_with_redactions("key.import", vec!["secret".into(), "secret-long".into()])
            .unwrap();
        let second = store
            .try_start_with_redactions("key.import", vec!["other-secret".into()])
            .unwrap();

        store
            .finish(
                first.id,
                Some(1),
                "stdout secret-long secret",
                "stderr secret-long",
            )
            .unwrap();
        store
            .finish(second.id, Some(0), "other-secret", "secret-long")
            .unwrap();

        let first = store.get(first.id).unwrap();
        assert_eq!(first.stdout, "stdout [redacted] [redacted]");
        assert_eq!(first.stderr, "stderr [redacted]");
        let second = store.get(second.id).unwrap();
        assert_eq!(second.stdout, "[redacted]");
        assert_eq!(second.stderr, "secret-long");
    }

    #[test]
    fn truncated_output_never_retains_a_secret_prefix_at_the_capture_boundary() {
        let maximum_payload = MAX_TASK_OUTPUT_BYTES - OUTPUT_TRUNCATED_MARKER.len();
        for secret in ["qv-private-material", "secret-material"] {
            for prefix_bytes in secret
                .char_indices()
                .map(|(index, _)| index)
                .skip(1)
                .chain(std::iter::once(secret.len()))
            {
                let prefix = &secret[..prefix_bytes];
                let mut raw = "x".repeat(maximum_payload - prefix.len());
                raw.push_str(secret);
                raw.push_str("trailing output");
                let captured = limit_output(raw, true);
                assert!(captured.ends_with(OUTPUT_TRUNCATED_MARKER));

                let store = CapabilityTaskStore::default();
                let task = store
                    .try_start_with_redactions("key.import", vec![secret.into()])
                    .unwrap();
                store.finish(task.id, Some(0), captured, "").unwrap();
                let stored = store.get(task.id).unwrap();

                assert!(!stored.stdout.contains(prefix));
                assert!(stored.stdout.ends_with(OUTPUT_TRUNCATED_MARKER));
                assert!(stored.stdout.contains("[redacted]"));
                assert!(stored.stdout.len() <= MAX_TASK_OUTPUT_BYTES);
                assert!(stored.stdout.capacity() <= MAX_TASK_OUTPUT_BYTES);
            }
        }
    }

    #[test]
    fn overlapping_secrets_do_not_leave_the_longer_secrets_tail_at_the_boundary() {
        let maximum_payload = MAX_TASK_OUTPUT_BYTES - OUTPUT_TRUNCATED_MARKER.len();
        let mut raw = "x".repeat(maximum_payload - "abcde".len());
        raw.push_str("abcdef beyond the capture boundary");
        let captured = limit_output(raw, true);
        assert!(captured.ends_with("abcde\n...[output truncated]...\n"));

        let store = CapabilityTaskStore::default();
        let task = store
            .try_start_with_redactions("key.import", vec!["abc".into(), "abcdef".into()])
            .unwrap();
        store.finish(task.id, Some(0), captured, "").unwrap();
        let stored = store.get(task.id).unwrap();

        assert!(!stored.stdout.contains("abcde"));
        assert!(!stored.stdout.contains("de"));
        assert!(stored
            .stdout
            .ends_with("[redacted]\n...[output truncated]...\n"));
        assert!(stored.stdout.len() <= MAX_TASK_OUTPUT_BYTES);
        assert!(stored.stdout.capacity() <= MAX_TASK_OUTPUT_BYTES);

        let ordinary = store
            .try_start_with_redactions("key.import", vec!["abcdef".into()])
            .unwrap();
        store.finish(ordinary.id, Some(0), "abcde", "").unwrap();
        assert_eq!(store.get(ordinary.id).unwrap().stdout, "abcde");
    }
}
