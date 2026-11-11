use crate::i18n::UiText;
use ratatui::{
    layout::Alignment,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame,
};

use crate::{
    app::App,
    domain::RuntimeSnapshot,
    i18n::{t, Locale},
};

use super::{chrome, text::truncate_cells, theme};

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let locale = app.locale();
    let Some(screen) = chrome::screen(
        frame,
        t(locale, UiText::RuntimeUpdateStatus),
        app,
        t(locale, UiText::RReloadEscQBackCtrlCExit),
    ) else {
        return;
    };

    let body = if app.runtime.in_flight {
        Paragraph::new(t(locale, UiText::LoadingSystemServiceStatus))
            .alignment(Alignment::Center)
            .block(chrome::panel(t(locale, UiText::LocalService)))
    } else if let Some(error) = &app.runtime.error {
        Paragraph::new(crate::i18n::localize_service_error(locale, error).into_owned())
            .style(Style::default().fg(Color::Red))
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true })
            .block(chrome::panel(t(locale, UiText::LoadFailed)))
    } else if let Some(snapshot) = &app.runtime.snapshot {
        snapshot_widget(
            snapshot,
            usize::from(screen.body.width.saturating_sub(2)),
            locale,
        )
    } else {
        Paragraph::new(t(locale, UiText::NoRuntimeSnapshotPressRToLoad))
            .alignment(Alignment::Center)
            .block(chrome::panel(t(locale, UiText::LocalService)))
    };
    frame.render_widget(body, screen.body);
}

fn snapshot_widget(snapshot: &RuntimeSnapshot, width: usize, locale: Locale) -> Paragraph<'static> {
    let state_style = if snapshot.running {
        Style::default().fg(Color::Green)
    } else {
        Style::default().fg(Color::Yellow)
    };
    let enabled = optional_bool(snapshot.enabled, locale);
    let auto_update = optional_bool(snapshot.auto_update_enabled, locale);
    let mut lines = vec![
        row(
            t(locale, UiText::Platform),
            &format!("{} · {}", snapshot.platform, snapshot.manager),
            width,
        ),
        Line::from(vec![
            Span::styled(t(locale, UiText::Service), theme::muted()),
            Span::raw(truncate_cells(
                &snapshot.service_name,
                width.saturating_sub(12),
            )),
        ]),
        Line::from(vec![
            Span::styled(t(locale, UiText::State), theme::muted()),
            Span::styled(snapshot.state.clone(), state_style),
            Span::raw(if snapshot.installed {
                t(locale, UiText::Installed)
            } else {
                t(locale, UiText::NotInstalled)
            }),
        ]),
        row(t(locale, UiText::Autostart), enabled, width),
        row(
            t(locale, UiText::ProcessPID),
            &snapshot
                .pid
                .map_or_else(|| "-".into(), |pid| pid.to_string()),
            width,
        ),
        row(t(locale, UiText::ServiceDefinition), &snapshot.definition_path, width),
        row(t(locale, UiText::AutomaticUpdates), auto_update, width),
        Line::from(""),
        Line::from(Span::styled(
            t(locale, UiText::ThisPageIsReadOnlyStartingStoppingInstallingUninstallingAndUpdatingRequireSeparateConfir),
            theme::muted(),
        )),
    ];
    if let Some(note) = &snapshot.note {
        lines.push(Line::from(Span::styled(
            truncate_cells(&format!("{}        {note}", t(locale, UiText::Note)), width),
            Style::default().fg(Color::Yellow),
        )));
    }
    Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .block(chrome::panel(t(locale, UiText::LocalService)))
}

fn row(label: &'static str, value: &str, width: usize) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<8}"), theme::muted()),
        Span::raw(truncate_cells(value, width.saturating_sub(12))),
    ])
}

fn optional_bool(value: Option<bool>, locale: Locale) -> &'static str {
    match value {
        Some(true) => t(locale, UiText::Enabled),
        Some(false) => t(locale, UiText::Disabled),
        None => t(locale, UiText::UnknownUndetermined),
    }
}
