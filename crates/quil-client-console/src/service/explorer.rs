use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use num_bigint::BigInt;
use serde::Deserialize;
use tokio::sync::RwLock;

use super::ServiceError;

const DEFAULT_BASE_URL: &str = "https://explorer-api.quilibrium.com";
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
const MIN_FILTER_BYTES: usize = 32;
const MAX_FILTER_BYTES: usize = 66;

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub(super) struct ExplorerCounts {
    pub active: u32,
    pub joining: u32,
    pub paused: u32,
    pub leaving: u32,
    pub rejected: u32,
    pub kicked: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ExplorerShard {
    pub filter_hex: String,
    pub counts: ExplorerCounts,
    pub size_bytes: BigInt,
    pub active_workers: u32,
    pub data_shards: u64,
}

impl ExplorerShard {
    pub fn estimated_join_ring(&self) -> u32 {
        self.counts.active.saturating_add(self.counts.joining) / 8
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ExplorerSnapshot {
    pub rows: Vec<ExplorerShard>,
    pub collected_at_unix_seconds: u64,
    pub frame_number: Option<u64>,
    pub network_provers: Option<u32>,
    pub invalid_rows: usize,
    pub stale: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct ExplorerDirectory {
    client: reqwest::Client,
    base_url: String,
    cache: Arc<RwLock<Option<ExplorerSnapshot>>>,
}

impl Default for ExplorerDirectory {
    fn default() -> Self {
        Self::new(DEFAULT_BASE_URL)
    }
}

impl ExplorerDirectory {
    fn new(base_url: &str) -> Self {
        let client = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .build()
            .expect("static Explorer HTTP client configuration");
        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_owned(),
            cache: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn snapshot(&self) -> Result<ExplorerSnapshot, ServiceError> {
        match self.fetch().await {
            Ok(snapshot) => {
                *self.cache.write().await = Some(snapshot.clone());
                Ok(snapshot)
            }
            Err(error) => {
                let cached = self.cache.read().await.clone();
                cached
                    .map(|mut snapshot| {
                        snapshot.stale = true;
                        snapshot.error = Some(error.clone());
                        snapshot
                    })
                    .ok_or(ServiceError::Shards(error))
            }
        }
    }

    async fn fetch(&self) -> Result<ExplorerSnapshot, String> {
        let shards_url = format!("{}/provers/shards", self.base_url);
        let stats_url = format!("{}/stats", self.base_url);
        let (shards, stats) = tokio::join!(
            fetch_bytes(&self.client, &shards_url),
            fetch_bytes(&self.client, &stats_url)
        );
        let parsed = parse_shards(&shards?)?;
        let (stats, stats_error) = match stats {
            Ok(bytes) => match serde_json::from_slice::<ExplorerStats>(&bytes) {
                Ok(stats) => (Some(stats), None),
                Err(error) => (None, Some(format!("invalid Explorer stats: {error}"))),
            },
            Err(error) => (None, Some(error)),
        };
        Ok(ExplorerSnapshot {
            rows: parsed.rows,
            collected_at_unix_seconds: unix_seconds(),
            frame_number: stats.as_ref().and_then(|value| value.current_frame),
            network_provers: stats.and_then(|value| value.total_provers),
            invalid_rows: parsed.invalid_rows,
            stale: false,
            error: stats_error,
        })
    }
}

async fn fetch_bytes(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| format!("Explorer request failed: {error}"))?
        .error_for_status()
        .map_err(|error| format!("Explorer returned an error: {error}"))?;
    response
        .bytes()
        .await
        .map(|bytes| bytes.to_vec())
        .map_err(|error| format!("Explorer response read failed: {error}"))
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawExplorerShard {
    filter: String,
    counts: ExplorerCounts,
    size_bytes: String,
    active_workers: u32,
    data_shards: u64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ExplorerStats {
    current_frame: Option<u64>,
    total_provers: Option<u32>,
}

struct ParsedDirectory {
    rows: Vec<ExplorerShard>,
    invalid_rows: usize,
}

fn parse_shards(body: &[u8]) -> Result<ParsedDirectory, String> {
    let raw = serde_json::from_slice::<Vec<RawExplorerShard>>(body)
        .map_err(|error| format!("invalid Explorer shard directory: {error}"))?;
    let total = raw.len();
    let rows = raw
        .into_iter()
        .filter_map(normalize_shard)
        .collect::<Vec<_>>();
    if rows.is_empty() {
        return Err("Explorer shard directory contained no valid rows".into());
    }
    Ok(ParsedDirectory {
        invalid_rows: total - rows.len(),
        rows,
    })
}

fn normalize_shard(raw: RawExplorerShard) -> Option<ExplorerShard> {
    let filter_hex = raw.filter.trim().strip_prefix("0x").unwrap_or(&raw.filter);
    if !filter_hex.len().is_multiple_of(2) {
        return None;
    }
    let filter_hex = filter_hex.to_ascii_lowercase();
    let filter = hex::decode(&filter_hex).ok()?;
    if !(MIN_FILTER_BYTES..=MAX_FILTER_BYTES).contains(&filter.len()) {
        return None;
    }
    let size_bytes = raw.size_bytes.parse::<BigInt>().ok()?;
    if size_bytes.sign() == num_bigint::Sign::Minus {
        return None;
    }
    Some(ExplorerShard {
        filter_hex,
        counts: raw.counts,
        size_bytes,
        active_workers: raw.active_workers,
        data_shards: raw.data_shards,
    })
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_normalizes_official_shards() {
        let filter = format!("{}0530", "11".repeat(32));
        let body = format!(
            r#"[{{"filter":"0x{filter}","counts":{{"active":23,"joining":1,"leaving":4}},"size_bytes":"1672873632","active_workers":24,"data_shards":2519388}}]"#
        );

        let parsed = parse_shards(body.as_bytes()).expect("valid directory");

        assert_eq!(parsed.rows[0].filter_hex, filter);
        assert_eq!(parsed.rows[0].counts.active, 23);
        assert_eq!(parsed.rows[0].estimated_join_ring(), 3);
        assert_eq!(parsed.invalid_rows, 0);
    }

    #[test]
    fn rejects_empty_directory_and_drops_invalid_rows() {
        assert!(parse_shards(br#"[]"#).is_err());
        let valid = format!("{}0530", "aa".repeat(32));
        let body = format!(
            r#"[{{"filter":"bad","counts":{{}},"size_bytes":"1","active_workers":0,"data_shards":1}},{{"filter":"{valid}","counts":{{}},"size_bytes":"2","active_workers":0,"data_shards":2}}]"#
        );

        let parsed = parse_shards(body.as_bytes()).expect("one valid row");

        assert_eq!(parsed.rows.len(), 1);
        assert_eq!(parsed.invalid_rows, 1);
    }
}
