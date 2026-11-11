use std::{cmp::Ordering, collections::BTreeMap, path::Path};

use num_bigint::{BigInt, Sign};
use quil_types::proto::node::{
    FrameCadence, FrameWindow, GetShardInfoRequest, GetShardInfoResponse, ShardProverCounts,
    ShardRewardInfo,
};

use super::{explorer::ExplorerDirectory, node, ServiceError};
use crate::domain::{
    SeatConcentrationSnapshot, ShardInfoSnapshot, ShardProverCountsSnapshot, ShardSnapshot,
    ShardSummarySnapshot,
};

const FRAMES_PER_DAY: u64 = 8_640;

pub(super) async fn read(
    config_dir: &Path,
    include_all: bool,
    explorer: &ExplorerDirectory,
) -> Result<ShardInfoSnapshot, ServiceError> {
    if !include_all {
        return read_local(config_dir, false).await;
    }
    let (local, directory) = tokio::join!(read_response(config_dir, true), explorer.snapshot());
    match (local, directory) {
        (Ok(response), Ok(directory)) => Ok(merge_snapshot(response, directory)),
        (Ok(response), Err(error)) => {
            let mut snapshot = map_snapshot(true, response);
            snapshot.directory_error = Some(error.to_string());
            Ok(snapshot)
        }
        (Err(error), Ok(directory)) => {
            let mut snapshot = merge_snapshot(GetShardInfoResponse::default(), directory);
            snapshot.directory_error = Some(format!("Local node shard data unavailable: {error}"));
            Ok(snapshot)
        }
        (Err(local), Err(directory)) => Err(ServiceError::Shards(format!(
            "local node: {local}; official directory: {directory}"
        ))),
    }
}

async fn read_local(
    config_dir: &Path,
    include_all: bool,
) -> Result<ShardInfoSnapshot, ServiceError> {
    let response = read_response(config_dir, include_all).await?;
    Ok(map_snapshot(include_all, response))
}

async fn read_response(
    config_dir: &Path,
    include_all: bool,
) -> Result<GetShardInfoResponse, ServiceError> {
    let config = quil_config::load_config(config_dir)
        .map_err(|error| ServiceError::Config(error.to_string()))?;
    let rpc_address = node::grpc_address(config.effective_listen_grpc_multiaddr())?;
    let mut client = node::connect_client(&rpc_address).await?;
    tokio::time::timeout(
        node::RPC_TIMEOUT,
        client.get_shard_info(tonic::Request::new(GetShardInfoRequest { include_all })),
    )
    .await
    .map_err(|_| ServiceError::Timeout)?
    .map_err(|error| ServiceError::Shards(error.to_string()))
    .map(|response| response.into_inner())
}

fn map_snapshot(include_all: bool, mut response: GetShardInfoResponse) -> ShardInfoSnapshot {
    response.shards.sort_by(shard_order);
    let summary = summarize(&response.shards);
    ShardInfoSnapshot {
        include_all,
        frame_number: response.frame_number,
        difficulty: response.difficulty,
        world_state: format_big_storage(&response.world_state_bytes),
        pomw_basis: format_big_integer(&response.pomw_basis),
        allocation_scoring_strategy: response.allocation_scoring_strategy,
        directory_collected_at_unix_seconds: None,
        directory_stale: false,
        directory_error: None,
        directory_invalid_rows: 0,
        network_provers: None,
        summary,
        shards: response.shards.into_iter().map(map_shard).collect(),
    }
}

fn summarize(shards: &[ShardRewardInfo]) -> ShardSummarySnapshot {
    let mut summary = ShardSummarySnapshot {
        total: shards.len(),
        allocated: shards.iter().filter(|shard| shard.is_allocated).count(),
        ring_unknown: shards
            .iter()
            .filter(|shard| shard.ring_known == Some(false))
            .count(),
        active_provers: shards
            .iter()
            .map(|shard| u64::from(shard.active_provers))
            .sum(),
        data_shards: shards.iter().map(|shard| shard.data_shards).sum(),
        seat_concentration: seat_concentration(shards),
        ..Default::default()
    };
    let mut theoretical = BigInt::from(0u8);
    let mut hour = BigInt::from(0u8);
    let mut day = BigInt::from(0u8);
    for shard in shards {
        if shard.ring_known == Some(false) {
            continue;
        }
        let reward = reward_value(&shard.estimated_reward);
        theoretical += &reward;
        summary.theoretical_coverage += 1;
        if let Some(value) = observed_window_value(&reward, shard.frame_cadence.as_ref(), false) {
            hour += value;
            summary.hour_coverage += 1;
        }
        if let Some(value) = observed_window_value(&reward, shard.frame_cadence.as_ref(), true) {
            day += value;
            summary.day_coverage += 1;
        }
    }
    summary.theoretical_daily_quil =
        (summary.theoretical_coverage > 0).then(|| format_quil_daily(&theoretical));
    summary.hour_daily_quil = (summary.hour_coverage > 0).then(|| format_quil_units(&hour));
    summary.day_daily_quil = (summary.day_coverage > 0).then(|| format_quil_units(&day));
    summary
}

fn shard_order(left: &ShardRewardInfo, right: &ShardRewardInfo) -> Ordering {
    let left_known = left.ring_known != Some(false);
    let right_known = right.ring_known != Some(false);
    right_known
        .cmp(&left_known)
        .then_with(|| {
            reward_value(&right.estimated_reward).cmp(&reward_value(&left.estimated_reward))
        })
        .then_with(|| left.filter.cmp(&right.filter))
}

fn map_shard(shard: ShardRewardInfo) -> ShardSnapshot {
    let ring_known = shard.ring_known != Some(false);
    let reward = reward_value(&shard.estimated_reward);
    let cadence = shard.frame_cadence.as_ref();
    ShardSnapshot {
        filter_hex: hex::encode(shard.filter),
        active_provers: shard.active_provers,
        ring: ring_known.then_some(shard.ring),
        ring_estimated: false,
        official: false,
        shard_size: format_big_storage(&shard.shard_size),
        data_shards: shard.data_shards,
        is_allocated: shard.is_allocated,
        materialized_frame: shard.materialized_frame,
        latest_frame: shard.latest_frame,
        allocation_reason: shard.allocation_reason.filter(|reason| !reason.is_empty()),
        theoretical_daily_quil: ring_known.then(|| format_quil_daily(&reward)),
        hour_daily_quil: ring_known
            .then(|| cadence.and_then(|value| observed_daily(reward.clone(), value, false)))
            .flatten(),
        day_daily_quil: ring_known
            .then(|| cadence.and_then(|value| observed_daily(reward, value, true)))
            .flatten(),
        prover_counts: shard.prover_counts.map(map_prover_counts),
    }
}

fn merge_snapshot(
    response: GetShardInfoResponse,
    directory: super::explorer::ExplorerSnapshot,
) -> ShardInfoSnapshot {
    let basis = reward_value(&response.pomw_basis);
    let world_size = reward_value(&response.world_state_bytes);
    let rewards = response
        .shards
        .iter()
        .map(|shard| {
            let reward =
                (shard.ring_known != Some(false)).then(|| reward_value(&shard.estimated_reward));
            (hex::encode(&shard.filter), reward)
        })
        .collect::<BTreeMap<_, _>>();
    let mut snapshot = map_snapshot(true, response);
    let mut rows = snapshot
        .shards
        .drain(..)
        .map(|row| {
            let reward = rewards.get(&row.filter_hex).cloned().flatten();
            (row.filter_hex.clone(), (row, reward))
        })
        .collect::<BTreeMap<_, _>>();

    for official in &directory.rows {
        let counts = map_explorer_counts(&official.counts);
        if let Some((row, _)) = rows.get_mut(&official.filter_hex) {
            row.active_provers = official.counts.active;
            row.shard_size = format_big_storage_value(&official.size_bytes);
            row.data_shards = official.data_shards;
            row.prover_counts = Some(counts);
            row.official = true;
            continue;
        }

        let ring = official.estimated_join_ring();
        let reward = estimated_reward(
            &basis,
            &official.size_bytes,
            &world_size,
            ring,
            official.data_shards,
        );
        let row = ShardSnapshot {
            filter_hex: official.filter_hex.clone(),
            active_provers: official.counts.active,
            ring: Some(ring),
            ring_estimated: true,
            official: true,
            shard_size: format_big_storage_value(&official.size_bytes),
            data_shards: official.data_shards,
            is_allocated: false,
            materialized_frame: 0,
            latest_frame: directory.frame_number.unwrap_or_default(),
            allocation_reason: None,
            theoretical_daily_quil: reward.as_ref().map(format_quil_daily),
            hour_daily_quil: None,
            day_daily_quil: None,
            prover_counts: Some(counts),
        };
        rows.insert(official.filter_hex.clone(), (row, reward));
    }

    let mut rows = rows.into_values().collect::<Vec<_>>();
    rows.sort_by(|(left, left_reward), (right, right_reward)| {
        right_reward
            .is_some()
            .cmp(&left_reward.is_some())
            .then_with(|| right_reward.cmp(left_reward))
            .then_with(|| left.filter_hex.cmp(&right.filter_hex))
    });
    let theoretical = rows
        .iter()
        .filter_map(|(_, reward)| reward.as_ref())
        .fold(BigInt::from(0u8), |total, reward| total + reward);
    snapshot.shards = rows.into_iter().map(|(row, _)| row).collect();
    snapshot.summary.total = snapshot.shards.len();
    snapshot.summary.allocated = snapshot
        .shards
        .iter()
        .filter(|row| row.is_allocated)
        .count();
    snapshot.summary.ring_unknown = snapshot
        .shards
        .iter()
        .filter(|row| row.ring.is_none())
        .count();
    snapshot.summary.active_provers = snapshot
        .shards
        .iter()
        .map(|row| u64::from(row.active_provers))
        .sum();
    snapshot.summary.data_shards = snapshot.shards.iter().map(|row| row.data_shards).sum();
    snapshot.summary.theoretical_coverage = snapshot
        .shards
        .iter()
        .filter(|row| row.theoretical_daily_quil.is_some())
        .count();
    snapshot.summary.theoretical_daily_quil =
        (snapshot.summary.theoretical_coverage > 0).then(|| format_quil_daily(&theoretical));
    snapshot.directory_collected_at_unix_seconds = Some(directory.collected_at_unix_seconds);
    snapshot.directory_stale = directory.stale;
    snapshot.directory_error = directory.error;
    snapshot.directory_invalid_rows = directory.invalid_rows;
    snapshot.network_provers = directory.network_provers;
    snapshot
}

fn map_explorer_counts(counts: &super::explorer::ExplorerCounts) -> ShardProverCountsSnapshot {
    ShardProverCountsSnapshot {
        joining: counts.joining,
        activating: 0,
        active: counts.active,
        paused: counts.paused,
        leaving: counts.leaving,
        departing: 0,
    }
}

fn estimated_reward(
    basis: &BigInt,
    shard_size: &BigInt,
    world_size: &BigInt,
    ring: u32,
    data_shards: u64,
) -> Option<BigInt> {
    if basis <= &BigInt::from(0u8)
        || shard_size <= &BigInt::from(0u8)
        || world_size <= &BigInt::from(0u8)
        || data_shards == 0
    {
        return None;
    }
    Some(quil_execution::pricing::allocation_prover_reward(
        basis,
        shard_size,
        world_size,
        u8::try_from(ring).ok()?,
        data_shards,
    ))
}

fn format_big_storage_value(value: &BigInt) -> String {
    format_storage(u64::try_from(value).unwrap_or(u64::MAX))
}

fn map_prover_counts(counts: ShardProverCounts) -> ShardProverCountsSnapshot {
    ShardProverCountsSnapshot {
        joining: counts.joining,
        activating: counts.activating,
        active: counts.active,
        paused: counts.paused,
        leaving: counts.leaving,
        departing: counts.departing,
    }
}

fn reward_value(bytes: &[u8]) -> BigInt {
    BigInt::from_bytes_be(Sign::Plus, bytes)
}

fn format_quil_daily(per_frame: &BigInt) -> String {
    let raw = per_frame * BigInt::from(FRAMES_PER_DAY);
    if raw.sign() != Sign::Plus {
        return "0.0".into();
    }
    let tenths = (raw + BigInt::from(5_000_000u64)) / BigInt::from(10_000_000u64);
    if tenths.sign() != Sign::Plus {
        return "<0.1".into();
    }
    format!("{}.{}", &tenths / 10, &tenths % 10)
}

fn observed_daily(per_frame: BigInt, cadence: &FrameCadence, use_day: bool) -> Option<String> {
    observed_window_value(&per_frame, Some(cadence), use_day).map(|value| format_quil_units(&value))
}

fn observed_window_value(
    per_frame: &BigInt,
    cadence: Option<&FrameCadence>,
    use_day: bool,
) -> Option<BigInt> {
    let cadence = cadence?;
    let window = if use_day {
        cadence.day.as_ref()?
    } else {
        cadence.hour.as_ref()?
    };
    observed_value(per_frame.clone(), window)
}

fn seat_concentration(shards: &[ShardRewardInfo]) -> Option<SeatConcentrationSnapshot> {
    let reports = shards
        .iter()
        .filter_map(|shard| shard.seat_concentration.as_ref())
        .collect::<Vec<_>>();
    let at_risk = reports
        .iter()
        .filter(|report| {
            report.concentration_seats > 0 && report.own_group_seats >= report.concentration_seats
        })
        .count();
    (!reports.is_empty()).then_some(SeatConcentrationSnapshot {
        at_risk,
        reported: reports.len(),
    })
}

fn observed_value(per_frame: BigInt, window: &FrameWindow) -> Option<BigInt> {
    if window.observed_seconds < 60 || window.observed_seconds > window.window_seconds {
        return None;
    }
    Some(
        per_frame * BigInt::from(window.advanced_frames) * BigInt::from(86_400u64)
            / BigInt::from(window.observed_seconds),
    )
}

fn format_quil_units(value: &BigInt) -> String {
    let negative = value.sign() == Sign::Minus;
    let magnitude = if negative { -value } else { value.clone() };
    let tenths = (&magnitude + BigInt::from(5_000_000u64)) / BigInt::from(10_000_000u64);
    let text = if magnitude > BigInt::from(0) && tenths == BigInt::from(0) {
        "<0.1".into()
    } else {
        format!("{}.{}", &tenths / 10, &tenths % 10)
    };
    format!("~{}{text}", if negative { "-" } else { "" })
}

fn format_big_storage(bytes: &[u8]) -> String {
    let value = BigInt::from_bytes_be(Sign::Plus, bytes);
    let value = u64::try_from(value).unwrap_or(u64::MAX);
    format_storage(value)
}

fn format_big_integer(bytes: &[u8]) -> String {
    BigInt::from_bytes_be(Sign::Plus, bytes).to_string()
}

fn format_storage(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;
    let value = bytes as f64;
    if bytes >= TB {
        format!("{:.1} TB", value / TB as f64)
    } else if bytes >= GB {
        format!("{:.1} GB", value / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", value / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", value / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use crate::service::explorer::{ExplorerCounts, ExplorerShard, ExplorerSnapshot};
    use quil_types::proto::node::{
        FrameCadence, FrameWindow, GetShardInfoResponse, SeatConcentration,
    };

    use super::*;

    #[test]
    fn local_ring_and_reward_win_while_official_counts_win() {
        let filter = filter(0x11, &[0x05, 0x30]);
        let response = GetShardInfoResponse {
            shards: vec![ShardRewardInfo {
                filter: filter.clone(),
                active_provers: 7,
                ring: 1,
                ring_known: Some(true),
                shard_size: 1_024u64.to_be_bytes().to_vec(),
                estimated_reward: 100_000_000u64.to_be_bytes().to_vec(),
                data_shards: 8,
                ..Default::default()
            }],
            pomw_basis: 1_000u64.to_be_bytes().to_vec(),
            world_state_bytes: 100_000u64.to_be_bytes().to_vec(),
            ..Default::default()
        };
        let explorer = explorer_snapshot(filter.clone(), 23, 1, 4);

        let merged = merge_snapshot(response, explorer);
        let row = &merged.shards[0];

        assert_eq!(row.ring, Some(1));
        assert!(!row.ring_estimated);
        assert_eq!(row.prover_counts.as_ref().unwrap().active, 23);
        assert_eq!(row.theoretical_daily_quil.as_deref(), Some("8640.0"));
        assert!(row.official);
    }

    #[test]
    fn official_only_filter_gets_estimated_ring() {
        let filter = filter(0x22, &[0x05, 0x30]);
        let response = GetShardInfoResponse {
            pomw_basis: 1_000_000_000u64.to_be_bytes().to_vec(),
            world_state_bytes: 1_000_000u64.to_be_bytes().to_vec(),
            ..Default::default()
        };
        let explorer = explorer_snapshot(filter.clone(), 16, 0, 2);

        let merged = merge_snapshot(response, explorer);
        let row = &merged.shards[0];

        assert_eq!(row.filter_hex, hex::encode(filter));
        assert_eq!(row.ring, Some(2));
        assert!(row.ring_estimated);
        assert!(row.official);
    }

    #[test]
    fn maps_and_sorts_shards_by_estimated_reward() {
        let response = GetShardInfoResponse {
            shards: vec![
                shard(1, 4_498, Some(true)),
                shard(2, 67_900, Some(true)),
                shard(3, 999_999, Some(false)),
            ],
            difficulty: 42,
            frame_number: 900,
            pomw_basis: 123_456u64.to_be_bytes().to_vec(),
            allocation_scoring_strategy: "reward".into(),
            ..Default::default()
        };

        let snapshot = map_snapshot(false, response);

        assert!(!snapshot.include_all);
        assert_eq!(snapshot.frame_number, 900);
        assert_eq!(snapshot.pomw_basis, "123456");
        assert_eq!(snapshot.summary.total, 3);
        assert_eq!(snapshot.summary.allocated, 1);
        assert_eq!(snapshot.summary.ring_unknown, 1);
        assert_eq!(snapshot.summary.active_provers, 21);
        assert_eq!(snapshot.summary.data_shards, 24);
        assert_eq!(snapshot.summary.theoretical_coverage, 2);
        assert_eq!(
            snapshot.summary.theoretical_daily_quil.as_deref(),
            Some("6.3")
        );
        assert_eq!(snapshot.shards[0].filter_hex, "02");
        assert_eq!(
            snapshot.shards[0].theoretical_daily_quil.as_deref(),
            Some("5.9")
        );
        assert_eq!(
            snapshot.shards[1].theoretical_daily_quil.as_deref(),
            Some("0.4")
        );
        assert_eq!(snapshot.shards[2].ring, None);
        assert_eq!(snapshot.shards[2].theoretical_daily_quil, None);
    }

    #[test]
    fn observed_estimate_trusts_the_node_owned_window_across_clock_skew() {
        let old_timestamp = FrameCadence {
            last_observed_unix_seconds: 1_000,
            hour: Some(FrameWindow {
                window_seconds: 3_600,
                observed_seconds: 600,
                advanced_frames: 30,
            }),
            ..Default::default()
        };

        assert_eq!(
            observed_daily(100_000_000.into(), &old_timestamp, false),
            Some("~4320.0".into())
        );

        let future_timestamp = FrameCadence {
            last_observed_unix_seconds: u64::MAX,
            ..old_timestamp
        };
        assert_eq!(
            observed_daily(100_000_000.into(), &future_timestamp, false),
            Some("~4320.0".into())
        );
    }

    #[test]
    fn aggregate_estimates_report_partial_cadence_coverage() {
        let now = 1_000;
        let fresh = FrameCadence {
            last_observed_unix_seconds: now,
            hour: Some(FrameWindow {
                window_seconds: 3_600,
                observed_seconds: 600,
                advanced_frames: 30,
            }),
            day: Some(FrameWindow {
                window_seconds: 86_400,
                observed_seconds: 3_600,
                advanced_frames: 180,
            }),
        };
        let stale = FrameCadence {
            last_observed_unix_seconds: now - 91,
            ..fresh
        };
        let response = GetShardInfoResponse {
            shards: vec![
                ShardRewardInfo {
                    frame_cadence: Some(fresh),
                    ..shard(1, 100_000_000, Some(true))
                },
                ShardRewardInfo {
                    frame_cadence: Some(stale),
                    ..shard(2, 100_000_000, Some(true))
                },
            ],
            ..Default::default()
        };

        let snapshot = map_snapshot(true, response);

        assert_eq!(snapshot.summary.hour_coverage, 2);
        assert_eq!(snapshot.summary.day_coverage, 2);
        assert_eq!(snapshot.summary.hour_daily_quil.as_deref(), Some("~8640.0"));
        assert_eq!(snapshot.summary.day_daily_quil.as_deref(), Some("~8640.0"));
    }

    #[test]
    fn seat_concentration_counts_reported_committees_at_risk() {
        let report = |own_group_seats| {
            Some(SeatConcentration {
                members: 17,
                quorum: 12,
                largest_group_seats: 5,
                own_group_seats,
                unaddressed_members: 0,
                concentration_seats: 4,
                epoch: 1393,
            })
        };
        let response = GetShardInfoResponse {
            shards: [report(4), report(3), report(0), None]
                .into_iter()
                .map(|seat_concentration| ShardRewardInfo {
                    seat_concentration,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };

        let snapshot = map_snapshot(true, response);

        assert_eq!(
            snapshot.summary.seat_concentration,
            Some(SeatConcentrationSnapshot {
                at_risk: 1,
                reported: 3,
            })
        );
    }

    fn shard(filter: u8, reward: u64, ring_known: Option<bool>) -> ShardRewardInfo {
        ShardRewardInfo {
            filter: vec![filter],
            active_provers: 7,
            ring: 1,
            shard_size: 1_048_576u64.to_be_bytes().to_vec(),
            estimated_reward: reward.to_be_bytes().to_vec(),
            is_allocated: filter == 1,
            data_shards: 8,
            ring_known,
            ..Default::default()
        }
    }

    fn filter(application: u8, suffix: &[u8]) -> Vec<u8> {
        let mut filter = vec![application; 32];
        filter.extend_from_slice(suffix);
        filter
    }

    fn explorer_snapshot(
        filter: Vec<u8>,
        active: u32,
        joining: u32,
        leaving: u32,
    ) -> ExplorerSnapshot {
        ExplorerSnapshot {
            rows: vec![ExplorerShard {
                filter_hex: hex::encode(filter),
                counts: ExplorerCounts {
                    active,
                    joining,
                    leaving,
                    ..Default::default()
                },
                size_bytes: 1_024u64.into(),
                active_workers: active + joining,
                data_shards: 8,
            }],
            collected_at_unix_seconds: 999,
            frame_number: Some(900),
            network_provers: Some(194),
            invalid_rows: 0,
            stale: false,
            error: None,
        }
    }
}
