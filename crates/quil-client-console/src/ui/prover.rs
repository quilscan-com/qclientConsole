use crate::i18n::UiText;
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, ListState, Paragraph},
    Frame,
};
use unicode_width::UnicodeWidthStr;

use crate::{
    allocation::{allocation_lifecycle, ActionHint, ActionTiming, AllocationAction},
    app::App,
    domain::{AllocationSnapshot, AllocationStatus, WorkerSnapshot},
    i18n::{allocation_action_label, t, Locale},
};

use super::{
    chrome::{self, Breakpoint},
    detail::{render_fields, DetailField},
    text::{compact_middle, truncate_cells},
    theme,
};

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let locale = app.locale();
    let detail_open = app.expanded_allocation == Some(app.allocation_index);
    let breakpoint = Breakpoint::from_width(frame.area().width);
    let footer = if frame.area().width < 50 && detail_open {
        t(locale, UiText::ScrollEscQListAActionsCExit)
    } else if breakpoint == Breakpoint::Narrow && detail_open {
        t(locale, UiText::PgUpPgDnScrollEscQListAActionsRRefreshCExit)
    } else if breakpoint == Breakpoint::Narrow {
        t(
            locale,
            UiText::SelectEnterDetailsAActionsRRefreshEscQBackCExit,
        )
    } else {
        t(
            locale,
            UiText::SelectPgUpPgDnDetailsAActionsRRefreshEscQBackCExit,
        )
    };
    let Some(screen) = chrome::screen(frame, "Prover / Allocation", app, footer) else {
        return;
    };

    let areas = prover_layout(frame.area());
    draw_summary(frame, app, areas.summary);
    let allocations = allocations(app);
    if allocations.is_empty() {
        draw_empty(frame, app, areas.content);
        return;
    }

    let selected = app
        .allocation_index
        .min(allocations.len().saturating_sub(1));
    match screen.breakpoint {
        Breakpoint::Wide | Breakpoint::Standard => {
            draw_allocation_list(frame, app, selected, areas.list);
            draw_allocation_detail(frame, app, &allocations[selected], areas.detail);
        }
        Breakpoint::Narrow if detail_open => {
            draw_allocation_detail(frame, app, &allocations[selected], areas.detail);
        }
        Breakpoint::Narrow => draw_allocation_list(frame, app, selected, areas.list),
    }
}

struct ProverAreas {
    summary: Rect,
    content: Rect,
    list: Rect,
    detail: Rect,
}

fn prover_layout(area: Rect) -> ProverAreas {
    let [_, body, _] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(area);
    let [summary, content] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(body);
    let (list, detail) = if Breakpoint::from_width(area.width) == Breakpoint::Narrow {
        (content, content)
    } else {
        let [list, detail] =
            Layout::horizontal([Constraint::Percentage(38), Constraint::Percentage(62)])
                .areas(content);
        (list, detail)
    };
    ProverAreas {
        summary,
        content,
        list,
        detail,
    }
}

struct DetailViewport {
    visible: usize,
    limit: usize,
    overflowing: bool,
}

impl DetailViewport {
    fn new(total: usize, area: Rect) -> Self {
        let height = usize::from(area.height.saturating_sub(2));
        let overflowing = total > height;
        let visible = height.saturating_sub(usize::from(overflowing));
        let limit = if visible > 0 {
            total.saturating_sub(visible)
        } else {
            0
        };
        Self {
            visible,
            limit,
            overflowing,
        }
    }
}

pub(crate) fn detail_scroll_limit(app: &App) -> usize {
    if app.viewport_width < 42
        || app.viewport_height < 12
        || (app.viewport_width < 70 && !app.narrow_prover_detail_open())
    {
        return 0;
    }
    let allocations = allocations(app);
    let Some(allocation) = allocations.get(
        app.allocation_index
            .min(allocations.len().saturating_sub(1)),
    ) else {
        return 0;
    };
    let area = prover_layout(Rect::new(0, 0, app.viewport_width, app.viewport_height)).detail;
    let rows = wrapped_detail_lines(app, allocation, area.width.saturating_sub(2));
    DetailViewport::new(rows.len(), area).limit
}

fn draw_summary(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let summary = app.snapshot.as_ref().map_or_else(
        || t(app.locale(), UiText::NodeDataNotLoaded).to_string(),
        |snapshot| {
            let cores = match (
                snapshot.workers.is_empty(),
                snapshot.worker_info_error.is_some(),
            ) {
                (true, true) => t(app.locale(), UiText::WorkerInformationUnavailable).into(),
                (false, true) => {
                    format!(
                        "{} Core · {}",
                        snapshot.workers.len(),
                        t(app.locale(), UiText::SomeWorkerInformationUnavailable)
                    )
                }
                (_, false) => format!("{} Core", snapshot.workers.len()),
            };
            format!(
                "{} Allocation · {} · Received Frame {}",
                snapshot.allocations.len(),
                cores,
                snapshot.last_received_frame
            )
        },
    );
    let text = app.error.as_ref().map_or(summary, |error| {
        let error = crate::i18n::localize_service_error(app.locale(), error);
        format!("{}: {error}", t(app.locale(), UiText::StaleRefreshFailed))
    });
    frame.render_widget(
        Paragraph::new(truncate_cells(&text, usize::from(area.width))).style(theme::muted()),
        area,
    );
}

fn draw_empty(frame: &mut Frame<'_>, app: &App, area: Rect) {
    frame.render_widget(
        Paragraph::new(if app.loading {
            t(app.locale(), UiText::LoadingAllocationState)
        } else {
            t(app.locale(), UiText::TheNodeReturnedNoAllocationData)
        })
        .style(theme::muted())
        .block(chrome::panel(t(app.locale(), UiText::AllocationList))),
        area,
    );
}

fn draw_allocation_list(frame: &mut Frame<'_>, app: &App, selected: usize, area: Rect) {
    let allocations = allocations(app);
    let width = usize::from(area.width.saturating_sub(4));
    let items = allocations.iter().map(|allocation| {
        let calculated = lifecycle(app, allocation);
        let binding = worker_binding(app, allocation);
        let suffix = format!(
            " {} · {}",
            allocation_label(allocation.status),
            binding.summary(app.locale())
        );
        let filter_width = width.saturating_sub(suffix.width() + 1).max(4);
        ListItem::new(Line::from(vec![
            Span::styled(
                compact_middle(&allocation.filter_hex, filter_width),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(truncate_cells(
                &format!(" {suffix} · {}", calculated.label),
                width.saturating_sub(filter_width),
            )),
        ]))
    });
    let mut state = ListState::default().with_selected(Some(selected));
    frame.render_stateful_widget(
        List::new(items)
            .block(chrome::panel(t(app.locale(), UiText::AllocationList)))
            .highlight_style(theme::focus().add_modifier(Modifier::BOLD))
            .highlight_symbol("› "),
        area,
        &mut state,
    );
}

fn draw_allocation_detail(
    frame: &mut Frame<'_>,
    app: &App,
    allocation: &AllocationSnapshot,
    area: Rect,
) {
    let width = area.width.saturating_sub(2);
    let all_lines = wrapped_detail_lines(app, allocation, width);
    let total = all_lines.len();
    let viewport = DetailViewport::new(total, area);
    let offset = app.prover_detail_scroll.min(viewport.limit);
    let mut lines = all_lines
        .into_iter()
        .skip(offset)
        .take(viewport.visible)
        .collect::<Vec<_>>();
    if viewport.overflowing {
        let scroll_keys = if Breakpoint::from_width(frame.area().width) == Breakpoint::Narrow {
            "↑↓/PgUp/PgDn"
        } else {
            "PgUp/PgDn"
        };
        let range = match app.locale() {
            Locale::English => format!(
                "Rows {}–{} / {} · {scroll_keys} scroll",
                offset + 1,
                offset + viewport.visible,
                total,
            ),
            Locale::TraditionalChinese => format!(
                "第 {}–{} / {} 行 · {scroll_keys} 捲動",
                offset + 1,
                offset + viewport.visible,
                total,
            ),
            Locale::Japanese => format!(
                "{} {}–{} / {} · {scroll_keys}",
                t(Locale::Japanese, UiText::Rows),
                offset + 1,
                offset + viewport.visible,
                total,
            ),
            Locale::French => format!(
                "Lignes {}–{} / {} · {scroll_keys} défiler",
                offset + 1,
                offset + viewport.visible,
                total,
            ),
        };
        lines.push(Line::styled(
            truncate_cells(&range, usize::from(width)),
            theme::muted(),
        ));
    }
    frame.render_widget(
        Paragraph::new(lines).block(chrome::panel(t(app.locale(), UiText::AllocationDetails))),
        area,
    );
}

fn allocation_detail_lines(app: &App, allocation: &AllocationSnapshot) -> Vec<DetailField> {
    let locale = app.locale();
    let calculated = lifecycle(app, allocation);
    let binding = worker_binding(app, allocation);
    let mut lines = vec![DetailField::new(
        t(locale, UiText::RawStatus),
        format!(
            "{} · {} {}",
            allocation_label(allocation.status),
            t(locale, UiText::Calculated),
            calculated.label
        ),
    )];
    lines.extend(core_lines(binding, locale));
    lines.push(DetailField::new(
        t(locale, UiText::Lifecycle),
        format!(
            "{} {} / {} {}",
            t(locale, UiText::Join),
            format_lifecycle_frame(allocation.join_frame_number),
            t(locale, UiText::ConfirmLowercase),
            format_lifecycle_frame(allocation.join_confirm_frame_number),
        ),
    ));
    lines.push(DetailField::new(
        t(locale, UiText::Leave),
        format!(
            "{} / {} {}",
            format_lifecycle_frame(allocation.leave_frame_number),
            t(locale, UiText::ConfirmLowercase),
            format_lifecycle_frame(allocation.leave_confirm_frame_number),
        ),
    ));
    lines.push(DetailField::new(
        t(locale, UiText::LastActive),
        format!(
            "{} · Epoch {}",
            format_lifecycle_frame(allocation.last_active_frame_number),
            allocation.epoch
        ),
    ));
    lines.extend(recovery_lines(binding, locale));
    lines.extend(seal_storage_lines(binding, locale));
    lines.push(DetailField::new(
        t(locale, UiText::NextAction),
        calculated
            .next_action
            .as_ref()
            .map(|hint| format_action_hint(hint, locale, true))
            .unwrap_or_else(|| t(locale, UiText::None).into()),
    ));
    lines.push(DetailField::new(
        t(locale, UiText::DefaultAction),
        calculated
            .default_action
            .as_ref()
            .map(|hint| format_action_hint(hint, locale, false))
            .unwrap_or_else(|| t(locale, UiText::None).into()),
    ));
    lines.push(DetailField::new("Filter", allocation.filter_hex.clone()));
    lines
}

#[cfg(test)]
pub(super) fn detail_rows_for_test(
    app: &App,
    allocation: &AllocationSnapshot,
    width: u16,
) -> Vec<Line<'static>> {
    wrapped_detail_lines(app, allocation, width)
}

fn core_lines(binding: WorkerBinding<'_>, locale: Locale) -> Vec<DetailField> {
    match binding {
        WorkerBinding::Bound(worker) => {
            let management = if worker.manually_managed {
                t(locale, UiText::ManualManagement)
            } else {
                t(locale, UiText::AutomaticManagement)
            };
            let mut lines = vec![DetailField::new(
                "Core",
                format!("{} · {management}", worker.core_id),
            )];
            match worker.execution.as_ref() {
                Some(execution) => {
                    let mut summary = format!(
                        "{} {} · Frame {} · {} {}",
                        t(locale, UiText::RuntimeState),
                        execution_label(&execution.state, locale),
                        execution
                            .materialized_frame
                            .map_or_else(|| "—".into(), |frame| frame.to_string()),
                        t(locale, UiText::Age),
                        execution_age(
                            execution.last_advance_unix_ms,
                            execution.observed_unix_ms,
                            locale
                        )
                    );
                    if let Some(awaited_frame) = execution.awaited_frame {
                        summary.push_str(&format!(
                            " · {} Frame {awaited_frame}",
                            t(locale, UiText::WaitingFor)
                        ));
                    }
                    lines.push(DetailField::new(t(locale, UiText::Execution), summary));
                    if let Some(blocker) = execution.blocker.as_deref() {
                        lines.push(DetailField::new(
                            t(locale, UiText::Blocker),
                            blocker_label(blocker, locale),
                        ));
                    }
                }
                None => lines.push(DetailField::new(
                    t(locale, UiText::Execution),
                    t(locale, UiText::NotObserved),
                )),
            }
            lines
        }
        WorkerBinding::Unknown => vec![DetailField::new(
            "Core",
            t(locale, UiText::BindingUnknownWorkerInformationUnavailable),
        )],
        WorkerBinding::Unbound => vec![DetailField::new(
            "Core",
            t(locale, UiText::NoCoreBoundNoWorkerData),
        )],
    }
}

fn blocker_label(blocker: &str, locale: Locale) -> &str {
    match blocker.trim().to_ascii_lowercase().as_str() {
        "waiting for transport" => t(locale, UiText::WaitingForTransport),
        "waiting for global state" => t(locale, UiText::WaitingForGlobalState),
        _ => blocker,
    }
}

fn recovery_lines(binding: WorkerBinding<'_>, locale: Locale) -> Vec<DetailField> {
    let WorkerBinding::Bound(worker) = binding else {
        return vec![DetailField::new(
            "Recovery",
            match binding {
                WorkerBinding::Unknown => t(locale, UiText::WorkerInformationUnavailable),
                WorkerBinding::Unbound => t(locale, UiText::NoWorkerData),
                WorkerBinding::Bound(_) => unreachable!(),
            },
        )];
    };
    let Some(recovery) = worker
        .execution
        .as_ref()
        .and_then(|execution| execution.recovery.as_ref())
    else {
        return vec![DetailField::new("Recovery", t(locale, UiText::NotObserved))];
    };
    let failures = if recovery.failures_by_class.is_empty() {
        t(locale, UiText::None).into()
    } else {
        recovery
            .failures_by_class
            .iter()
            .map(|(class, count)| format!("{class}×{count}"))
            .collect::<Vec<_>>()
            .join(",")
    };
    vec![
        DetailField::new(
            "Recovery",
            format!(
                "{} · {}/{}",
                recovery_phase(&recovery.phase, locale),
                recovery.installed_leaves,
                recovery.planned_leaves,
            ),
        ),
        DetailField::new(
            t(locale, UiText::RecoverySource),
            format!(
                "{} · {} {}",
                recovery
                    .source
                    .as_deref()
                    .unwrap_or(t(locale, UiText::NotReturned)),
                t(locale, UiText::RecoveryAttempts),
                recovery.attempts
            ),
        ),
        DetailField::new(
            t(locale, UiText::RecoveryResult),
            recovery
                .last_outcome
                .as_deref()
                .unwrap_or(t(locale, UiText::NotReturned)),
        ),
        DetailField::new(
            t(locale, UiText::RecoveryFailures),
            format!(
                "{} · {} {}",
                failures,
                t(locale, UiText::ErrorLowercase),
                recovery
                    .last_error
                    .as_deref()
                    .unwrap_or(t(locale, UiText::None))
            ),
        ),
    ]
}

fn seal_storage_lines(binding: WorkerBinding<'_>, locale: Locale) -> Vec<DetailField> {
    match binding {
        WorkerBinding::Bound(worker) => {
            let seal = worker
                .execution
                .as_ref()
                .and_then(|execution| execution.seal.as_ref())
                .map_or_else(
                    || t(locale, UiText::NotObserved).into(),
                    |seal| {
                        format!(
                            "{} · generation {} · checkpoint {} · view {} · global {} · {} {}",
                            if seal.is_recorded() {
                                t(locale, UiText::Recorded)
                            } else {
                                t(locale, UiText::NotRecordedLowercase)
                            },
                            seal.generation,
                            seal.checkpoint,
                            seal.view,
                            seal.global_executed,
                            t(locale, UiText::Submissions),
                            seal.submissions
                        )
                    },
                );
            vec![
                DetailField::new("Seal", seal),
                DetailField::new(
                    t(locale, UiText::Storage),
                    format_bytes(worker.available_storage, worker.total_storage, locale),
                ),
            ]
        }
        WorkerBinding::Unknown => vec![DetailField::new(
            t(locale, UiText::SealStorage),
            t(locale, UiText::WorkerInformationUnavailable),
        )],
        WorkerBinding::Unbound => vec![DetailField::new(
            t(locale, UiText::SealStorage),
            t(locale, UiText::NoWorkerData),
        )],
    }
}

/// Generates every detail row before the viewport selects the visible portion.
fn wrapped_detail_lines(
    app: &App,
    allocation: &AllocationSnapshot,
    width: u16,
) -> Vec<Line<'static>> {
    render_fields(allocation_detail_lines(app, allocation), width)
}

fn allocations(app: &App) -> Vec<AllocationSnapshot> {
    let mut allocations = app
        .snapshot
        .as_ref()
        .map(|snapshot| snapshot.allocations.clone())
        .unwrap_or_default();
    for operation in app
        .prover_operations
        .items
        .iter()
        .filter(|operation| operation.is_pending())
    {
        let status = match &operation.action {
            crate::domain::ProverAction::Join { .. } => AllocationStatus::Joining,
            crate::domain::ProverAction::Leave { .. } => AllocationStatus::Leaving,
            _ => continue,
        };
        if let Some(allocation) = allocations
            .iter_mut()
            .find(|allocation| allocation.filter_hex == operation.filter_hex)
        {
            allocation.status = status;
        } else {
            allocations.push(AllocationSnapshot {
                filter_hex: operation.filter_hex.clone(),
                status,
                ..Default::default()
            });
        }
    }
    allocations
}

#[derive(Clone, Copy)]
enum WorkerBinding<'a> {
    Bound(&'a WorkerSnapshot),
    Unknown,
    Unbound,
}

impl WorkerBinding<'_> {
    fn summary(self, locale: Locale) -> String {
        match self {
            Self::Bound(worker) => format!("Core {}", worker.core_id),
            Self::Unknown => t(locale, UiText::CoreBindingUnknown).into(),
            Self::Unbound => t(locale, UiText::NoCoreBound).into(),
        }
    }
}

fn worker_binding<'a>(app: &'a App, allocation: &AllocationSnapshot) -> WorkerBinding<'a> {
    let Some(snapshot) = app.snapshot.as_ref() else {
        return WorkerBinding::Unknown;
    };
    if let Some(worker) = snapshot
        .workers
        .iter()
        .find(|worker| worker.filter_hex == allocation.filter_hex)
    {
        WorkerBinding::Bound(worker)
    } else if let Some(worker) = app
        .prover_operations
        .items
        .iter()
        .filter(|operation| operation.is_pending())
        .find(|operation| operation.filter_hex == allocation.filter_hex)
        .and_then(|operation| operation.core_id)
        .and_then(|core_id| {
            snapshot
                .workers
                .iter()
                .find(|worker| worker.core_id == core_id)
        })
    {
        WorkerBinding::Bound(worker)
    } else if snapshot.worker_info_error.is_some() {
        WorkerBinding::Unknown
    } else {
        WorkerBinding::Unbound
    }
}

fn lifecycle(app: &App, allocation: &AllocationSnapshot) -> crate::allocation::AllocationLifecycle {
    let snapshot = app.snapshot.as_ref();
    allocation_lifecycle(
        allocation,
        snapshot.map_or(0, |snapshot| snapshot.last_received_frame),
        snapshot.and_then(|snapshot| snapshot.epoch.as_ref()),
    )
}

fn allocation_label(status: AllocationStatus) -> &'static str {
    match status {
        AllocationStatus::Joining => "JOINING",
        AllocationStatus::Active => "ACTIVE",
        AllocationStatus::Paused => "PAUSED",
        AllocationStatus::Leaving => "LEAVING",
        AllocationStatus::Rejected => "REJECTED",
        AllocationStatus::Kicked => "KICKED",
        AllocationStatus::Historic => "HISTORIC",
        AllocationStatus::Unknown => "UNKNOWN",
    }
}

fn format_action_hint(hint: &ActionHint, locale: Locale, localized: bool) -> String {
    let actions = hint
        .actions
        .iter()
        .map(|action| action_label(*action, locale, localized))
        .collect::<Vec<_>>()
        .join("/");
    match hint.timing {
        ActionTiming::Now => format!("{actions} · {}", t(locale, UiText::Now)),
        ActionTiming::At {
            frame,
            frames_remaining,
        } => format!("{actions} @ {frame} (+{frames_remaining})"),
        ActionTiming::Unknown => format!("{actions} · {}", t(locale, UiText::TimeUnknown)),
    }
}

fn action_label(action: AllocationAction, locale: Locale, localized: bool) -> &'static str {
    if !localized {
        return match action {
            AllocationAction::Reject => "reject",
            AllocationAction::Confirm => "confirm",
            AllocationAction::Pause => "pause",
            AllocationAction::Resume => "resume",
            AllocationAction::Leave => "leave",
            AllocationAction::Renew => "renew",
            AllocationAction::Activate => "activate",
            AllocationAction::Depart => "depart",
            AllocationAction::Expire => "expire",
        };
    }
    allocation_action_label(locale, action)
}

fn execution_label(state: &str, locale: Locale) -> &str {
    match state.to_ascii_lowercase().as_str() {
        "starting" => t(locale, UiText::Starting),
        "running" => t(locale, UiText::Running),
        "blocked" => t(locale, UiText::Blocked),
        "stopped" => t(locale, UiText::Stopped),
        "waiting" => t(locale, UiText::Waiting),
        _ => state,
    }
}

fn execution_age(last_advance_unix_ms: u64, observed_unix_ms: u64, locale: Locale) -> String {
    if last_advance_unix_ms == 0 || observed_unix_ms < last_advance_unix_ms {
        return t(locale, UiText::Unknown).into();
    }
    format!(
        "{:.1}s",
        observed_unix_ms.saturating_sub(last_advance_unix_ms) as f64 / 1_000.0
    )
}

fn recovery_phase(phase: &str, locale: Locale) -> &str {
    match phase {
        "waiting_for_permit" => t(locale, UiText::WaitingForRecoveryPermit),
        "replaying_frames" => t(locale, UiText::ReplayingShardFrames),
        "fetching_checkpoint" => t(locale, UiText::FetchingCheckpoint),
        "installing_leaves" => t(locale, UiText::InstallingStateData),
        "installing_checkpoint" => t(locale, UiText::InstallingCheckpoint),
        "idle" => t(locale, UiText::Idle),
        _ => phase,
    }
}

fn format_bytes(available: u64, total: u64, locale: Locale) -> String {
    if total == 0 {
        return t(locale, UiText::NotReturnedByNode).into();
    }
    format!(
        "{} / {} {}",
        human_bytes(available),
        human_bytes(total),
        t(locale, UiText::AvailableLowercase)
    )
}

fn format_lifecycle_frame(frame: u64) -> String {
    if frame == 0 {
        "—".into()
    } else {
        frame.to_string()
    }
}

fn human_bytes(value: u64) -> String {
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    if value as f64 >= GIB {
        format!("{:.1} GiB", value as f64 / GIB)
    } else if value as f64 >= MIB {
        format!("{:.1} MiB", value as f64 / MIB)
    } else {
        format!("{value} B")
    }
}

#[cfg(test)]
mod tests {
    use super::{allocations, format_lifecycle_frame};
    use crate::{
        app::App,
        domain::{AllocationSnapshot, AllocationStatus, NodeSnapshot, ProverAction},
    };

    #[test]
    fn lifecycle_frame_formatter_treats_zero_as_not_returned() {
        assert_eq!(format_lifecycle_frame(0), "—");
        assert_eq!(format_lifecycle_frame(42), "42");
    }

    #[test]
    fn pending_operations_overlay_joining_and_leaving_immediately() {
        let mut app = App {
            snapshot: Some(NodeSnapshot {
                allocations: vec![AllocationSnapshot {
                    filter_hex: "11".repeat(32),
                    status: AllocationStatus::Active,
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        app.prover_operations
            .enqueue(
                ProverAction::Leave {
                    filters: vec![vec![0x11; 32]],
                },
                Some(1),
                1,
            )
            .unwrap();
        app.prover_operations
            .enqueue(
                ProverAction::Join {
                    filters: vec![vec![0x22; 32]],
                    delegate: Vec::new(),
                    worker_ids: vec![2],
                },
                Some(2),
                1,
            )
            .unwrap();

        let rows = allocations(&app);

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].status, AllocationStatus::Leaving);
        assert_eq!(rows[1].status, AllocationStatus::Joining);
    }
}
