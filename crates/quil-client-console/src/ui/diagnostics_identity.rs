use crate::i18n::UiText;
use ratatui::{
    layout::{Alignment, Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::{app::App, i18n::t};

use super::{
    chrome,
    text::{tail_cells, truncate_cells},
    theme,
};
use unicode_width::UnicodeWidthStr;

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let locale = app.locale();
    let area = frame.area();
    if area.width < 42 || area.height < 23 {
        frame.render_widget(
            Paragraph::new(t(locale, UiText::TerminalTooSmallResizeToAtLeast4223))
                .alignment(Alignment::Center)
                .style(theme::attention()),
            area,
        );
        return;
    }

    let footer = if app.diagnostics_identity.editing {
        t(locale, UiText::EnterPathEnterDoneEscCancelCtrlCQuit)
    } else {
        t(locale, UiText::SelectEnterEditRunEscQBackCtrlCQuit)
    };
    let Some(screen) = chrome::screen(
        frame,
        t(locale, UiText::IndependentDiagnosticIdentityTitle),
        app,
        footer,
    ) else {
        return;
    };
    let rows = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(4),
        Constraint::Min(12),
    ])
    .split(screen.body);

    let path_style = if app.diagnostics_identity.editing {
        Style::default().fg(theme::ACCENT)
    } else if app.diagnostics_identity.action_index == 0 {
        theme::selected()
    } else {
        theme::normal()
    };
    let path_width = usize::from(rows[0].width.saturating_sub(2));
    let path = if app.diagnostics_identity.editing {
        tail_cells(&app.diagnostics_identity.path, path_width)
    } else {
        truncate_cells(&app.diagnostics_identity.path, path_width)
    };
    let path_cursor = UnicodeWidthStr::width(path.as_str()).min(path_width);
    frame.render_widget(
        Paragraph::new(path)
            .style(path_style)
            .block(chrome::focus_panel(
                t(locale, UiText::IdentityFilePath),
                app.diagnostics_identity.action_index == 0,
            )),
        rows[0],
    );

    let load = action(
        t(locale, UiText::LoadAndEnable),
        app.diagnostics_identity.action_index == 1,
    );
    let create = action(
        t(locale, UiText::CreateAndEnable),
        app.diagnostics_identity.action_index == 2,
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![load, Span::raw("      "), create]))
            .alignment(Alignment::Center)
            .block(chrome::focus_panel(
                t(locale, UiText::IdentityActions),
                app.diagnostics_identity.action_index > 0,
            )),
        rows[1],
    );

    let body = if app.diagnostics_identity.in_flight {
        vec![Line::from(t(locale, UiText::ProcessingDiagnosticIdentity))]
    } else {
        status_lines(app, rows[2].width.saturating_sub(2) as usize)
    };
    frame.render_widget(
        Paragraph::new(body).block(chrome::panel(t(locale, UiText::StatusNotAProverKey))),
        rows[2],
    );

    if app.diagnostics_identity.editing {
        frame.set_cursor_position((rows[0].x + 1 + path_cursor as u16, rows[0].y + 1));
    }
}

fn status_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if let Some(error) = &app.diagnostics_identity.error {
        lines.push(Line::from(Span::styled(
            truncate_cells(
                &crate::i18n::localize_service_error(app.locale(), error),
                width,
            ),
            Style::default().fg(Color::Red),
        )));
        lines.push(Line::from(""));
    }
    if let Some(identity) = &app.diagnostics_identity.snapshot {
        lines.push(Line::from(Span::styled(
            if app.diagnostics_identity.error.is_some() {
                t(app.locale(), UiText::PreviousIdentityRemainsEnabled)
            } else {
                t(app.locale(), UiText::Enabled)
            },
            Style::default().fg(Color::Green),
        )));
        lines.push(Line::from(vec![
            Span::styled("Peer ID    ", theme::muted()),
            Span::raw(truncate_cells(&identity.peer_id, width.saturating_sub(11))),
        ]));
        lines.push(Line::from(vec![
            Span::styled(t(app.locale(), UiText::File), theme::muted()),
            Span::raw(truncate_cells(&identity.path, width.saturating_sub(11))),
        ]));
        lines.push(Line::from(t(
            app.locale(),
            UiText::NoteTheTargetNodeCanStillSeeTheSourceIP,
        )));
        lines.push(Line::from(t(
            app.locale(),
            UiText::AuthorizationRequiresAddingThePeerIDToAllowedPeers,
        )));
    } else if app.diagnostics_identity.error.is_none() {
        lines.extend([
            Line::from(t(
                app.locale(),
                UiText::LoadAnExistingIdentityOrCreateOneAtTheSelectedPath,
            )),
            Line::from(t(
                app.locale(),
                UiText::CreationNeverOverwritesAnExistingFileUnixPermissionsAreFixedAt600,
            )),
            Line::from(t(
                app.locale(),
                UiText::ThePrivateKeyStaysInThatFileOnlyThePeerIDIsShownHere,
            )),
        ]);
    }
    lines
}

fn action(label: &'static str, selected: bool) -> Span<'static> {
    if selected {
        Span::styled(
            format!("[ {label} ]"),
            theme::selected().add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw(format!("  {label}  "))
    }
}
