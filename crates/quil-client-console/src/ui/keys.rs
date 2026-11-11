use crate::i18n::UiText;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::{app::App, i18n::t};

use super::{
    chrome,
    text::{compact_middle, truncate_cells},
    theme,
};
use unicode_width::UnicodeWidthStr;

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let locale = app.locale();
    let Some(screen) = chrome::screen(
        frame,
        t(locale, UiText::KeyManagement),
        app,
        t(locale, UiText::SelectEnterManageRReloadEscQBackCtrlCExit),
    ) else {
        return;
    };

    if app.key_in_flight {
        frame.render_widget(
            Paragraph::new(t(locale, UiText::LoadingKeys))
                .alignment(Alignment::Center)
                .block(chrome::panel(t(locale, UiText::KeysPublicKeysOnly))),
            screen.body,
        );
    } else if let Some(error) = &app.key_error {
        frame.render_widget(
            Paragraph::new(crate::i18n::localize_service_error(locale, error).into_owned())
                .style(Style::default().fg(Color::Red))
                .wrap(Wrap { trim: true })
                .block(chrome::panel(t(locale, UiText::LoadFailed))),
            screen.body,
        );
    } else if screen.body.width >= 70 {
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(46), Constraint::Percentage(54)])
            .split(screen.body);
        draw_list(frame, app, columns[0]);
        draw_detail(frame, app, columns[1]);
    } else {
        draw_list(frame, app, screen.body);
    }
}

fn draw_list(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let row_width = usize::from(area.width.saturating_sub(4));
    let items = app.keys.iter().map(|key| {
        let kind = truncate_cells(key.type_name(), row_width / 2);
        let kind_width = UnicodeWidthStr::width(kind.as_str());
        let id_width = row_width.saturating_sub(kind_width + 1);
        let id = truncate_cells(&key.id, id_width);
        let padding = " ".repeat(id_width.saturating_sub(UnicodeWidthStr::width(id.as_str())) + 1);
        ListItem::new(Line::from(vec![
            Span::raw(id),
            Span::raw(padding),
            Span::styled(kind, theme::muted()),
        ]))
    });
    let mut state =
        ListState::default().with_selected((!app.keys.is_empty()).then_some(app.key_index));
    frame.render_stateful_widget(
        List::new(items)
            .block(chrome::panel(t(app.locale(), UiText::KeysPublicKeysOnly)))
            .highlight_style(theme::selected())
            .highlight_symbol("› "),
        area,
        &mut state,
    );
}

fn draw_detail(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let lines = match app.keys.get(app.key_index) {
        Some(key) => vec![
            Line::from(format!("{}: {}", t(app.locale(), UiText::Name), key.id)),
            Line::from(format!(
                "{}: {}",
                t(app.locale(), UiText::Type),
                key.type_name()
            )),
            Line::from(""),
            Line::from(format!("{}:", t(app.locale(), UiText::PublicKey))),
            Line::from(compact_middle(
                &key.public_key_hex,
                usize::from(area.width.saturating_sub(2)),
            )),
        ],
        None => vec![Line::from(t(
            app.locale(),
            UiText::NoKeysAreAvailableForThisConfiguration,
        ))],
    };
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(chrome::panel(t(app.locale(), UiText::PublicKeyDetails))),
        area,
    );
}
