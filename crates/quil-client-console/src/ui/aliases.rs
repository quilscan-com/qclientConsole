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
        t(locale, UiText::AddressAliases),
        app,
        t(locale, UiText::SelectEnterManageRReloadEscQBackCtrlCExit),
    ) else {
        return;
    };

    if app.alias_in_flight {
        frame.render_widget(
            Paragraph::new(t(locale, UiText::LoadingAliases))
                .alignment(Alignment::Center)
                .block(chrome::panel(t(locale, UiText::Aliases))),
            screen.body,
        );
    } else if let Some(error) = &app.alias_error {
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
            .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
            .split(screen.body);
        draw_list(frame, app, columns[0]);
        draw_detail(frame, app, columns[1]);
    } else {
        draw_list(frame, app, screen.body);
    }
}

fn draw_list(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let row_width = usize::from(area.width.saturating_sub(4));
    let items = if app.aliases.is_empty() {
        vec![ListItem::new(t(
            app.locale(),
            UiText::NoAddressAliasesPressEnterToAddOne,
        ))]
    } else {
        app.aliases
            .iter()
            .map(|alias| {
                let kind = if alias.kind.is_empty() {
                    t(app.locale(), UiText::Unclassified)
                } else {
                    alias.kind.as_str()
                };
                let kind = truncate_cells(kind, (row_width / 3).max(8));
                let kind_width = UnicodeWidthStr::width(kind.as_str());
                let name_width = row_width.saturating_sub(kind_width + 1);
                let name = truncate_cells(&alias.name, name_width);
                let padding = " "
                    .repeat(name_width.saturating_sub(UnicodeWidthStr::width(name.as_str())) + 1);
                ListItem::new(Line::from(vec![
                    Span::raw(name),
                    Span::raw(padding),
                    Span::styled(kind, theme::muted()),
                ]))
            })
            .collect()
    };
    let mut state =
        ListState::default().with_selected((!app.aliases.is_empty()).then_some(app.alias_index));
    frame.render_stateful_widget(
        List::new(items)
            .block(chrome::panel(t(app.locale(), UiText::Aliases)))
            .highlight_style(theme::selected())
            .highlight_symbol("› "),
        area,
        &mut state,
    );
}

fn draw_detail(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let lines = match app.aliases.get(app.alias_index) {
        Some(alias) => vec![
            Line::from(Span::styled(
                t(
                    app.locale(),
                    UiText::LocalAliasYmlNoOnChainTransactionIsSubmitted,
                ),
                theme::muted(),
            )),
            Line::from(""),
            Line::from(format!("{}: {}", t(app.locale(), UiText::Name), alias.name)),
            Line::from(format!(
                "{}: {}",
                t(app.locale(), UiText::Type),
                if alias.kind.is_empty() {
                    t(app.locale(), UiText::NotSet)
                } else {
                    alias.kind.as_str()
                }
            )),
            Line::from(""),
            Line::from(format!("{}:", t(app.locale(), UiText::Address))),
            Line::from(compact_middle(
                &alias.address_hex,
                usize::from(area.width.saturating_sub(2)),
            )),
        ],
        None => vec![
            Line::from(t(
                app.locale(),
                UiText::AliasesSaveLongHexadecimalAddressesUnderReadableNames,
            )),
            Line::from(t(
                app.locale(),
                UiText::TheyAreStoredInLocalAliasYmlAndNeverSubmittedOnChain,
            )),
            Line::from(""),
            Line::from(t(app.locale(), UiText::PressEnterToOpenTheManagementMenu)),
        ],
    };
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(chrome::panel(t(app.locale(), UiText::AliasDetails))),
        area,
    );
}
