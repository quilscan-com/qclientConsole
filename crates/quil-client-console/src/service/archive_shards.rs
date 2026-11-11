use std::{cmp::Reverse, collections::BTreeSet, path::Path};

use quil_rpc::archive_shard_list::ShardList;
use quil_rpc::ArchiveShardInfo;

use super::{node, prover, ServiceError};
use crate::domain::{ArchiveConsensus, ArchiveEndpointSnapshot, ArchiveShardSnapshot};

pub(super) async fn read(config_dir: &Path) -> Result<ArchiveShardSnapshot, ServiceError> {
    let config = quil_config::load_config(config_dir)
        .map_err(|error| ServiceError::Config(error.to_string()))?;
    let manager = prover::build_key_manager(&config, config_dir)?;
    let key = manager
        .get_signer_by_id("q-prover-key")
        .map_err(|error| ServiceError::Key(error.to_string()))?
        .private_key()
        .to_vec();
    let address = local_stream_addr(&config)?;
    let mut client = tokio::time::timeout(
        node::RPC_TIMEOUT,
        quil_rpc::ArchiveClient::connect_own_master(&address, &key),
    )
    .await
    .map_err(|_| {
        ServiceError::ArchiveShards("Timed out connecting to the node archive directory".into())
    })?
    .map_err(|error| {
        ServiceError::ArchiveShards(format!("Cannot connect to node {address}: {error}"))
    })?;
    let endpoints = tokio::time::timeout(node::RPC_TIMEOUT, client.get_archive_endpoints())
        .await
        .map_err(|_| {
            ServiceError::ArchiveShards("Timed out querying node archive endpoints".into())
        })?
        .map_err(|error| {
            ServiceError::ArchiveShards(format!("Cannot query node archive endpoints: {error}"))
        })?;
    if endpoints.is_empty() {
        return Err(ServiceError::ArchiveShards(
            "The node has no archive endpoints".into(),
        ));
    }

    let shard_keys = [quil_grid_shard_key()];
    let answers = futures::future::join_all(endpoints.iter().map(|endpoint| async {
        tokio::time::timeout(
            node::RPC_TIMEOUT,
            quil_rpc::fetch_shard_info_from_endpoint(endpoint, &key, &shard_keys),
        )
        .await
        .map_err(|_| "Query timed out".to_string())
        .and_then(|answer| answer.map_err(|error| error.to_string()))
    }))
    .await;
    let reports = endpoints
        .into_iter()
        .zip(answers)
        .map(|(endpoint, answer)| ArchiveReport {
            endpoint,
            result: report_result(answer),
        })
        .collect();
    Ok(compare(reports))
}

pub(super) fn local_stream_addr(config: &quil_config::Config) -> Result<String, ServiceError> {
    let stream = if config.p2p.stream_listen_multiaddr.is_empty() {
        "/ip4/0.0.0.0/tcp/8340"
    } else {
        config.p2p.stream_listen_multiaddr.as_str()
    };
    multiaddr_to_host_port(stream)
        .map(|address| address.replace("0.0.0.0", "127.0.0.1"))
        .map_err(ServiceError::Config)
}

fn multiaddr_to_host_port(multiaddr: &str) -> Result<String, String> {
    let parts = multiaddr.trim_matches('/').split('/').collect::<Vec<_>>();
    if parts.len() < 4 {
        return Err(format!("Unsupported stream address: {multiaddr}"));
    }
    let mut host = None;
    let mut port = None;
    for pair in parts.chunks_exact(2) {
        match pair[0] {
            "ip4" | "ip6" | "dns" | "dns4" | "dns6" => host = Some(pair[1]),
            "tcp" | "udp" => port = Some(pair[1]),
            _ => {}
        }
    }
    let (host, port) = host
        .zip(port)
        .ok_or_else(|| format!("Cannot extract host:port from stream address: {multiaddr}"))?;
    if host.contains(':') {
        Ok(format!("[{host}]:{port}"))
    } else {
        Ok(format!("{host}:{port}"))
    }
}

fn quil_grid_shard_key() -> Vec<u8> {
    let quil = quil_execution::domains::QUIL_TOKEN;
    let mut key = quil_hypergraph::addressing::get_bloom_filter_indices(&quil, 256, 3).to_vec();
    key.extend_from_slice(&quil);
    key
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or_default().to_string()
}

fn report_result(answer: Result<ArchiveShardInfo, String>) -> Result<ShardList, String> {
    let info = answer.map_err(|error| first_line(&error))?;
    if info.sizes.is_empty() && info.failed_shard_keys == 0 {
        return Err("reported no shards".into());
    }
    ShardList::from_complete(&info).ok_or_else(|| {
        format!(
            "partial answer: {} shard keys failed",
            info.failed_shard_keys
        )
    })
}

#[derive(Debug, Clone)]
struct ArchiveReport {
    pub endpoint: String,
    pub result: Result<ShardList, String>,
}

fn compare(reports: Vec<ArchiveReport>) -> ArchiveShardSnapshot {
    let mut groups: Vec<(ShardList, usize)> = Vec::new();
    for report in &reports {
        let Ok(list) = &report.result else {
            continue;
        };
        match groups
            .iter_mut()
            .find(|(group, _)| group.filters == list.filters)
        {
            Some((_, count)) => *count += 1,
            None => groups.push((list.clone(), 1)),
        }
    }
    groups.sort_by_key(|(_, count)| Reverse(*count));

    let answered = groups.iter().map(|(_, count)| count).sum();
    let summary = match groups.as_slice() {
        [] => ArchiveConsensus::NoAnswers,
        [_] => ArchiveConsensus::AllAgree,
        [first, second, ..] if first.1 == second.1 => ArchiveConsensus::NoMajority,
        _ => ArchiveConsensus::Majority,
    };
    let majority = (summary == ArchiveConsensus::Majority).then(|| &groups[0].0);
    let (all_filters, common_filters) = no_majority_bounds(summary, &groups);

    let archives: Vec<ArchiveEndpointSnapshot> = reports
        .into_iter()
        .map(|report| match report.result {
            Ok(list) => {
                let (status, extra, missing) = match majority {
                    Some(reference) if list.filters == reference.filters => {
                        ("agrees", Vec::new(), Vec::new())
                    }
                    Some(reference) => {
                        ("differs", list.only_in(reference), reference.only_in(&list))
                    }
                    None if summary == ArchiveConsensus::AllAgree => {
                        ("agrees", Vec::new(), Vec::new())
                    }
                    None => (
                        "no majority",
                        list.filters.difference(&common_filters).cloned().collect(),
                        all_filters.difference(&list.filters).cloned().collect(),
                    ),
                };
                ArchiveEndpointSnapshot {
                    endpoint: report.endpoint,
                    shard_count: Some(list.filters.len()),
                    frozen_count: Some(list.frozen.len()),
                    digest: Some(list.digest()),
                    status: status.into(),
                    extra_filters: hex_filters(extra),
                    missing_filters: hex_filters(missing),
                    error: None,
                }
            }
            Err(error) => ArchiveEndpointSnapshot {
                endpoint: report.endpoint,
                shard_count: None,
                frozen_count: None,
                digest: None,
                status: "error".into(),
                extra_filters: Vec::new(),
                missing_filters: Vec::new(),
                error: Some(error),
            },
        })
        .collect();

    ArchiveShardSnapshot {
        summary,
        total: archives.len(),
        answered,
        archives,
    }
}

fn no_majority_bounds(
    summary: ArchiveConsensus,
    groups: &[(ShardList, usize)],
) -> (BTreeSet<Vec<u8>>, BTreeSet<Vec<u8>>) {
    if summary != ArchiveConsensus::NoMajority {
        return (BTreeSet::new(), BTreeSet::new());
    }
    let all = groups
        .iter()
        .flat_map(|(list, _)| list.filters.iter().cloned())
        .collect::<BTreeSet<_>>();
    let common = all
        .iter()
        .filter(|filter| {
            groups
                .iter()
                .all(|(list, _)| list.filters.contains(*filter))
        })
        .cloned()
        .collect();
    (all, common)
}

fn hex_filters(filters: Vec<Vec<u8>>) -> Vec<String> {
    filters.into_iter().map(hex::encode).collect()
}

#[cfg(test)]
mod tests {
    use quil_rpc::ArchiveShardInfo;

    use super::*;

    fn list(filters: &[u8], frozen: &[u8]) -> ShardList {
        ShardList {
            filters: filters.iter().map(|filter| vec![*filter]).collect(),
            frozen: frozen.iter().map(|filter| vec![*filter]).collect(),
        }
    }

    fn report(endpoint: &str, result: Result<ShardList, String>) -> ArchiveReport {
        ArchiveReport {
            endpoint: endpoint.into(),
            result,
        }
    }

    #[test]
    fn quil_grid_key_is_bloom_l1_followed_by_quil_token() {
        let key = quil_grid_shard_key();
        let quil = quil_execution::domains::QUIL_TOKEN;
        let expected_l1 = quil_hypergraph::addressing::get_bloom_filter_indices(&quil, 256, 3);

        assert_eq!(key.len(), 35);
        assert_eq!(&key[..3], &expected_l1);
        assert_eq!(&key[3..], &quil);
    }

    #[test]
    fn local_stream_address_rewrites_unspecified_ipv4_to_loopback() {
        let mut config = quil_config::Config::default();
        config.p2p.stream_listen_multiaddr = "/ip4/0.0.0.0/tcp/8340".into();

        assert_eq!(local_stream_addr(&config).unwrap(), "127.0.0.1:8340");
    }

    #[test]
    fn stream_multiaddresses_support_dns_and_bracket_ipv6() {
        assert_eq!(
            multiaddr_to_host_port("/dns4/node.local/tcp/8340").unwrap(),
            "node.local:8340"
        );
        assert_eq!(
            multiaddr_to_host_port("/ip6/::1/tcp/8340").unwrap(),
            "[::1]:8340"
        );
    }

    #[test]
    fn invalid_stream_multiaddress_is_rejected() {
        assert!(multiaddr_to_host_port("/unix/node.sock").is_err());
    }

    #[test]
    fn unique_largest_group_becomes_majority() {
        let snapshot = compare(vec![
            report("a:8340", Ok(list(&[1, 2], &[2]))),
            report("b:8340", Ok(list(&[1, 2], &[2]))),
            report("c:8340", Ok(list(&[1, 3], &[]))),
        ]);

        assert_eq!(snapshot.summary, ArchiveConsensus::Majority);
        assert_eq!(snapshot.archives[2].missing_filters, vec!["02"]);
        assert_eq!(snapshot.archives[2].extra_filters, vec!["03"]);
        assert_eq!(snapshot.archives[0].frozen_count, Some(1));
    }

    #[test]
    fn tied_groups_have_no_majority() {
        let snapshot = compare(vec![
            report("a:8340", Ok(list(&[1], &[]))),
            report("b:8340", Ok(list(&[2], &[]))),
        ]);

        assert_eq!(snapshot.summary, ArchiveConsensus::NoMajority);
        assert!(snapshot
            .archives
            .iter()
            .all(|archive| archive.status == "no majority"));
        assert_eq!(snapshot.archives[0].extra_filters, vec!["01"]);
        assert_eq!(snapshot.archives[0].missing_filters, vec!["02"]);
        assert_eq!(snapshot.archives[1].extra_filters, vec!["02"]);
        assert_eq!(snapshot.archives[1].missing_filters, vec!["01"]);
    }

    #[test]
    fn failed_archives_are_retained_without_reducing_successful_count() {
        let snapshot = compare(vec![
            report("a:8340", Ok(list(&[1], &[]))),
            report("down:8340", Err("timeout".into())),
        ]);

        assert_eq!(snapshot.total, 2);
        assert_eq!(snapshot.answered, 1);
        assert_eq!(snapshot.archives[1].error.as_deref(), Some("timeout"));
        assert_eq!(snapshot.archives[1].status, "error");
    }

    #[test]
    fn equal_answers_all_agree() {
        let snapshot = compare(vec![
            report("a:8340", Ok(list(&[1, 2], &[2]))),
            report("b:8340", Ok(list(&[2, 1], &[]))),
        ]);

        assert_eq!(snapshot.summary, ArchiveConsensus::AllAgree);
        assert_eq!(snapshot.answered, 2);
        assert!(snapshot
            .archives
            .iter()
            .all(|archive| archive.status == "agrees"));
    }

    #[test]
    fn all_failed_have_no_answers() {
        let snapshot = compare(vec![
            report("a:8340", Err("timeout".into())),
            report("b:8340", Err("refused".into())),
        ]);

        assert_eq!(snapshot.summary, ArchiveConsensus::NoAnswers);
        assert_eq!(snapshot.total, 2);
        assert_eq!(snapshot.answered, 0);
        assert_eq!(snapshot.archives.len(), 2);
    }

    #[test]
    fn partial_response_is_retained_as_an_endpoint_error() {
        let mut info = ArchiveShardInfo::default();
        info.sizes.insert(vec![0xab], 1);
        info.failed_shard_keys = 1;

        assert_eq!(
            report_result(Ok(info)),
            Err("partial answer: 1 shard keys failed".into())
        );
    }

    #[test]
    fn empty_successful_response_is_retained_as_an_endpoint_error() {
        assert_eq!(
            report_result(Ok(ArchiveShardInfo::default())),
            Err("reported no shards".into())
        );
    }
}
