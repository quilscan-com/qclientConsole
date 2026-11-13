use crate::i18n::UiText;
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, ListState, Paragraph},
    Frame,
};

use super::{
    chrome::{self, Breakpoint},
    text::{compact_middle, truncate_cells, wrap_cells},
    theme,
};
use crate::{
    app::App,
    domain::{HealthConditionSnapshot, HealthSnapshot, HealthWorkerSnapshot},
    i18n::{t, Locale},
};

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let locale = app.locale();
    let footer = if frame.area().width < 70 {
        t(
            locale,
            UiText::PgUpPgDnConditionsSelectEnterDetailsRRefreshEscQBackCtrlCQuit,
        )
    } else {
        t(
            locale,
            UiText::PgUpPgDnConditionsSelectCoreRRefreshEscQBackCtrlCQuit,
        )
    };
    let Some(screen) = chrome::screen(frame, t(locale, UiText::NodeHealthReport), app, footer)
    else {
        return;
    };
    let panel = chrome::panel(t(locale, UiText::HealthReport));
    let inner = panel.inner(screen.body);
    frame.render_widget(panel, screen.body);
    let status = if app.health.in_flight {
        if app.health.snapshot.is_some() {
            t(locale, UiText::RefreshingStaleReport).into()
        } else {
            t(locale, UiText::LoadingHealthReport).into()
        }
    } else if let Some(error) = &app.health.error {
        let error = crate::i18n::localize_service_error(locale, error);
        if app.health.snapshot.is_some() {
            format!("{}: {error}", t(locale, UiText::StaleReportRefreshFailed))
        } else {
            format!("{}: {error}", t(locale, UiText::LoadFailedSentenceCase))
        }
    } else {
        app.health.target.localized_label(locale)
    };
    frame.render_widget(
        Paragraph::new(truncate_cells(&status, inner.width.into())).style(
            if app.health.error.is_some() {
                theme::failure()
            } else {
                theme::muted()
            },
        ),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );
    let content = Rect::new(
        inner.x,
        inner.y + 1,
        inner.width,
        inner.height.saturating_sub(1),
    );
    if let Some(snapshot) = &app.health.snapshot {
        draw_snapshot(
            frame,
            snapshot,
            app.health.index,
            app.health.detail,
            app.health.condition_scroll,
            screen.breakpoint,
            locale,
            content,
        );
    } else if !app.health.in_flight && app.health.error.is_none() {
        frame.render_widget(
            Paragraph::new(t(locale, UiText::NoReportYetPressRToLoad)),
            content,
        );
    }
}

fn draw_snapshot(
    frame: &mut Frame<'_>,
    snapshot: &HealthSnapshot,
    index: usize,
    detail: bool,
    condition_scroll: u16,
    breakpoint: Breakpoint,
    locale: Locale,
    area: Rect,
) {
    let condition_lines = condition_lines(snapshot, locale, area.width);
    let conditions_height = condition_viewport_height(area.height, condition_lines.len());
    let [summary, conditions, body] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(conditions_height),
        Constraint::Min(0),
    ])
    .areas(area);
    let reported_workers = snapshot
        .workers
        .len()
        .saturating_add(snapshot.omitted_workers as usize);
    let worker_count = if snapshot.worker_capacity == 0 {
        reported_workers.to_string()
    } else {
        format!("{reported_workers}/{}", snapshot.worker_capacity)
    };
    fixed_rows(
        frame,
        vec![
            format!(
                "Release {} · Build {} · GLOBAL Frame {}",
                snapshot.release, snapshot.build_fingerprint, snapshot.global_head_frame
            ),
            format!(
                "{} {} · {} {} · {} {} · {} {}",
                t(locale, UiText::Workers),
                worker_count,
                t(locale, UiText::Draining),
                snapshot
                    .workers
                    .iter()
                    .filter(|worker| worker.draining)
                    .count(),
                t(locale, UiText::Omitted),
                snapshot.omitted_workers,
                t(locale, UiText::ReportedAt),
                snapshot.generated_unix_ms
            ),
            store_summary(snapshot, locale),
        ],
        summary,
    );
    let conditions_paragraph = Paragraph::new(condition_lines);
    let viewport = frame.area();
    let max_scroll = condition_scroll_max(snapshot, locale, viewport.width, viewport.height);
    frame.render_widget(
        conditions_paragraph.scroll((condition_scroll.min(max_scroll), 0)),
        conditions,
    );
    if snapshot.workers.is_empty() {
        frame.render_widget(
            Paragraph::new(t(locale, UiText::TheNodeReturnedNoWorkerHealthData)),
            body,
        );
        return;
    }
    let selected = index.min(snapshot.workers.len() - 1);
    if breakpoint != Breakpoint::Narrow {
        let [list, _, detail] = Layout::horizontal([
            Constraint::Length(28),
            Constraint::Length(2),
            Constraint::Min(0),
        ])
        .areas(body);
        draw_workers(frame, snapshot, selected, locale, list);
        draw_detail(
            frame,
            snapshot,
            &snapshot.workers[selected],
            snapshot.generated_unix_ms,
            locale,
            detail,
        );
    } else if detail {
        draw_detail(
            frame,
            snapshot,
            &snapshot.workers[selected],
            snapshot.generated_unix_ms,
            locale,
            body,
        );
    } else {
        draw_workers(frame, snapshot, selected, locale, body);
    }
}

fn condition_viewport_height(content_height: u16, line_count: usize) -> u16 {
    let responsive_height = content_height
        .saturating_sub(21)
        .clamp(3, 10)
        .min(content_height.saturating_sub(5));
    let content_height = u16::try_from(line_count.max(3)).unwrap_or(u16::MAX);
    responsive_height.min(content_height)
}

pub(super) fn condition_scroll_max(
    snapshot: &HealthSnapshot,
    locale: Locale,
    viewport_width: u16,
    viewport_height: u16,
) -> u16 {
    if viewport_width < 42 || viewport_height < 12 {
        return 0;
    }
    let content_width = viewport_width.saturating_sub(2);
    let content_height = viewport_height.saturating_sub(5);
    let condition_lines = condition_lines(snapshot, locale, content_width);
    let conditions_height = condition_viewport_height(content_height, condition_lines.len());
    let max_scroll = condition_lines
        .len()
        .saturating_sub(usize::from(conditions_height));
    u16::try_from(max_scroll).unwrap_or(u16::MAX)
}

fn draw_workers(
    frame: &mut Frame<'_>,
    snapshot: &HealthSnapshot,
    index: usize,
    locale: Locale,
    area: Rect,
) {
    let items = snapshot.workers.iter().map(|worker| {
        let condition = if snapshot.conditions_reported {
            snapshot.highest_condition_for_worker(&worker.filter_hex, worker.core_id)
        } else {
            None
        };
        let allocation = if worker.allocated {
            t(locale, UiText::AllocatedLowercase)
        } else {
            t(locale, UiText::Unallocated)
        };
        let marker = condition
            .map(|condition| format!("{} ", severity_marker(&condition.severity)))
            .unwrap_or_default();
        let draining = if worker.draining {
            format!("{} ", t(locale, UiText::Draining).to_ascii_lowercase())
        } else {
            String::new()
        };
        let text = format!(
            "{marker}Core {} {draining}{} {}",
            worker.core_id, worker.phase, allocation
        );
        ListItem::new(truncate_cells(
            &text,
            usize::from(area.width.saturating_sub(2)),
        ))
        .style(condition.map_or_else(
            || {
                if blocked(worker) || worker.draining {
                    theme::attention()
                } else {
                    theme::normal()
                }
            },
            condition_style,
        ))
    });
    let mut state = ListState::default().with_selected(Some(index));
    frame.render_stateful_widget(
        List::new(items)
            .highlight_style(Style::default().add_modifier(Modifier::BOLD))
            .highlight_symbol(theme::FOCUS_MARKER),
        area,
        &mut state,
    );
}

fn blocked(worker: &HealthWorkerSnapshot) -> bool {
    worker.phase == t(Locale::TraditionalChinese, UiText::Blocked)
        || worker.phase.trim().eq_ignore_ascii_case("blocked")
}

fn draw_detail(
    frame: &mut Frame<'_>,
    snapshot: &HealthSnapshot,
    worker: &HealthWorkerSnapshot,
    now: u64,
    locale: Locale,
    area: Rect,
) {
    let outcome = match (worker.last_success_unix_ms, worker.last_failure_unix_ms) {
        (0, 0) => t(locale, UiText::NotRecorded).into(),
        (success, failure) if success > failure => format!(
            "{} · {}",
            t(locale, UiText::Success),
            age(success, now, locale)
        ),
        (success, failure) if success == failure => {
            format!(
                "{} · {}",
                t(locale, UiText::SuccessFailureAtSameTime),
                age(success, now, locale)
            )
        }
        (_, failure) => format!(
            "{} {} ({})",
            t(locale, UiText::Failed),
            worker.last_failure,
            age(failure, now, locale)
        ),
    };
    let failures = if worker.recovery_failures.is_empty() {
        t(locale, UiText::None).into()
    } else {
        worker
            .recovery_failures
            .iter()
            .map(|(class, count)| format!("{class} {count}"))
            .collect::<Vec<_>>()
            .join(" · ")
    };
    let mut lines = vec![
        (
            format!("Core {} {}", worker.core_id, t(locale, UiText::Details)),
            theme::label(),
        ),
        (
            format!(
                "{} {} · Allocation {} · {} {}",
                t(locale, UiText::Status),
                worker.phase,
                if worker.allocated {
                    t(locale, UiText::Yes)
                } else {
                    t(locale, UiText::No)
                },
                t(locale, UiText::Draining),
                if worker.draining {
                    t(locale, UiText::Yes)
                } else {
                    t(locale, UiText::No)
                },
            ),
            if blocked(worker) || worker.draining {
                theme::attention()
            } else {
                theme::normal()
            },
        ),
        (
            format!("{} {}", t(locale, UiText::Blocker), worker.blocker),
            if blocked(worker) {
                theme::attention()
            } else {
                theme::normal()
            },
        ),
    ];
    let associated = if snapshot.conditions_reported {
        snapshot.conditions_for_worker(&worker.filter_hex, worker.core_id)
    } else {
        Vec::new()
    };
    lines.extend(associated.into_iter().map(|condition| {
        (
            format!(
                "{} {} · {}: {} · {}: {}",
                severity_marker(&condition.severity),
                condition.kind,
                t(locale, UiText::Scope).trim(),
                condition.scope,
                t(locale, UiText::Action),
                condition.action,
            ),
            condition_style(condition),
        )
    }));
    lines.extend([
        (
            format!(
                "Materialized {} · Committee {}",
                optional(worker.materialized_frame),
                optional(worker.committee_head_frame)
            ),
            theme::normal(),
        ),
        (
            format!(
                "Generation {} · {} {}",
                generation(worker.committed_generation),
                t(locale, UiText::Observed),
                age(worker.observed_unix_ms, now, locale)
            ),
            theme::normal(),
        ),
        (
            format!(
                "Recovery {} · {} {} · {} {}",
                worker.recovery_phase,
                t(locale, UiText::Attempts),
                worker.recovery_attempts,
                t(locale, UiText::ConsecutiveFailures),
                worker.recovery_consecutive_failures
            ),
            theme::normal(),
        ),
        (
            t(locale, UiText::RemoteArchiveRecoveryDependency).into(),
            theme::normal(),
        ),
        (
            format!("{} {failures}", t(locale, UiText::FailureClasses)),
            theme::normal(),
        ),
        (
            format!("{} {outcome}", t(locale, UiText::LatestResult)),
            theme::normal(),
        ),
        (
            format!(
                "{} {} · {} {}",
                t(locale, UiText::LatestSuccess),
                age(worker.last_success_unix_ms, now, locale),
                t(locale, UiText::Advanced),
                age(worker.last_advance_unix_ms, now, locale)
            ),
            theme::normal(),
        ),
        (
            format!(
                "Join Epoch {} · Activation {} · Awaited {}",
                optional(worker.join_confirm_epoch),
                optional(worker.activation_frame),
                optional(worker.awaited_frame)
            ),
            theme::normal(),
        ),
        (
            format!(
                "Seal {} · GLOBAL {} {}",
                generation(worker.seal_generation),
                t(locale, UiText::RecordLabel),
                age(worker.seal_recorded_unix_ms, now, locale)
            ),
            theme::normal(),
        ),
        (
            format!(
                "Filter {}",
                compact_middle(
                    &worker.filter_hex,
                    usize::from(area.width.saturating_sub(7))
                )
            ),
            theme::normal(),
        ),
        (
            format!(
                "{} {} ms",
                t(locale, UiText::LongestPermitWait),
                worker.longest_permit_wait_ms
            ),
            theme::normal(),
        ),
        (
            format!(
                "{} {} ({})",
                t(locale, UiText::LatestFailure),
                worker.last_failure,
                age(worker.last_failure_unix_ms, now, locale)
            ),
            theme::normal(),
        ),
    ]);
    let lines = lines
        .into_iter()
        .map(|(line, style)| {
            Line::from(Span::styled(
                truncate_cells(&line, area.width.into()),
                style,
            ))
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines), area);
}

fn store_summary(snapshot: &HealthSnapshot, locale: Locale) -> String {
    let write_state = snapshot
        .master_store_write_failed
        .then(|| t(locale, UiText::WriteStopped));
    let io_state = snapshot.store_io.as_ref().map_or_else(
        || t(locale, UiText::Unmeasured).to_string(),
        |io| match io.verdict.trim().to_ascii_lowercase().as_str() {
            "ok" => "OK".into(),
            "slow" => {
                if io.slow_reason.is_empty() {
                    format!("SLOW · {}s", io.slow_secs)
                } else {
                    format!("SLOW · {} · {}s", io.slow_reason, io.slow_secs)
                }
            }
            "unmeasured" | "unspecified" | "unknown" => t(locale, UiText::Unmeasured).into(),
            verdict => verdict.to_ascii_uppercase(),
        },
    );
    let figures = snapshot
        .store_io
        .as_ref()
        .map(|io| io.figures.trim())
        .filter(|figures| !figures.is_empty())
        .map(|figures| format!(" · {figures}"))
        .unwrap_or_default();
    match write_state {
        Some(write_state) => format!(
            "{} {write_state} · I/O {io_state}{figures}",
            t(locale, UiText::Store)
        ),
        None => format!("{} {io_state}{figures}", t(locale, UiText::Store)),
    }
}

fn condition_lines(
    snapshot: &HealthSnapshot,
    locale: Locale,
    content_width: u16,
) -> Vec<Line<'static>> {
    condition_logical_lines(snapshot, locale)
        .into_iter()
        .flat_map(|(text, style)| {
            wrap_cells(&text, usize::from(content_width))
                .into_iter()
                .map(move |line| Line::styled(line, style))
        })
        .collect()
}

fn condition_logical_lines(snapshot: &HealthSnapshot, locale: Locale) -> Vec<(String, Style)> {
    if !snapshot.conditions_reported {
        return vec![(
            t(locale, UiText::OlderNodeDidNotReportConditions).into(),
            theme::muted(),
        )];
    }
    let counts = ["error", "warning", "info"].map(|severity| {
        snapshot
            .conditions
            .iter()
            .filter(|condition| condition.severity.eq_ignore_ascii_case(severity))
            .count()
    });
    if snapshot.conditions.is_empty() {
        if snapshot.unobserved_conditions.is_empty() {
            return vec![(
                format!("{} OK", t(locale, UiText::Conditions)),
                theme::healthy(),
            )];
        }
        return vec![
            (
                format!(
                    "{} {}",
                    t(locale, UiText::Conditions),
                    t(locale, UiText::UnknownUndetermined)
                ),
                theme::muted(),
            ),
            (
                format!(
                    "{}: {}",
                    t(locale, UiText::Unobserved),
                    snapshot.unobserved_conditions.join(", ")
                ),
                theme::muted(),
            ),
        ];
    }
    let mut lines = vec![(
        format!(
            "{} ERROR {} · WARN {} · INFO {}",
            t(locale, UiText::Conditions),
            counts[0],
            counts[1],
            counts[2]
        ),
        theme::label(),
    )];
    for condition in &snapshot.conditions {
        let style = condition_style(condition);
        lines.extend([
            (
                format!(
                    "{} {}",
                    severity_marker(&condition.severity),
                    condition.notice
                ),
                style,
            ),
            (
                format!(
                    "{}: {} · {}: {}/{}",
                    t(locale, UiText::Scope).trim(),
                    condition.scope,
                    t(locale, UiText::Affected),
                    condition.affected,
                    condition.tracked,
                ),
                style,
            ),
            (
                format!("{}: {}", t(locale, UiText::Action), condition.action),
                style,
            ),
        ]);
    }
    if !snapshot.unobserved_conditions.is_empty() {
        lines.push((
            format!(
                "{}: {}",
                t(locale, UiText::Unobserved),
                snapshot.unobserved_conditions.join(", ")
            ),
            theme::muted(),
        ));
    }
    lines
}

pub(crate) fn severity_marker(severity: &str) -> &'static str {
    match severity.trim().to_ascii_lowercase().as_str() {
        "error" => "[ERROR]",
        "warning" | "warn" => "[WARN]",
        "info" => "[INFO]",
        _ => "[UNKNOWN]",
    }
}

pub(crate) fn condition_style(condition: &HealthConditionSnapshot) -> Style {
    match condition.severity_rank() {
        0 => theme::failure(),
        1 => theme::attention(),
        _ => theme::muted(),
    }
}

fn fixed_rows(frame: &mut Frame<'_>, rows: Vec<String>, area: Rect) {
    frame.render_widget(
        Paragraph::new(
            rows.into_iter()
                .map(|row| Line::from(truncate_cells(&row, area.width.into())))
                .collect::<Vec<_>>(),
        ),
        area,
    );
}

fn optional(value: Option<u64>) -> String {
    value.map_or_else(|| "-".into(), |value| value.to_string())
}
fn generation(value: Option<u64>) -> String {
    value.map_or_else(|| "-".into(), |value| format!("g{value}"))
}
fn age(timestamp_ms: u64, now_ms: u64, locale: Locale) -> String {
    if timestamp_ms == 0 {
        return "-".into();
    }
    let seconds = now_ms.saturating_sub(timestamp_ms) / 1_000;
    if seconds < 60 {
        format!("{seconds}s {}", t(locale, UiText::Ago))
    } else if seconds < 3_600 {
        format!("{}m {}", seconds / 60, t(locale, UiText::Ago))
    } else {
        format!("{}h {}", seconds / 3_600, t(locale, UiText::Ago))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_markers_keep_unknown_distinct_from_info() {
        assert_eq!(severity_marker("info"), "[INFO]");
        assert_eq!(severity_marker("future-severity"), "[UNKNOWN]");
    }

    #[test]
    fn age_keeps_missing_timestamp_distinct_from_now() {
        assert_eq!(age(0, 10_000, Locale::English), "-");
        assert_eq!(age(9_000, 10_000, Locale::English), "1s ago");
        assert_eq!(age(9_000, 10_000, Locale::TraditionalChinese), "1s 前");
    }
}
