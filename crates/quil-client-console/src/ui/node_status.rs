use crate::i18n::UiText;
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::{
    app::App,
    domain::{AllocationStatus, NodeSnapshot},
    i18n::{localize_service_error, t, Locale},
};

use super::{
    chrome,
    text::{compact_middle, truncate_cells},
    theme,
};

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let locale = app.locale();
    let Some(screen) = chrome::screen(
        frame,
        t(locale, UiText::NodeStatus),
        app,
        t(locale, UiText::RRefreshEscQBackCtrlCExit),
    ) else {
        return;
    };
    let panel = chrome::panel(t(locale, UiText::NodeFactsAndActionableAlerts));
    let inner = panel.inner(screen.body);
    frame.render_widget(panel, screen.body);
    let body = if let Some(error) = &app.error {
        let error = localize_service_error(locale, error);
        let status = if app.snapshot.is_some() {
            format!("{}: {error}", t(locale, UiText::StaleRefreshFailed))
        } else {
            format!("{}: {error}", t(locale, UiText::LoadFailedSentenceCase))
        };
        frame.render_widget(
            Paragraph::new(truncate_cells(&status, inner.width.into())).style(theme::attention()),
            Rect::new(inner.x, inner.y, inner.width, 1),
        );
        Rect::new(
            inner.x,
            inner.y + 1,
            inner.width,
            inner.height.saturating_sub(1),
        )
    } else {
        inner
    };
    match &app.snapshot {
        Some(snapshot) if body.height < 15 => draw_narrow(frame, snapshot, body, locale),
        Some(snapshot) if screen.breakpoint != chrome::Breakpoint::Narrow => {
            draw_wide(frame, snapshot, body, locale)
        }
        Some(snapshot) => draw_narrow(frame, snapshot, body, locale),
        None => frame.render_widget(
            Paragraph::new(if app.loading {
                t(locale, UiText::LoadingNodeStatusMessage)
            } else {
                t(locale, UiText::NodeStatusUnavailablePressRToRefresh)
            }),
            body,
        ),
    }
}

fn draw_wide(frame: &mut Frame<'_>, snapshot: &NodeSnapshot, area: Rect, locale: Locale) {
    let counts = AllocationCounts::from_snapshot(snapshot);
    let alerts = derive_node_alerts(snapshot, &counts, locale);
    let columns =
        Layout::horizontal([Constraint::Percentage(52), Constraint::Percentage(48)]).split(area);
    draw_rows(frame, detail_lines(snapshot, locale), columns[0]);

    let right = Layout::vertical([Constraint::Length(6), Constraint::Min(5)]).split(columns[1]);
    draw_rows(
        frame,
        allocation_lines(snapshot.allocations.is_empty(), &counts, locale),
        right[0],
    );
    draw_alerts(frame, &alerts, right[1], locale);
}

fn draw_narrow(frame: &mut Frame<'_>, snapshot: &NodeSnapshot, area: Rect, locale: Locale) {
    let counts = AllocationCounts::from_snapshot(snapshot);
    let alerts = derive_node_alerts(snapshot, &counts, locale);
    let content_width = usize::from(area.width);
    let mut lines = compact_status_lines(snapshot, &counts, &alerts, content_width, locale);
    if area.height >= 15 {
        lines.extend([
            fact_line(
                "Reachable / RPC",
                format!(
                    "{} · {}",
                    if snapshot.reachable {
                        t(locale, UiText::Reachable)
                    } else {
                        t(locale, UiText::Unreachable)
                    },
                    if snapshot.connected {
                        t(locale, UiText::Connected)
                    } else {
                        t(locale, UiText::Disconnected)
                    }
                ),
            ),
            fact_line(
                "Seniority",
                snapshot
                    .peer_seniority
                    .as_deref()
                    .unwrap_or(t(locale, UiText::Unavailable)),
            ),
            fact_line("Peer Score", snapshot.peer_score.to_string()),
            fact_line("Node RPC", snapshot.rpc_address.clone()),
        ]);
    }

    draw_rows(frame, lines, area);
}

fn compact_status_lines(
    snapshot: &NodeSnapshot,
    counts: &AllocationCounts,
    alerts: &[NodeAlert],
    content_width: usize,
    locale: Locale,
) -> Vec<Line<'static>> {
    let alert_labels = alerts
        .iter()
        .map(|alert| alert.compact.as_str())
        .collect::<Vec<_>>();
    let mut lines = vec![
        Line::from(format!(
            "Frame Lag {} · Rx {}/Head {}",
            frame_lag(snapshot),
            snapshot.last_received_frame,
            snapshot.last_global_head_frame
        )),
        Line::from(
            match snapshot
                .epoch
                .as_ref()
                .filter(|epoch| epoch.length_frames > 0)
            {
                Some(epoch) => format!(
                    "Epoch {} · {} {} · {} {}/{} {}",
                    epoch.current,
                    t(locale, UiText::Boundary),
                    epoch
                        .current
                        .saturating_add(1)
                        .saturating_mul(epoch.length_frames),
                    t(locale, UiText::RunningLowercase),
                    snapshot.running_workers,
                    t(locale, UiText::AllocatedLowercase),
                    snapshot.allocated_workers
                ),
                None => format!(
                    "Epoch {} · {} {}/{} {}",
                    t(locale, UiText::UnavailableLowercase),
                    t(locale, UiText::RunningLowercase),
                    snapshot.running_workers,
                    t(locale, UiText::AllocatedLowercase),
                    snapshot.allocated_workers
                ),
            },
        ),
    ];
    if snapshot.allocations.is_empty() {
        lines.extend([
            Line::from(t(locale, UiText::NoCurrentAllocation)),
            Line::from("Joining 0 · Leaving 0 · Kicked 0"),
            Line::from("Historic 0 · Unknown 0"),
        ]);
    } else {
        lines.extend([
            Line::from(format!(
                "Active {} · Paused {} · Rejected {}",
                counts.active, counts.paused, counts.rejected
            )),
            Line::from(format!(
                "Joining {} · Leaving {} · Kicked {}",
                counts.joining, counts.leaving, counts.kicked
            )),
            Line::from(format!(
                "Historic {} · Unknown {}",
                counts.historic, counts.unknown
            )),
        ]);
    }

    for (index, row) in compact_alert_rows(&alert_labels, alerts.len(), content_width, locale)
        .into_iter()
        .enumerate()
    {
        lines.push(if index == 0 {
            Line::from(Span::styled(row, alert_style(alert_labels.is_empty())))
        } else {
            Line::from(row)
        });
    }
    lines
}

fn draw_alerts(frame: &mut Frame<'_>, alerts: &[NodeAlert], area: Rect, locale: Locale) {
    let lines = if alerts.is_empty() {
        vec![Line::from(Span::styled(
            t(locale, UiText::OKNoCurrentAlerts),
            theme::success(),
        ))]
    } else {
        alerts
            .iter()
            .map(|alert| {
                Line::from(vec![
                    Span::styled("! ", theme::attention()),
                    Span::raw(alert.full.clone()),
                ])
            })
            .collect()
    };
    draw_rows(frame, lines, area);
}

fn detail_lines(snapshot: &NodeSnapshot, locale: Locale) -> Vec<Line<'static>> {
    vec![
        fact_line(
            t(locale, UiText::Configuration),
            snapshot.config_name.clone(),
        ),
        fact_line("Node RPC", snapshot.rpc_address.clone()),
        fact_line(
            t(locale, UiText::RPCConnection),
            if snapshot.connected {
                t(locale, UiText::Connected)
            } else {
                t(locale, UiText::Disconnected)
            },
        ),
        fact_line(
            "Reachable",
            if snapshot.reachable {
                t(locale, UiText::Reachable)
            } else {
                t(locale, UiText::Unreachable)
            },
        ),
        fact_line("Peer ID", compact_middle(&snapshot.peer_id, 30)),
        fact_line(
            "Seniority",
            snapshot
                .peer_seniority
                .as_deref()
                .unwrap_or(t(locale, UiText::Unavailable)),
        ),
        fact_line("Peer Score", snapshot.peer_score.to_string()),
        fact_line(
            "Frame",
            format!(
                "received {} · head {}",
                snapshot.last_received_frame, snapshot.last_global_head_frame
            ),
        ),
        fact_line("Frame Lag", frame_lag(snapshot).to_string()),
        fact_line(
            t(locale, UiText::EpochBoundary),
            epoch_summary(snapshot, locale),
        ),
        fact_line(
            "Worker (RPC)",
            format!(
                "{} {} · {} {}",
                t(locale, UiText::Running),
                snapshot.running_workers,
                t(locale, UiText::Allocated),
                snapshot.allocated_workers
            ),
        ),
    ]
}

fn allocation_lines(empty: bool, counts: &AllocationCounts, locale: Locale) -> Vec<Line<'static>> {
    if empty {
        return vec![Line::from(t(locale, UiText::NoCurrentAllocation))];
    }
    vec![
        Line::from(format!(
            "Active {} · Joining {} · Leaving {}",
            counts.active, counts.joining, counts.leaving
        )),
        Line::from(format!(
            "Paused {} · Rejected {} · Kicked {}",
            counts.paused, counts.rejected, counts.kicked
        )),
        Line::from(format!(
            "Historic {} · Unknown {}",
            counts.historic, counts.unknown
        )),
    ]
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct AllocationCounts {
    unknown: usize,
    joining: usize,
    active: usize,
    paused: usize,
    leaving: usize,
    rejected: usize,
    kicked: usize,
    historic: usize,
}

impl AllocationCounts {
    fn from_snapshot(snapshot: &NodeSnapshot) -> Self {
        let mut counts = Self::default();
        for allocation in &snapshot.allocations {
            let count = match allocation.status {
                AllocationStatus::Unknown => &mut counts.unknown,
                AllocationStatus::Joining => &mut counts.joining,
                AllocationStatus::Active => &mut counts.active,
                AllocationStatus::Paused => &mut counts.paused,
                AllocationStatus::Leaving => &mut counts.leaving,
                AllocationStatus::Rejected => &mut counts.rejected,
                AllocationStatus::Kicked => &mut counts.kicked,
                AllocationStatus::Historic => &mut counts.historic,
            };
            *count += 1;
        }
        counts
    }

    fn attention(self) -> usize {
        self.unknown + self.paused + self.rejected + self.kicked
    }
}

fn epoch_summary(snapshot: &NodeSnapshot, locale: Locale) -> String {
    let Some(epoch) = snapshot
        .epoch
        .as_ref()
        .filter(|epoch| epoch.length_frames > 0)
    else {
        return t(locale, UiText::Unavailable).into();
    };
    let next_boundary = epoch
        .current
        .saturating_add(1)
        .saturating_mul(epoch.length_frames);
    format!(
        "Epoch {} · {} {} · {} {}",
        epoch.current,
        t(locale, UiText::Length),
        epoch.length_frames,
        t(locale, UiText::NextBoundary),
        next_boundary
    )
}

pub(super) fn frame_lag(snapshot: &NodeSnapshot) -> u64 {
    snapshot
        .last_global_head_frame
        .saturating_sub(snapshot.last_received_frame)
}

#[cfg(test)]
fn alert_messages(snapshot: &NodeSnapshot) -> Vec<String> {
    node_alerts(snapshot)
        .into_iter()
        .map(|alert| alert.full)
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NodeAlert {
    full: String,
    compact: String,
}

#[cfg(test)]
pub(super) fn node_alerts(snapshot: &NodeSnapshot) -> Vec<NodeAlert> {
    let counts = AllocationCounts::from_snapshot(snapshot);
    derive_node_alerts(snapshot, &counts, Locale::TraditionalChinese)
}

fn derive_node_alerts(
    snapshot: &NodeSnapshot,
    counts: &AllocationCounts,
    locale: Locale,
) -> Vec<NodeAlert> {
    let mut alerts = Vec::new();
    let lag = frame_lag(snapshot);
    if lag > 0 {
        alerts.push(NodeAlert {
            full: format!(
                "{} {lag}: {}",
                t(locale, UiText::FrameLag),
                t(locale, UiText::CheckSynchronizationAndRefresh)
            ),
            compact: format!("FrameLag{lag}"),
        });
    }
    if !snapshot.reachable {
        alerts.push(NodeAlert {
            full: t(locale, UiText::NodeUnreachableCheckNetworkAndRPCThenRefresh).into(),
            compact: t(locale, UiText::Unreachable).into(),
        });
    }

    let blocked_workers = actionable_blocked_worker_count(snapshot);
    if blocked_workers > 0 {
        alerts.push(NodeAlert {
            full: format!(
                "{} {blocked_workers}: {}",
                t(locale, UiText::BlockedWorkers),
                t(locale, UiText::OpenProverAllocationForTheBlocker)
            ),
            compact: format!("{} {blocked_workers}", t(locale, UiText::WorkerBlocked)),
        });
    }

    let needs_attention = counts.attention();
    if needs_attention > 0 {
        alerts.push(NodeAlert {
            full: format!(
                "{} {needs_attention}: Paused {} · Rejected {} · Kicked {} · Unknown {}",
                t(locale, UiText::AllocationsNeedAttention),
                counts.paused,
                counts.rejected,
                counts.kicked,
                counts.unknown
            ),
            compact: format!(
                "{} {needs_attention}",
                t(locale, UiText::AllocationAttention)
            ),
        });
    }
    if let Some(error) = snapshot
        .worker_info_error
        .as_deref()
        .filter(|error| !error.trim().is_empty())
    {
        alerts.push(NodeAlert {
            full: format!(
                "{}: {error}; {}",
                t(locale, UiText::WorkerInformationUnavailable),
                t(locale, UiText::CheckRPCAndRefresh)
            ),
            compact: t(locale, UiText::WorkerInfoUnavailable).into(),
        });
    }
    alerts
}

fn compact_alert_rows(
    labels: &[&str],
    alert_count: usize,
    content_width: usize,
    locale: Locale,
) -> Vec<String> {
    let mut rows = Vec::with_capacity(2);
    let mut next = 0;
    let mut first = format!("{} {alert_count}", t(locale, UiText::Alerts));
    while let Some(label) = labels.get(next) {
        let candidate = append_compact_label(&first, label);
        if Line::from(candidate.as_str()).width() > content_width {
            break;
        }
        first = candidate;
        next += 1;
    }
    rows.push(first);

    if next < labels.len() {
        let row_start = next;
        let mut second = String::new();
        while let Some(label) = labels.get(next) {
            let candidate = append_compact_label(&second, label);
            let remaining = labels.len() - next - 1;
            let reserved = if remaining == 0 {
                candidate.clone()
            } else {
                append_compact_label(
                    &candidate,
                    &format!("{} {remaining}", t(locale, UiText::Plus)),
                )
            };
            if Line::from(reserved.as_str()).width() > content_width {
                break;
            }
            second = candidate;
            next += 1;
        }

        if next < labels.len() {
            let marker = format!("{} {}", t(locale, UiText::Plus), labels.len() - next);
            let candidate = append_compact_label(&second, &marker);
            if Line::from(candidate.as_str()).width() <= content_width {
                second = candidate;
            } else {
                second = format!("{} {}", t(locale, UiText::Plus), labels.len() - row_start);
            }
        }
        rows.push(second);
    }
    rows
}

fn append_compact_label(row: &str, label: &str) -> String {
    if row.is_empty() {
        label.into()
    } else {
        format!("{row} · {label}")
    }
}

fn draw_rows(frame: &mut Frame<'_>, lines: Vec<Line<'static>>, area: Rect) {
    let lines = lines
        .into_iter()
        .map(|line| {
            if line.width() <= usize::from(area.width) {
                return line;
            }
            let style = line
                .spans
                .first()
                .map_or(theme::normal(), |span| span.style);
            Line::from(Span::styled(
                truncate_cells(&line.to_string(), area.width.into()),
                style,
            ))
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines), area);
}

fn actionable_blocked_worker_count(snapshot: &NodeSnapshot) -> usize {
    snapshot
        .workers
        .iter()
        .filter(|worker| {
            worker.execution.as_ref().is_some_and(|execution| {
                execution.state.trim().eq_ignore_ascii_case("blocked")
                    && !execution
                        .blocker
                        .as_deref()
                        .is_some_and(is_successor_handoff)
            })
        })
        .count()
}

fn is_successor_handoff(blocker: &str) -> bool {
    let mut words = blocker.split_whitespace();
    words
        .next()
        .is_some_and(|word| word.eq_ignore_ascii_case("awaiting"))
        && words
            .next()
            .is_some_and(|word| word.eq_ignore_ascii_case("successor"))
        && words.next().is_none()
}

fn fact_line(label: &str, value: impl Into<String>) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label}  "), theme::muted()),
        Span::raw(value.into()),
    ])
}

fn alert_style(ok: bool) -> Style {
    if ok {
        theme::success()
    } else {
        theme::attention()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        AllocationSnapshot, AllocationStatus, WorkerExecutionSnapshot, WorkerSnapshot,
    };

    fn snapshot_with_worker(state: &str, blocker: Option<&str>) -> NodeSnapshot {
        NodeSnapshot {
            reachable: true,
            workers: vec![WorkerSnapshot {
                core_id: 1,
                filter_hex: String::new(),
                available_storage: 0,
                total_storage: 0,
                manually_managed: false,
                execution: Some(WorkerExecutionSnapshot {
                    state: state.into(),
                    blocker: blocker.map(str::to_owned),
                    materialized_frame: None,
                    last_advance_unix_ms: 0,
                    observed_unix_ms: 0,
                    awaited_frame: None,
                    recovery: None,
                    seal: None,
                }),
            }],
            ..Default::default()
        }
    }

    fn has_blocked_worker_alert(snapshot: &NodeSnapshot) -> bool {
        alert_messages(snapshot)
            .iter()
            .any(|alert| alert.contains("Worker 受阻"))
    }

    fn five_alert_snapshot() -> NodeSnapshot {
        let mut snapshot = snapshot_with_worker("blocked", Some("storage unavailable"));
        snapshot.last_global_head_frame = 10;
        snapshot.reachable = false;
        snapshot.allocations = vec![AllocationSnapshot {
            status: AllocationStatus::Unknown,
            ..Default::default()
        }];
        snapshot.worker_info_error = Some("worker info rpc unavailable".into());
        snapshot
    }

    #[test]
    fn actionable_blocked_worker_emits_an_alert() {
        let snapshot = snapshot_with_worker("BLOCKED", Some("storage unavailable"));

        assert!(has_blocked_worker_alert(&snapshot));
    }

    #[test]
    fn waiting_worker_with_a_blocker_is_not_reported_as_blocked() {
        let snapshot = snapshot_with_worker("waiting", Some("awaited frame 42"));

        assert!(!has_blocked_worker_alert(&snapshot));
    }

    #[test]
    fn successor_handoff_is_not_reported_as_actionable_blocked() {
        let snapshot = snapshot_with_worker("  BlOcKeD  ", Some("  AWAITING   successor  "));

        assert!(!has_blocked_worker_alert(&snapshot));
    }

    #[test]
    fn clean_running_worker_has_no_blocked_alert() {
        let snapshot = snapshot_with_worker("running", None);

        assert!(!has_blocked_worker_alert(&snapshot));
    }

    #[test]
    fn compact_source_rows_fit_the_minimum_content_width() {
        let snapshot = five_alert_snapshot();
        let counts = AllocationCounts::from_snapshot(&snapshot);
        let alerts = derive_node_alerts(&snapshot, &counts, Locale::TraditionalChinese);

        let lines =
            compact_status_lines(&snapshot, &counts, &alerts, 40, Locale::TraditionalChinese);

        assert_eq!(alerts.len(), 5);
        assert!(
            lines.iter().all(|line| line.width() <= 40),
            "compact source rows exceed the 40-cell budget: {lines:?}"
        );
    }

    #[test]
    fn typed_alerts_keep_all_five_full_and_compact_categories_aligned() {
        let snapshot = five_alert_snapshot();
        let counts = AllocationCounts::from_snapshot(&snapshot);

        let alerts = derive_node_alerts(&snapshot, &counts, Locale::TraditionalChinese);

        assert_eq!(alerts.len(), 5);
        for alert in &alerts {
            assert!(!alert.full.is_empty());
            assert!(!alert.compact.is_empty());
        }
        let pairs = alerts
            .iter()
            .map(|alert| (alert.full.as_str(), alert.compact.as_str()))
            .collect::<Vec<_>>();
        assert!(pairs[0].0.contains("Frame 落後"));
        assert!(pairs[0].1.contains("FrameLag"));
        assert!(pairs[1].0.contains("節點無法連線"));
        assert!(pairs[1].1.contains("無法連線"));
        assert!(pairs[2].0.contains("Worker 受阻"));
        assert!(pairs[2].1.contains("Worker受阻"));
        assert!(pairs[3].0.contains("Allocation 需處理"));
        assert!(pairs[3].1.contains("Allocation需處理"));
        assert!(pairs[4].0.contains("Worker 資訊無法使用"));
        assert!(pairs[4].1.contains("Worker資訊無法使用"));
    }
}
