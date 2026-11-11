use crate::i18n::UiText;
use ratatui::{
    layout::{Alignment, Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame,
};
use unicode_width::UnicodeWidthStr;

use crate::{app::App, i18n::t};

use super::{
    chrome,
    text::{compact_middle, tail_cells, truncate_cells},
    theme,
};

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let locale = app.locale();
    let footer = if app.remote_health.editing {
        t(locale, UiText::EnterValueEnterDoneEscCancelCtrlCQuit)
    } else {
        t(locale, UiText::SelectEnterEditConnectEscQBackCtrlCQuit)
    };
    let Some(screen) = chrome::screen(
        frame,
        t(locale, UiText::RemoteHealthDirectConnection),
        app,
        footer,
    ) else {
        return;
    };

    let rows = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Min(1),
    ])
    .split(screen.body);

    field(
        frame,
        rows[0],
        t(locale, UiText::TargetPeerID),
        &app.remote_health.peer_id,
        app.remote_health.field_index == 0,
        app.remote_health.editing,
    );
    field(
        frame,
        rows[1],
        t(locale, UiText::StreamAddress),
        &app.remote_health.address,
        app.remote_health.field_index == 1,
        app.remote_health.editing,
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            t(locale, UiText::ConnectAndLoadHealthReport),
            if app.remote_health.field_index == 2 {
                theme::selected().add_modifier(Modifier::BOLD)
            } else {
                theme::normal()
            },
        ))
        .alignment(Alignment::Center)
        .block(chrome::focus_panel(
            t(locale, UiText::Run),
            app.remote_health.field_index == 2,
        )),
        rows[2],
    );

    let status = if let Some(error) = &app.remote_health.error {
        Line::from(Span::styled(
            crate::i18n::localize_service_error(locale, error).into_owned(),
            Style::default().fg(Color::Red),
        ))
    } else if let Some(identity) = &app.diagnostics_identity.snapshot {
        Line::from(format!(
            "{}: {}",
            t(locale, UiText::RequestIdentity),
            compact_middle(
                &identity.peer_id,
                usize::from(rows[3].width.saturating_sub(12))
            )
        ))
    } else {
        Line::from(t(
            locale,
            UiText::NoDiagnosticIdentityIsEnabledReturnToDiagnosticIdentityToLoadOrCreateOne,
        ))
    };
    frame.render_widget(Paragraph::new(status).wrap(Wrap { trim: true }), rows[3]);

    if app.remote_health.editing && app.remote_health.field_index < 2 {
        let value = if app.remote_health.field_index == 0 {
            &app.remote_health.peer_id
        } else {
            &app.remote_health.address
        };
        let row = rows[app.remote_health.field_index];
        let width = usize::from(row.width.saturating_sub(2));
        let visible = tail_cells(value, width);
        let cursor = UnicodeWidthStr::width(visible.as_str()).min(width);
        frame.set_cursor_position((row.x + 1 + cursor as u16, row.y + 1));
    }
}

fn field(
    frame: &mut Frame<'_>,
    area: ratatui::layout::Rect,
    title: &'static str,
    value: &str,
    selected: bool,
    editing: bool,
) {
    let style = if selected && editing {
        Style::default().fg(theme::ACCENT)
    } else if selected {
        theme::selected()
    } else {
        theme::normal()
    };
    let width = usize::from(area.width.saturating_sub(2));
    let value = if selected && editing {
        tail_cells(value, width)
    } else {
        truncate_cells(value, width)
    };
    frame.render_widget(
        Paragraph::new(value)
            .style(style)
            .block(chrome::focus_panel(title, selected)),
        area,
    );
}
