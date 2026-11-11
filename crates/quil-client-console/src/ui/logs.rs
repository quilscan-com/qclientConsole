use crate::i18n::UiText;
use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame,
};

use crate::{
    app::{log_core_ids, App},
    i18n::t,
};

use super::{chrome, text::truncate_cells, theme};

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let locale = app.locale();
    let Some(screen) = chrome::screen(
        frame,
        t(locale, UiText::NodeLogs),
        app,
        t(
            locale,
            UiText::MasterWorkerScrollEnterRReloadEscQBackCtrlCExit,
        ),
    ) else {
        return;
    };
    let cores = log_core_ids(app);
    let core_id = cores.get(app.log_core_index).copied().unwrap_or(0);
    let source = app
        .log_snapshot
        .as_ref()
        .map(|snapshot| snapshot.path.as_str())
        .unwrap_or(t(locale, UiText::WaitingToLoad));
    let source = truncate_cells(source, usize::from(screen.body.width.saturating_sub(24)));
    let mut content = vec![Line::from(vec![
        Span::styled(
            if core_id == 0 {
                "‹ Master ›"
            } else {
                "‹ Worker ›"
            },
            theme::title(),
        ),
        Span::raw(format!("  Core {core_id}  ·  {source}")),
    ])];
    let mut log_lines = if app.log_in_flight {
        vec![Line::from(Span::styled(
            t(locale, UiText::LoadingLogs),
            theme::title(),
        ))]
    } else if let Some(error) = &app.log_error {
        vec![
            Line::from(Span::styled(
                t(locale, UiText::LoadFailedSentenceCase),
                Style::default().fg(Color::Red),
            )),
            Line::from(crate::i18n::localize_service_error(locale, error).into_owned()),
        ]
    } else if let Some(snapshot) = &app.log_snapshot {
        visible_lines(
            snapshot.lines.as_slice(),
            screen.body.height.saturating_sub(1),
            app.log_scroll,
        )
        .iter()
        .map(|line| {
            Line::from(truncate_cells(
                line,
                usize::from(screen.body.width.saturating_sub(2)),
            ))
        })
        .collect()
    } else {
        vec![Line::from(t(locale, UiText::NoLogsToDisplay))]
    };
    content.append(&mut log_lines);
    frame.render_widget(
        Paragraph::new(content)
            .wrap(Wrap { trim: false })
            .block(chrome::panel(t(locale, UiText::LatestLogs))),
        screen.body,
    );
}

fn visible_lines(lines: &[String], area_height: u16, scroll_from_end: usize) -> &[String] {
    let capacity = area_height.saturating_sub(2) as usize;
    let end = lines.len().saturating_sub(scroll_from_end.min(lines.len()));
    let start = end.saturating_sub(capacity);
    &lines[start..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrolling_moves_window_away_from_latest_line() {
        let lines = (1..=5).map(|value| value.to_string()).collect::<Vec<_>>();

        assert_eq!(visible_lines(&lines, 5, 0), &["3", "4", "5"]);
        assert_eq!(visible_lines(&lines, 5, 2), &["1", "2", "3"]);
    }
}
