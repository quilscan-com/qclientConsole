use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;

use crate::{
    config::ConfigEntry,
    domain::{
        format_quil_amount, HealthTarget, HypergraphKind, NodeSnapshot, RewardMintPhase,
        WalletAction, WalletOperationSnapshot, WalletOperationStatus,
    },
    i18n::{
        web::{format_web_text, page_locale, web_text},
        Locale,
    },
    service::ServiceError,
};

use super::{require_session, ApiError, AppState};

#[derive(Serialize)]
pub(super) struct PageResponse {
    slug: String,
    title: &'static str,
    available: bool,
    message: Option<String>,
    data: Value,
}

impl PageResponse {
    fn ready(slug: &str, title: &'static str, data: Value) -> Self {
        Self {
            slug: slug.into(),
            title,
            available: true,
            message: None,
            data,
        }
    }

    fn unavailable(slug: &str, title: &'static str, message: impl Into<String>) -> Self {
        Self {
            slug: slug.into(),
            title,
            available: false,
            message: Some(message.into()),
            data: Value::Null,
        }
    }

    fn result(slug: &str, title: &'static str, result: Result<Value, ServiceError>) -> Self {
        match result {
            Ok(data) => Self::ready(slug, title, data),
            Err(error) => Self::unavailable(slug, title, error.to_string()),
        }
    }
}

pub(super) async fn page(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Result<Json<PageResponse>, ApiError> {
    require_session(&headers, &state.auth)?;
    let locale = page_locale(query.get("lang").map(String::as_str));
    let response = load_page(locale, &state, &slug, query.get("q").map(String::as_str))
        .await
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, format!("unknown page: {slug}")))?;
    Ok(Json(response))
}

async fn load_page(
    locale: Locale,
    state: &AppState,
    slug: &str,
    query: Option<&str>,
) -> Option<PageResponse> {
    let core = &state.core;
    let response = match slug {
        "overview" | "node-status" => {
            let snapshot = core.state();
            PageResponse::ready(
                slug,
                if slug == "overview" {
                    "Overview"
                } else {
                    "Node Status"
                },
                node_view(locale,
                    snapshot.active_config.as_ref(),
                    snapshot.node_snapshot.as_ref(),
                ),
            )
        }
        "configs" => {
            let snapshot = core.state();
            let sections = snapshot
                .active_config
                .as_ref()
                .map_or_else(Vec::new, |entry| {
                    vec![section(
                        web_text(locale, "Current node configuration"),
                        vec![
                            field(web_text(locale, "Configuration name"), entry.name.clone()),
                            field(web_text(locale, "Configuration directory"), entry.path.to_string_lossy()),
                            field(web_text(locale, "Default configuration"), yes_no(locale, entry.is_default)),
                        ],
                    )]
                });
            PageResponse::ready(
                slug,
                "Node Configurations",
                view(
                    Vec::new(),
                    sections,
                    Vec::new(),
                    snapshot.active_config.is_none().then(|| {
                        empty_state(
                            web_text(locale, "No node configuration selected"),
                            web_text(locale, "Select a valid node configuration directory in the terminal console first."),
                        )
                    }),
                ),
            )
        }
        "runtime" => {
            let result = core.runtime_status().await.map(|value| {
                view(
                    vec![
                        stat(
                            web_text(locale, "Service status"),
                            if value.running {
                                web_text(locale, "Running")
                            } else {
                                web_text(locale, "Not running")
                            },
                            status_tone(value.running),
                        ),
                        stat(
                            web_text(locale, "Startup status"),
                            value.enabled.map(|value| yes_no(locale, value)).unwrap_or(web_text(locale, "Unknown")),
                            "blue",
                        ),
                    ],
                    vec![
                        section(
                            web_text(locale, "Runtime environment"),
                            vec![
                                field(web_text(locale, "Platform"), value.platform),
                                field(web_text(locale, "Service manager"), value.manager),
                                field(web_text(locale, "Service name"), value.service_name),
                                field(web_text(locale, "Current status"), value.state),
                                field(
                                    web_text(locale, "Process ID"),
                                    value
                                        .pid
                                        .map(|pid| pid.to_string())
                                        .unwrap_or_else(|| "—".into()),
                                ),
                            ],
                        ),
                        section(
                            web_text(locale, "Installation and updates"),
                            vec![
                                field(web_text(locale, "Installed"), yes_no(locale, value.installed)),
                                field(
                                    web_text(locale, "Automatic updates"),
                                    value.auto_update_enabled.map(|value| yes_no(locale, value)).unwrap_or(web_text(locale, "Unknown")),
                                ),
                                field(web_text(locale, "Service definition"), value.definition_path),
                                field(web_text(locale, "Description"), value.note.unwrap_or_else(|| "—".into())),
                            ],
                        ),
                    ],
                    Vec::new(),
                    None,
                )
            });
            PageResponse::result(slug, "Runtime", result)
        }
        "archive-shards" => {
            let result = core.archive_shards().await.map(|value| {
                let empty = value.archives.is_empty().then(|| empty_state(web_text(locale, "No Archive responses"), web_text(locale, "The current configuration returned no Archive Shards data.")));
                view(
                    vec![
                        stat(web_text(locale, "Archive endpoints"), value.total, "violet"),
                        stat(web_text(locale, "Answered"), value.answered, status_tone(value.answered == value.total && value.total > 0)),
                        stat(web_text(locale, "Consensus"), archive_consensus_label(locale, value.summary), "blue"),
                    ],
                    Vec::new(),
                    vec![table(web_text(locale, "Archive endpoints"), vec![
                        column("endpoint", web_text(locale, "Endpoint")), column("status", web_text(locale, "Status")), column("shard_count", web_text(locale, "Shard count")),
                        column("frozen_count", web_text(locale, "Frozen count")), column("digest", web_text(locale, "Digest")),
                        column("extra_filters", web_text(locale, "Extra filters")), column("missing_filters", web_text(locale, "Missing filters")), column("error", web_text(locale, "Error")),
                    ], value.archives.into_iter().map(|archive| json!({
                        "endpoint": archive.endpoint, "status": archive.status,
                        "shard_count": archive.shard_count, "frozen_count": archive.frozen_count,
                        "digest": archive.digest, "extra_filters": archive.extra_filters,
                        "missing_filters": archive.missing_filters, "error": archive.error,
                    })).collect())],
                    empty,
                )
            });
            PageResponse::result(slug, "Archive Shards", result)
        }
        "prover" => {
            let snapshot = core.state();
            let result = snapshot
                .node_snapshot
                .as_ref()
                .map(|node| prover_view(locale, node))
                .ok_or_else(|| {
                    ServiceError::Connect(web_text(locale, "Refresh node status before viewing Prover data").into())
                });
            PageResponse::result(slug, "Prover", result)
        }
        "shards" => {
            let result = core.shard_info(true).await.map(|value| {
                let empty = value.shards.is_empty().then(|| empty_state(web_text(locale, "Shard directory unavailable"), web_text(locale, "Neither the current node nor the official directory returned any shards.")));
                let mut stats = vec![
                    stat(web_text(locale, "Total shards"), value.summary.total, "violet"),
                    stat(web_text(locale, "Allocated"), value.summary.allocated, "green"),
                    stat(web_text(locale, "Active Provers"), value.summary.active_provers, "blue"),
                    stat(web_text(locale, "Data shards"), value.summary.data_shards, "amber"),
                ];
                if let Some(concentration) = value.summary.seat_concentration.as_ref() {
                    stats.push(keyed_stat(
                        "seat_concentration",
                        "Seat concentration",
                        format!("{}/{}", concentration.at_risk, concentration.reported),
                        if concentration.at_risk > 0 { "amber" } else { "green" },
                    ));
                }
                view(
                    stats,
                    vec![section(web_text(locale, "Shard directory"), vec![
                        field("Global Frame", value.frame_number), field("Difficulty", value.difficulty),
                        field("World State", value.world_state),
                        field(web_text(locale, "Network Provers"), value.network_provers.map(|count| count.to_string()).unwrap_or_else(|| web_text(locale, "Unknown").into())),
                        field(web_text(locale, "Directory status"), if value.directory_stale { web_text(locale, "Data may be stale") } else { web_text(locale, "Up to date") }),
                        field(web_text(locale, "Directory error"), value.directory_error.unwrap_or_else(|| web_text(locale, "None").into())),
                    ])],
                    vec![table(web_text(locale, "Complete shard directory"), vec![
                        column("filter", "Filter"), column("ring", "Ring"), column("active_provers", web_text(locale, "Active Provers")),
                        column("official", web_text(locale, "Official data")), column("allocated", web_text(locale, "Allocated")),
                        column("materialized_frame", web_text(locale, "Materialized frame")), column("latest_frame", web_text(locale, "Latest frame")),
                        column("daily_quil", web_text(locale, "Estimated daily rewards")),
                    ], value.shards.into_iter().map(|shard| json!({
                        "filter": shard.filter_hex, "ring": shard.ring, "active_provers": shard.active_provers,
                        "official": yes_no(locale, shard.official), "allocated": yes_no(locale, shard.is_allocated),
                        "materialized_frame": shard.materialized_frame, "latest_frame": shard.latest_frame,
                        "daily_quil": shard.day_daily_quil.map(|amount| format!("{amount} QUIL")),
                    })).collect())],
                    empty,
                )
            });
            PageResponse::result(slug, "Shards", result)
        }
        "rewards" => {
            let result: Result<Value, ServiceError> = async {
                let reward = core.reward_summary().await?;
                let mint = core.load_reward_mint().await?;
                let claimable = reward
                    .claimable_base_units
                    .map(quil)
                    .unwrap_or_else(|| web_text(locale, "Unavailable").into());
                let recipient = reward.recipient_address_hex.clone();
                let witness_error = reward.witness_error.clone();
                let mint_poll_error = core
                    .state()
                    .active_config
                    .as_ref()
                    .and_then(|config| state.mint_tasks.poll_error(&config.path));
                let mut sections = vec![section(
                    web_text(locale, "Reward identity"),
                    vec![
                        keyed_field(
                            "reward_identity",
                            web_text(locale, "Identity status"),
                            reward_identity_label(locale, &reward.identity),
                        ),
                        keyed_field("prover_address", web_text(locale, "Prover address"), reward.prover_address_hex),
                        keyed_field(
                            "legacy_account_address",
                            web_text(locale, "Legacy account address"),
                            reward.legacy_account_address_hex,
                        ),
                        field(
                            web_text(locale, "Reward proof"),
                            if reward.proof_verified {
                                web_text(locale, "Verified")
                            } else {
                                web_text(locale, "Unverified")
                            },
                        ),
                        keyed_field(
                            "cited_frame",
                            web_text(locale, "Cited frame"),
                            reward
                                .cited_frame
                                .map(|frame| frame.to_string())
                                .unwrap_or_else(|| web_text(locale, "Unavailable").into()),
                        ),
                    ],
                )];
                sections.push(section(
                    web_text(locale, "Mint progress"),
                    vec![
                        keyed_field(
                            "phase_code",
                            web_text(locale, "Phase code"),
                            mint.as_ref()
                                .map(|status| mint_phase_code(status.phase))
                                .unwrap_or("not_started"),
                        ),
                        keyed_field("recipient", web_text(locale, "Current confidential wallet"), recipient),
                        keyed_field(
                            "receipt",
                            "Receipt",
                            mint.as_ref()
                                .map(|status| status.receipt.as_str())
                                .filter(|receipt| !receipt.trim().is_empty()),
                        ),
                        keyed_field(
                            "mint_detail",
                            web_text(locale, "Status detail"),
                            mint.as_ref()
                                .and_then(|status| status.detail.as_deref())
                                .unwrap_or_else(|| {
                                    if mint.is_some() {
                                        web_text(locale, "In progress")
                                    } else {
                                        web_text(locale, "Not started")
                                    }
                                }),
                        ),
                        keyed_field(
                            "mint_poll_error",
                            web_text(locale, "Background polling error"),
                            mint_poll_error.as_deref(),
                        ),
                        keyed_field(
                            "can_rebroadcast",
                            web_text(locale, "Can rebroadcast"),
                            mint.as_ref().is_some_and(|status| status.can_rebroadcast),
                        ),
                    ],
                ));
                if let Some(error) = witness_error.as_deref() {
                    sections.push(section(
                        web_text(locale, "Read status"),
                        vec![keyed_field("witness_error", web_text(locale, "Reward Witness"), error)],
                    ));
                }
                let empty = if let Some(error) = witness_error {
                    Some(empty_state(
                        web_text(locale, "Reward Witness unavailable"),
                        format_web_text(locale, "{}. The current wallet and identity remain available; check the read-only RPC under Other.", &error),
                    ))
                } else {
                    reward
                        .claimable_base_units
                        .filter(|amount| *amount > 0)
                        .is_none()
                        .then(|| {
                            empty_state(web_text(locale, "No claimable rewards"), web_text(locale, "The current reward identity has no verified claimable rewards."))
                        })
                };
                Ok(view(
                    vec![
                        keyed_stat("claimable", web_text(locale, "Claimable rewards"), claimable, "green"),
                        keyed_stat(
                            "proof_verified",
                            web_text(locale, "Proof status"),
                            reward.proof_verified,
                            status_tone(reward.proof_verified),
                        ),
                        keyed_stat(
                            "mint_phase",
                            web_text(locale, "Mint status"),
                            mint.as_ref()
                                .map(|status| mint_phase_label(locale, status.phase))
                                .unwrap_or(web_text(locale, "Not begun")),
                            "blue",
                        ),
                    ],
                    sections,
                    Vec::new(),
                    empty,
                ))
            }
            .await;
            PageResponse::result(slug, "Rewards / Mint", result)
        }
        "legacy-assets" => {
            let result = core.legacy_assets().await.map(|value| {
                let empty = value.coins.is_empty().then(|| if value.identity_available {
                    empty_state(web_text(locale, "No legacy assets"), web_text(locale, "The current account returned no Legacy Coins."))
                } else {
                    empty_state(web_text(locale, "Legacy identity unavailable"), web_text(locale, "The current configuration has no identity for reading Legacy Assets."))
                });
                view(
                    vec![
                        stat(web_text(locale, "Available balance"), quil(value.unshielded_total_base_units), "green"),
                        stat(web_text(locale, "Available Coins"), value.unshielded_count, "blue"), stat("Shielded Coins", value.shielded_count, "violet"),
                    ],
                    vec![section(web_text(locale, "Identity"), vec![field(web_text(locale, "Legacy identity"), if value.identity_available { web_text(locale, "Available") } else { web_text(locale, "Unavailable") })])],
                    vec![table("Legacy Coins", vec![
                        column("address", web_text(locale, "Coin address")), column("amount", web_text(locale, "Amount")), column("shielded", "Shielded"),
                    ], value.coins.into_iter().map(|coin| json!({
                        "address": coin.address_hex, "amount": quil(coin.amount_base_units), "shielded": yes_no(locale, coin.shielded),
                    })).collect())], empty,
                )
            });
            PageResponse::result(slug, "Legacy Assets", result)
        }
        "confidential-wallet" => {
            let result = core.confidential_wallet().await.map(|value| {
                let empty = value.coins.is_empty().then(|| empty_state(web_text(locale, "No spendable Coins"), web_text(locale, "The wallet scan completed with no Confidential Coins.")));
                let coin_count = value.coins.len();
                let mut tables = vec![table("Confidential Coins", vec![column("address", web_text(locale, "Coin address")), column("amount", web_text(locale, "Amount"))],
                    value.coins.into_iter().map(|coin| json!({ "address": coin.address_hex, "amount": quil(coin.amount_base_units) })).collect())];
                if !value.recent_operations.is_empty() {
                    tables.push(operation_table(locale, value.recent_operations));
                }
                view(
                    vec![stat(web_text(locale, "Wallet balance"), quil(value.total_base_units), "green"), stat("Coins", coin_count, "blue")],
                    vec![section(web_text(locale, "Wallet"), vec![field(web_text(locale, "Confidential wallet address"), value.address_hex)])],
                    tables,
                    empty,
                )
            });
            PageResponse::result(slug, "Confidential Wallet", result)
        }
        "pending-escrows" => {
            let result = core.pending_escrows().await.map(|value| {
                let empty = value.escrows.is_empty().then(|| value.unavailable_reason.as_deref().map_or_else(
                    || empty_state(web_text(locale, "No pending transfers"), web_text(locale, "The current wallet has no receivable or refundable Escrows.")),
                    |reason| empty_state(web_text(locale, "Pending transfers unavailable"), reason),
                ));
                let escrow_count = value.escrows.len();
                let mut tables = vec![table("Pending Escrows", vec![
                    column("address", web_text(locale, "Escrow address")), column("amount", web_text(locale, "Amount")),
                    column("branch", web_text(locale, "Executable branch")), column("refund_frame", web_text(locale, "Refund start frame")),
                ], value.escrows.into_iter().map(|escrow| json!({
                    "address": escrow.address_hex, "amount": quil(escrow.amount_base_units),
                    "branch": escrow.branch.localized_label(locale), "refund_frame": escrow.refund_after_global_frame,
                })).collect())];
                if !value.recent_operations.is_empty() {
                    tables.push(operation_table(locale, value.recent_operations));
                }
                view(
                    vec![
                        stat(web_text(locale, "Pending count"), escrow_count, "amber"),
                        stat(web_text(locale, "Wallet identity"), if value.identity_available { web_text(locale, "Available") } else { web_text(locale, "Unavailable") }, status_tone(value.identity_available)),
                    ], Vec::new(),
                    tables, empty,
                )
            });
            PageResponse::result(slug, "Pending Escrows", result)
        }
        "keys" => {
            let result = core.list_keys().await.map(|values| {
                let empty = values.is_empty().then(|| empty_state(web_text(locale, "No keys"), web_text(locale, "The current configuration returned no public keys.")));
                view(vec![stat(web_text(locale, "Key count"), values.len(), "violet")], Vec::new(),
                    vec![table(web_text(locale, "Local keys"), vec![column("id", web_text(locale, "Key ID")), column("type", web_text(locale, "Type")), column("public_key", web_text(locale, "Public key"))],
                        values.into_iter().map(|key| json!({ "id": key.id, "type": key.type_name(), "public_key": key.public_key_hex })).collect())], empty)
            });
            PageResponse::result(slug, "Keys", result)
        }
        "aliases" => {
            let result = core.list_aliases().await.map(|values| {
                let empty = values.is_empty().then(|| empty_state(web_text(locale, "No address aliases"), web_text(locale, "The current configuration has no saved address aliases.")));
                view(vec![stat(web_text(locale, "Alias count"), values.len(), "violet")], Vec::new(),
                    vec![table(web_text(locale, "Address aliases"), vec![column("name", web_text(locale, "Name")), column("address", web_text(locale, "Address")), column("kind", web_text(locale, "Type"))],
                        values.into_iter().map(|alias| json!({ "name": alias.name, "address": alias.address_hex, "kind": alias.kind })).collect())], empty)
            });
            PageResponse::result(slug, "Aliases", result)
        }
        "health" => {
            let result = core.health_report(HealthTarget::SelfNode).await.map(|value| {
                let empty = value.workers.is_empty().then(|| empty_state(web_text(locale, "No Worker health data"), web_text(locale, "The node health report contains no Worker details.")));
                view(
                    vec![
                        stat("Global Head", value.global_head_frame, "blue"), stat("Workers", value.workers.len(), "green"),
                        stat(web_text(locale, "Omitted Workers"), value.omitted_workers, "amber"),
                    ],
                    vec![section(web_text(locale, "Health report"), vec![
                        field("Release", value.release), field("Build Fingerprint", value.build_fingerprint), field(web_text(locale, "Generated at"), value.generated_unix_ms),
                    ])],
                    vec![table(web_text(locale, "Worker health"), vec![
                        column("core_id", "Core"), column("filter", "Filter"), column("allocated", web_text(locale, "Allocated")),
                        column("phase", web_text(locale, "Phase")), column("blocker", web_text(locale, "Blocker")),
                        column("materialized_frame", web_text(locale, "Materialized frame")), column("awaited_frame", web_text(locale, "Awaited frame")),
                    ], value.workers.into_iter().map(|worker| json!({
                        "core_id": worker.core_id, "filter": worker.filter_hex, "allocated": yes_no(locale, worker.allocated),
                        "phase": worker.phase, "blocker": worker.blocker,
                        "materialized_frame": worker.materialized_frame, "awaited_frame": worker.awaited_frame,
                    })).collect())], empty,
                )
            });
            PageResponse::result(slug, "Health", result)
        }
        "logs" => {
            let result = core.read_logs(0, 200).await.map(|value| {
                let empty = value
                    .lines
                    .is_empty()
                    .then(|| empty_state(web_text(locale, "No logs"), web_text(locale, "The current log file contains no lines to display.")));
                view(
                    vec![stat(web_text(locale, "Log lines"), value.lines.len(), "blue")],
                    vec![section(
                        web_text(locale, "Log source"),
                        vec![field("Core", value.core_id), field(web_text(locale, "File"), value.path)],
                    )],
                    vec![table(
                        web_text(locale, "Recent logs"),
                        vec![column("line", web_text(locale, "Content"))],
                        value
                            .lines
                            .into_iter()
                            .map(|line| json!({ "line": line }))
                            .collect(),
                    )],
                    empty,
                )
            });
            PageResponse::result(slug, "Logs", result)
        }
        "hypergraph" => match nonempty(query) {
            None => PageResponse::unavailable(
                slug,
                "Hypergraph",
                web_text(locale, "Enter a Vertex address or hyperedge:<address> to run a query."),
            ),
            Some(input) => {
                let (kind, address) = input
                    .split_once(':')
                    .map(|(kind, address)| {
                        (
                            if kind.eq_ignore_ascii_case("hyperedge") {
                                HypergraphKind::Hyperedge
                            } else {
                                HypergraphKind::Vertex
                            },
                            address,
                        )
                    })
                    .unwrap_or((HypergraphKind::Vertex, input));
                let result = core.query_hypergraph(kind, address.to_owned()).await.map(|value| {
                    let empty = value.entries.is_empty().then(|| empty_state(web_text(locale, "No set entries"), web_text(locale, "The target exists, but the current set returned no entries.")));
                    view(
                        vec![stat(web_text(locale, "Presence"), value.present.map(|value| yes_no(locale, value)).unwrap_or(web_text(locale, "Unknown")), status_tone(value.present == Some(true)))],
                        vec![section(web_text(locale, "Query target"), vec![
                            field(web_text(locale, "Type"), value.kind.label()), field(web_text(locale, "Address"), value.address_hex), field(web_text(locale, "Set type"), value.set_type),
                            field("Phase", value.phase_type), field("Shard L1", value.shard_l1_hex), field("Shard L2", value.shard_l2_hex),
                        ])],
                        vec![table(web_text(locale, "Hypergraph entries"), vec![column("key", "Key"), column("value", "Value")],
                            value.entries.into_iter().map(|entry| json!({ "key": entry.key_hex, "value": entry.value_hex })).collect())], empty,
                    )
                });
                PageResponse::result(slug, "Hypergraph", result)
            }
        },
        "diagnostics" => match nonempty(query) {
            None => PageResponse::unavailable(
                slug,
                "Diagnostics",
                web_text(locale, "Enter a node Multiaddr to run network diagnostics."),
            ),
            Some(address) => {
                let result = core.diagnostics_dial(address.to_owned()).await.map(|value| view(
                    vec![
                        stat(web_text(locale, "Connection time"), format!("{} ms", value.connection_ms), "blue"),
                        stat("Ping", value.ping_ms.map(|ms| format!("{ms} ms")).unwrap_or_else(|| web_text(locale, "Unavailable").into()), "green"),
                        stat(web_text(locale, "Listen addresses"), value.listen_addresses.len(), "violet"),
                    ],
                    vec![section(web_text(locale, "Remote node"), vec![
                        field(web_text(locale, "Target address"), value.address), field("Peer ID", value.peer_id), field(web_text(locale, "Transport"), value.transport),
                        field("Agent Version", value.agent_version.unwrap_or_else(|| web_text(locale, "Unknown").into())),
                        field("Protocol Version", value.protocol_version.unwrap_or_else(|| web_text(locale, "Unknown").into())),
                        field("Observed Address", value.observed_address.unwrap_or_else(|| web_text(locale, "Unknown").into())),
                        field(web_text(locale, "Supported protocols"), value.protocols.join(", ")),
                    ])],
                    vec![table(web_text(locale, "Listen addresses"), vec![column("multiaddr", "Multiaddr"), column("reach", web_text(locale, "Reach"))],
                        value.listen_addresses.into_iter().map(|address| json!({ "multiaddr": address.multiaddr, "reach": address.reach.localized_label(locale) })).collect())],
                    None,
                ));
                PageResponse::result(slug, "Diagnostics", result)
            }
        },
        "peer-resolve" => match nonempty(query) {
            None => {
                PageResponse::unavailable(slug, "Peer Resolve", web_text(locale, "Enter a Peer ID to resolve node addresses."))
            }
            Some(peer_id) => {
                let result = core.resolve_peer(peer_id.to_owned()).await.map(|value| {
                    let empty = value.addresses.is_empty().then(|| empty_state(web_text(locale, "No available addresses"), web_text(locale, "The node directory returned no addresses for this Peer.")));
                    view(
                        vec![stat(web_text(locale, "Resolved addresses"), value.addresses.len(), "blue")],
                        vec![section("Peer", vec![field("Peer ID", value.peer_id)])],
                        vec![table(web_text(locale, "Peer addresses"), vec![
                            column("kind", web_text(locale, "Type")), column("host_port", "Host / Port"), column("reach", web_text(locale, "Reach")),
                            column("multiaddr", "Multiaddr"), column("filter", "Filter"),
                        ], value.addresses.into_iter().map(|address| json!({
                            "kind": address.kind, "host_port": address.host_port, "reach": address.reach.localized_label(locale),
                            "multiaddr": address.multiaddr, "filter": address.filter,
                        })).collect())], empty,
                    )
                });
                PageResponse::result(slug, "Peer Resolve", result)
            }
        },
        "diagnostics-identity" => PageResponse::unavailable(
            slug,
            "Diagnostics Identity",
            web_text(locale, "Diagnostic identity reading and creation are not yet available on the Web; use the terminal console."),
        ),
        "log-export" => PageResponse::unavailable(
            slug,
            "Log Export",
            web_text(locale, "Redacted log export is not yet available on the Web; use the terminal console."),
        ),
        "remote-health" => PageResponse::unavailable(
            slug,
            "Remote Health",
            web_text(locale, "Remote health reports are not yet available on the Web; use the terminal console."),
        ),
        "other" => PageResponse::unavailable(
            slug,
            "Other",
            web_text(locale, "Console settings use the settings API to load Web access, port, and language options."),
        ),
        "help" => PageResponse::ready(
            slug,
            "Help",
            view(
                Vec::new(),
                vec![
                    section(
                        web_text(locale, "Navigation"),
                        vec![field(
                            web_text(locale, "Description"),
                            web_text(locale, "Select a feature on the left; Web and terminal share the current node configuration."),
                        )],
                    ),
                    section(
                        web_text(locale, "Login session"),
                        vec![field(web_text(locale, "Automatic logout"), web_text(locale, "Sign in again after 30 minutes of inactivity."))],
                    ),
                    section(
                        web_text(locale, "Data refresh"),
                        vec![field(
                            web_text(locale, "Description"),
                            web_text(locale, "Click Refresh at the top right to reload data for the current feature."),
                        )],
                    ),
                ],
                Vec::new(),
                None,
            ),
        ),
        _ => return None,
    };
    Some(response)
}

fn node_view(locale: Locale, config: Option<&ConfigEntry>, node: Option<&NodeSnapshot>) -> Value {
    let mut sections = Vec::new();
    if let Some(config) = config {
        sections.push(section(
            web_text(locale, "Active configuration"),
            vec![
                field(web_text(locale, "Configuration name"), config.name.clone()),
                field(
                    web_text(locale, "Configuration directory"),
                    config.path.to_string_lossy(),
                ),
                field(
                    web_text(locale, "Default configuration"),
                    yes_no(locale, config.is_default),
                ),
            ],
        ));
    }
    if let Some(node) = node {
        sections.extend([
            section(
                web_text(locale, "Connection status"),
                vec![
                    field("Node RPC", node.rpc_address.clone()),
                    field(
                        web_text(locale, "RPC connection"),
                        if node.connected {
                            web_text(locale, "Connected")
                        } else {
                            web_text(locale, "Disconnected")
                        },
                    ),
                    field(
                        web_text(locale, "Node reachable"),
                        yes_no(locale, node.reachable),
                    ),
                    field("Peer ID", node.peer_id.clone()),
                    field(
                        "Peer Seniority",
                        node.peer_seniority
                            .clone()
                            .unwrap_or_else(|| web_text(locale, "Unavailable").into()),
                    ),
                    field("Peer Score", node.peer_score),
                ],
            ),
            section(
                web_text(locale, "Frames and version"),
                vec![
                    field(web_text(locale, "Received frame"), node.last_received_frame),
                    field("Global Head", node.last_global_head_frame),
                    field(
                        "Frame Lag",
                        node.last_global_head_frame
                            .saturating_sub(node.last_received_frame),
                    ),
                    field(
                        web_text(locale, "Node version"),
                        version(locale, &node.version, &node.patch_number),
                    ),
                ],
            ),
            section(
                "Worker",
                vec![
                    field(web_text(locale, "Worker processes"), node.running_workers),
                    field(web_text(locale, "Allocated"), node.allocated_workers),
                    field(web_text(locale, "Allocation count"), node.allocations.len()),
                    field(
                        web_text(locale, "Worker read error"),
                        node.worker_info_error
                            .clone()
                            .unwrap_or_else(|| web_text(locale, "None").into()),
                    ),
                ],
            ),
        ]);
    }
    view(
        node.map(|node| {
            vec![
                stat(
                    web_text(locale, "Connection status"),
                    if node.connected {
                        web_text(locale, "Connected")
                    } else {
                        web_text(locale, "Disconnected")
                    },
                    status_tone(node.connected),
                ),
                stat(
                    "Frame Lag",
                    node.last_global_head_frame
                        .saturating_sub(node.last_received_frame),
                    "violet",
                ),
                stat(
                    web_text(locale, "Worker processes"),
                    node.running_workers,
                    "green",
                ),
                stat(
                    web_text(locale, "Allocated Workers"),
                    node.allocated_workers,
                    "amber",
                ),
            ]
        })
        .unwrap_or_default(),
        sections,
        Vec::new(),
        node.is_none().then(|| {
            empty_state(
                web_text(locale, "Node status not loaded"),
                web_text(
                    locale,
                    "Refresh the page or check the current node configuration and RPC service.",
                ),
            )
        }),
    )
}

fn prover_view(locale: Locale, node: &NodeSnapshot) -> Value {
    let idle_workers = node
        .workers
        .iter()
        .filter(|worker| worker.filter_hex.is_empty() && worker.execution.is_none())
        .count();
    let empty = (node.allocations.is_empty() && node.workers.is_empty()).then(|| {
        empty_state(
            web_text(locale, "No Prover data"),
            web_text(
                locale,
                "The node returned no Allocation or Worker information.",
            ),
        )
    });
    let mut sections = node
        .worker_info_error
        .as_ref()
        .map_or_else(Vec::new, |error| {
            vec![section(
                web_text(locale, "Worker data"),
                vec![field(web_text(locale, "Read error"), error)],
            )]
        });
    if node.allocations.is_empty() && !node.workers.is_empty() && idle_workers == node.workers.len()
    {
        sections.push(section(
            web_text(locale, "Worker status"),
            vec![
                field(web_text(locale, "Current status"), web_text(locale, "Standby")),
                field(
                    web_text(locale, "Description"),
                    format_web_text(locale, "{} Worker processes started without an assigned Filter or Prover Allocation.", &node.workers.len().to_string()),
                ),
            ],
        ));
    }
    let mut tables = Vec::new();
    if !node.allocations.is_empty() {
        tables.push(table("Allocations", vec![
            column("filter", "Filter"), column("status", web_text(locale, "Status")), column("join_frame", "Join Frame"),
            column("join_confirm_frame", web_text(locale, "Confirmation frame")), column("leave_frame", "Leave Frame"),
            column("last_active_frame", web_text(locale, "Last active frame")), column("epoch", "Epoch"),
        ], node.allocations.iter().map(|allocation| json!({
            "filter": allocation.filter_hex, "status": format!("{:?}", allocation.status),
            "join_frame": allocation.join_frame_number, "join_confirm_frame": allocation.join_confirm_frame_number,
            "leave_frame": allocation.leave_frame_number, "last_active_frame": allocation.last_active_frame_number, "epoch": allocation.epoch,
        })).collect()));
    }
    tables.push(table("Workers", vec![
        column("core_id", "Core"), column("filter", "Filter"), column("available_storage", web_text(locale, "Available storage")),
        column("total_storage", web_text(locale, "Total storage")), column("managed", web_text(locale, "Management")), column("state", web_text(locale, "Execution state")),
        column("blocker", web_text(locale, "Blocker")), column("materialized_frame", web_text(locale, "Materialized frame")),
    ], node.workers.iter().map(|worker| json!({
        "core_id": worker.core_id,
        "filter": if worker.filter_hex.is_empty() { web_text(locale, "Unallocated") } else { worker.filter_hex.as_str() },
        "available_storage": worker_storage(locale, worker.available_storage, worker.total_storage),
        "total_storage": worker_storage(locale, worker.total_storage, worker.total_storage),
        "managed": if worker.manually_managed { web_text(locale, "Manual") } else { web_text(locale, "Automatic") },
        "state": worker.execution.as_ref().map(|execution| execution.state.as_str()).unwrap_or_else(|| if worker.filter_hex.is_empty() { web_text(locale, "Standby") } else { web_text(locale, "Not reported") }),
        "blocker": worker.execution.as_ref().and_then(|execution| execution.blocker.clone()),
        "materialized_frame": worker.execution.as_ref().and_then(|execution| execution.materialized_frame),
    })).collect()));
    view(
        vec![
            stat(
                web_text(locale, "Worker processes"),
                node.running_workers,
                "green",
            ),
            stat(
                web_text(locale, "Allocated Workers"),
                node.allocated_workers,
                "amber",
            ),
            stat(web_text(locale, "Standby Workers"), idle_workers, "blue"),
            stat("Allocations", node.allocations.len(), "violet"),
        ],
        sections,
        tables,
        empty,
    )
}

fn worker_storage(locale: Locale, value: u64, total: u64) -> Value {
    if total == 0 {
        json!(web_text(locale, "Not reported"))
    } else {
        json!(value)
    }
}

fn view(
    stats: Vec<Value>,
    sections: Vec<Value>,
    tables: Vec<Value>,
    empty: Option<Value>,
) -> Value {
    json!({ "stats": stats, "sections": sections, "tables": tables, "empty_state": empty })
}

fn stat(label: impl Into<String>, value: impl Serialize, tone: &'static str) -> Value {
    json!({ "label": label.into(), "value": value, "tone": tone })
}

fn keyed_stat(
    key: &'static str,
    label: impl Into<String>,
    value: impl Serialize,
    tone: &'static str,
) -> Value {
    json!({ "key": key, "label": label.into(), "value": value, "tone": tone })
}

fn section(title: impl Into<String>, fields: Vec<Value>) -> Value {
    json!({ "title": title.into(), "fields": fields })
}

fn field(label: impl Into<String>, value: impl Serialize) -> Value {
    json!({ "label": label.into(), "value": value })
}

fn keyed_field(key: &'static str, label: impl Into<String>, value: impl Serialize) -> Value {
    json!({ "key": key, "label": label.into(), "value": value })
}

fn table(title: impl Into<String>, columns: Vec<Value>, rows: Vec<Value>) -> Value {
    json!({ "title": title.into(), "columns": columns, "rows": rows })
}

fn column(key: &'static str, label: &'static str) -> Value {
    json!({ "key": key, "label": label })
}
fn empty_state(title: impl Into<String>, detail: impl Into<String>) -> Value {
    json!({ "title": title.into(), "detail": detail.into() })
}
fn nonempty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}
fn yes_no(locale: Locale, value: bool) -> &'static str {
    if value {
        web_text(locale, "Yes")
    } else {
        web_text(locale, "No")
    }
}
fn status_tone(ok: bool) -> &'static str {
    if ok {
        "green"
    } else {
        "amber"
    }
}
fn quil(value: u128) -> String {
    format!("{} QUIL", format_quil_amount(value))
}

fn version(locale: Locale, version: &[u8], patch: &[u8]) -> String {
    let base = version
        .iter()
        .map(u8::to_string)
        .collect::<Vec<_>>()
        .join(".");
    let patch = patch
        .iter()
        .map(u8::to_string)
        .collect::<Vec<_>>()
        .join(".");
    match (base.is_empty(), patch.is_empty()) {
        (true, true) => web_text(locale, "Unknown").into(),
        (false, true) => base,
        (true, false) => patch,
        (false, false) => format!("{base}.{patch}"),
    }
}

fn archive_consensus_label(locale: Locale, value: crate::domain::ArchiveConsensus) -> &'static str {
    use crate::domain::ArchiveConsensus::*;
    match value {
        AllAgree => web_text(locale, "All agree"),
        Majority => web_text(locale, "Majority agreement"),
        NoMajority => web_text(locale, "No majority"),
        NoAnswers => web_text(locale, "No responses"),
    }
}

fn reward_identity_label(locale: Locale, value: &crate::domain::RewardIdentity) -> String {
    use crate::domain::RewardIdentity::*;
    match value {
        Standard => web_text(locale, "Standard Prover identity").into(),
        Alternate { key_id } => format_web_text(locale, "Alternate identity · {}", key_id),
        Unavailable => web_text(locale, "Unavailable").into(),
        Ambiguous { candidates } => {
            format_web_text(locale, "{} candidate identities", &candidates.to_string())
        }
    }
}

fn operation_table(locale: Locale, operations: Vec<WalletOperationSnapshot>) -> Value {
    table(
        web_text(locale, "Recent wallet operations"),
        vec![
            column("txid", "TxID"),
            column("action", web_text(locale, "Action")),
            column("status", web_text(locale, "Status")),
            column("submitted", web_text(locale, "Submitted at (ms)")),
            column("detail", web_text(locale, "Description")),
        ],
        operations
            .into_iter()
            .map(|operation| {
                json!({
                    "txid": operation.operation_id,
                    "action": wallet_action_label(locale, operation.action),
                    "status": wallet_operation_status_label(locale, operation.phase),
                    "submitted": operation.submitted_unix_ms,
                    "detail": operation.detail.unwrap_or_default(),
                })
            })
            .collect(),
    )
}

fn wallet_action_label(locale: Locale, action: WalletAction) -> &'static str {
    match action {
        WalletAction::Transfer => web_text(locale, "Transfer"),
        WalletAction::PendingTransfer => web_text(locale, "Pending transfer"),
        WalletAction::Accept => web_text(locale, "Accept"),
        WalletAction::Reject => web_text(locale, "Refund"),
    }
}

fn wallet_operation_status_label(locale: Locale, status: WalletOperationStatus) -> &'static str {
    match status {
        WalletOperationStatus::Submitting => web_text(locale, "Submitting"),
        WalletOperationStatus::Submitted => web_text(locale, "Submitted"),
        WalletOperationStatus::Unknown => web_text(locale, "Outcome unknown"),
        WalletOperationStatus::Committed => web_text(locale, "GLOBAL committed"),
        WalletOperationStatus::Rejected => web_text(locale, "GLOBAL rejected"),
    }
}

fn mint_phase_label(locale: Locale, phase: RewardMintPhase) -> &'static str {
    use RewardMintPhase::*;
    match phase {
        Preflighting => web_text(locale, "Preflighting"),
        ReadyToConfirm => web_text(locale, "Awaiting confirmation"),
        ProvingAuthorization => web_text(locale, "Generating authorization proof"),
        AuthorizationPrepared => web_text(locale, "Authorization proof prepared"),
        SubmittingAuthorization => web_text(locale, "Submitting authorization"),
        AwaitingAuthorizationWitness => web_text(locale, "Awaiting authorization Witness"),
        ProvingClaim => web_text(locale, "Generating claim proof"),
        ClaimPrepared => web_text(locale, "Claim proof prepared"),
        SubmittingClaim => web_text(locale, "Submitting claim"),
        ClaimSubmitted => web_text(locale, "Submitted, awaiting on-chain decision"),
        ClaimAccepted => web_text(locale, "Accepted on chain, awaiting Coin visibility"),
        ClaimRejected => web_text(locale, "Rejected on chain, manual recovery available"),
        ClaimConfirmed => web_text(locale, "Claim completed"),
        AuthorizationUnknown => web_text(locale, "Authorization submission unknown"),
        ClaimUnknown => web_text(locale, "Claim submission unknown"),
        RecoverableError => web_text(locale, "Recoverable error, retrying"),
        PermanentError => web_text(locale, "Claim failed"),
    }
}

fn mint_phase_code(phase: RewardMintPhase) -> &'static str {
    use RewardMintPhase::*;
    match phase {
        Preflighting => "preflighting",
        ReadyToConfirm => "ready_to_confirm",
        ProvingAuthorization => "proving_authorization",
        AuthorizationPrepared => "authorization_prepared",
        SubmittingAuthorization => "submitting_authorization",
        AwaitingAuthorizationWitness => "awaiting_authorization_witness",
        ProvingClaim => "proving_claim",
        ClaimPrepared => "claim_prepared",
        SubmittingClaim => "submitting_claim",
        ClaimSubmitted => "claim_submitted",
        ClaimAccepted => "claim_accepted",
        ClaimRejected => "claim_rejected",
        ClaimConfirmed => "claim_confirmed",
        AuthorizationUnknown => "authorization_unknown",
        ClaimUnknown => "claim_unknown",
        RecoverableError => "recoverable_error",
        PermanentError => "permanent_error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{AllocationSnapshot, WorkerExecutionSnapshot, WorkerSnapshot};

    #[test]
    fn idle_workers_are_not_presented_as_running_allocated_provers() {
        let node = NodeSnapshot {
            running_workers: 1,
            workers: vec![WorkerSnapshot {
                core_id: 1,
                filter_hex: String::new(),
                available_storage: 0,
                total_storage: 0,
                manually_managed: false,
                execution: None,
            }],
            ..Default::default()
        };

        let page = prover_view(Locale::English, &node);
        assert_eq!(page["stats"][0]["label"], "Worker processes");
        assert_eq!(page["stats"][2]["label"], "Standby Workers");
        assert_eq!(page["stats"][2]["value"], 1);
        assert_eq!(page["tables"][0]["title"], "Workers");
        assert_eq!(page["tables"][0]["rows"][0]["filter"], "Unallocated");
        assert_eq!(
            page["tables"][0]["rows"][0]["available_storage"],
            "Not reported"
        );
        assert_eq!(page["tables"][0]["rows"][0]["state"], "Standby");
        assert_eq!(page["sections"][0]["title"], "Worker status");
        assert_eq!(page["empty_state"], Value::Null);
    }

    #[test]
    fn transitional_workers_and_allocations_are_not_described_as_fully_idle() {
        let executing = NodeSnapshot {
            running_workers: 1,
            workers: vec![WorkerSnapshot {
                core_id: 1,
                filter_hex: String::new(),
                available_storage: 0,
                total_storage: 0,
                manually_managed: false,
                execution: Some(WorkerExecutionSnapshot {
                    state: "starting".into(),
                    blocker: None,
                    materialized_frame: None,
                    last_advance_unix_ms: 0,
                    observed_unix_ms: 0,
                    awaited_frame: None,
                    recovery: None,
                    seal: None,
                }),
            }],
            ..Default::default()
        };
        let page = prover_view(Locale::English, &executing);
        assert_eq!(page["stats"][2]["value"], 0);
        assert!(page["sections"].as_array().unwrap().is_empty());

        let allocated = NodeSnapshot {
            running_workers: 1,
            allocations: vec![AllocationSnapshot::default()],
            workers: vec![WorkerSnapshot {
                core_id: 1,
                filter_hex: String::new(),
                available_storage: 0,
                total_storage: 0,
                manually_managed: false,
                execution: None,
            }],
            ..Default::default()
        };
        let page = prover_view(Locale::English, &allocated);
        assert!(page["sections"].as_array().unwrap().is_empty());
    }
}
