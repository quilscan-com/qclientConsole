use crate::i18n::UiText;
use ratatui::{
    layout::{Constraint, Layout, Rect},
    text::{Line, Span},
    widgets::{List, ListItem, ListState, Paragraph},
    Frame,
};

use super::{
    chrome::{self, Breakpoint},
    text::{compact_middle, truncate_cells},
    theme,
};
use crate::{
    app::App,
    domain::{HealthSnapshot, HealthWorkerSnapshot},
    i18n::{t, Locale},
};

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let locale = app.locale();
    let footer = if frame.area().width < 70 {
        t(locale, UiText::SelectEnterDetailsEscQBackRRefreshCtrlCQuit)
    } else {
        t(locale, UiText::SelectCoreRRefreshEscQBackCtrlCQuit)
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
    breakpoint: Breakpoint,
    locale: Locale,
    area: Rect,
) {
    let [summary, body] = Layout::vertical([Constraint::Length(2), Constraint::Min(0)]).areas(area);
    fixed_rows(
        frame,
        vec![
            format!(
                "Release {} · Build {} · GLOBAL Frame {}",
                snapshot.release, snapshot.build_fingerprint, snapshot.global_head_frame
            ),
            format!(
                "{} {} · {} {} · {} {}",
                t(locale, UiText::Workers),
                snapshot.workers.len(),
                t(locale, UiText::Omitted),
                snapshot.omitted_workers,
                t(locale, UiText::ReportedAt),
                snapshot.generated_unix_ms
            ),
        ],
        summary,
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
            Constraint::Length(24),
            Constraint::Length(2),
            Constraint::Min(0),
        ])
        .areas(body);
        draw_workers(frame, snapshot, selected, locale, list);
        draw_detail(
            frame,
            &snapshot.workers[selected],
            snapshot.generated_unix_ms,
            locale,
            detail,
        );
    } else if detail {
        draw_detail(
            frame,
            &snapshot.workers[selected],
            snapshot.generated_unix_ms,
            locale,
            body,
        );
    } else {
        draw_workers(frame, snapshot, selected, locale, body);
    }
}

fn draw_workers(
    frame: &mut Frame<'_>,
    snapshot: &HealthSnapshot,
    index: usize,
    locale: Locale,
    area: Rect,
) {
    let items = snapshot.workers.iter().map(|worker| {
        let allocation = if worker.allocated {
            t(locale, UiText::AllocatedLowercase)
        } else {
            t(locale, UiText::Unallocated)
        };
        let text = format!("Core {} {} {}", worker.core_id, worker.phase, allocation);
        ListItem::new(truncate_cells(
            &text,
            usize::from(area.width.saturating_sub(2)),
        ))
        .style(if blocked(worker) {
            theme::attention()
        } else {
            theme::normal()
        })
    });
    let mut state = ListState::default().with_selected(Some(index));
    frame.render_stateful_widget(
        List::new(items)
            .highlight_style(theme::focus())
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
    let lines = vec![
        format!("Core {} {}", worker.core_id, t(locale, UiText::Details)),
        format!(
            "{} {} · Allocation {}",
            t(locale, UiText::Status),
            worker.phase,
            if worker.allocated {
                t(locale, UiText::Yes)
            } else {
                t(locale, UiText::No)
            }
        ),
        format!("{} {}", t(locale, UiText::Blocker), worker.blocker),
        format!(
            "Materialized {} · Committee {}",
            optional(worker.materialized_frame),
            optional(worker.committee_head_frame)
        ),
        format!(
            "Generation {} · {} {}",
            generation(worker.committed_generation),
            t(locale, UiText::Observed),
            age(worker.observed_unix_ms, now, locale)
        ),
        format!(
            "Recovery {} · {} {} · {} {}",
            worker.recovery_phase,
            t(locale, UiText::Attempts),
            worker.recovery_attempts,
            t(locale, UiText::ConsecutiveFailures),
            worker.recovery_consecutive_failures
        ),
        t(locale, UiText::RemoteArchiveRecoveryDependency).into(),
        format!("{} {failures}", t(locale, UiText::FailureClasses)),
        format!("{} {outcome}", t(locale, UiText::LatestResult)),
        format!(
            "{} {} · {} {}",
            t(locale, UiText::LatestSuccess),
            age(worker.last_success_unix_ms, now, locale),
            t(locale, UiText::Advanced),
            age(worker.last_advance_unix_ms, now, locale)
        ),
        format!(
            "Join Epoch {} · Activation {} · Awaited {}",
            optional(worker.join_confirm_epoch),
            optional(worker.activation_frame),
            optional(worker.awaited_frame)
        ),
        format!(
            "Seal {} · GLOBAL {} {}",
            generation(worker.seal_generation),
            t(locale, UiText::RecordLabel),
            age(worker.seal_recorded_unix_ms, now, locale)
        ),
        format!(
            "Filter {}",
            compact_middle(
                &worker.filter_hex,
                usize::from(area.width.saturating_sub(7))
            )
        ),
        format!(
            "{} {} ms",
            t(locale, UiText::LongestPermitWait),
            worker.longest_permit_wait_ms
        ),
        format!(
            "{} {} ({})",
            t(locale, UiText::LatestFailure),
            worker.last_failure,
            age(worker.last_failure_unix_ms, now, locale)
        ),
    ];
    let lines = lines
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            Line::from(Span::styled(
                truncate_cells(&line, area.width.into()),
                if index == 0 {
                    theme::label()
                } else if blocked(worker) && (index == 1 || index == 2) {
                    theme::attention()
                } else {
                    theme::normal()
                },
            ))
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines), area);
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
    fn age_keeps_missing_timestamp_distinct_from_now() {
        assert_eq!(age(0, 10_000, Locale::English), "-");
        assert_eq!(age(9_000, 10_000, Locale::English), "1s ago");
        assert_eq!(age(9_000, 10_000, Locale::TraditionalChinese), "1s 前");
    }
}
