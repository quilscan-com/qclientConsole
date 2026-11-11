use crate::i18n::UiText;
use ratatui::{
    layout::{Constraint, Layout, Margin},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use unicode_width::UnicodeWidthStr;

use crate::{app::App, i18n::t};

use super::{chrome, text, theme};

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let locale = app.locale();
    let footer = if app.alias_form_editing {
        t(locale, UiText::EditingEnterDoneEscCancelCtrlCExit)
    } else {
        t(locale, UiText::FieldEnterEditContinueEscQBackCtrlCExit)
    };
    let Some(screen) = chrome::screen(frame, t(locale, UiText::EditAlias), app, footer) else {
        return;
    };
    frame.render_widget(
        chrome::panel(t(locale, UiText::AddOrUpdateAliasTitle)),
        screen.body,
    );
    let inner = screen.body.inner(Margin::new(1, 1));
    let rows = Layout::vertical([Constraint::Min(5), Constraint::Length(2)]).split(inner);

    let fields = [
        (
            t(locale, UiText::Alias),
            app.alias_form_name.as_str(),
            t(locale, UiText::ExampleMyWallet),
        ),
        (
            t(locale, UiText::HexAddress),
            app.alias_form_address.as_str(),
            t(locale, UiText::Optional0xPrefix),
        ),
        (
            t(locale, UiText::TypeOptional),
            app.alias_form_kind.as_str(),
            t(locale, UiText::ExampleAccountVertex),
        ),
    ];
    let mut lines = Vec::new();
    const LABEL_WIDTH: usize = 14;
    let value_budget = usize::from(inner.width).saturating_sub(2 + LABEL_WIDTH);
    for (index, (label, value, hint)) in fields.iter().enumerate() {
        let selected = app.alias_form_field == index;
        let shown = if value.is_empty() {
            (*hint).to_string()
        } else if selected && app.alias_form_editing {
            text::tail_cells(value, value_budget)
        } else {
            text::truncate_cells(value, value_budget)
        };
        lines.push(Line::from(vec![
            Span::styled(
                if selected { "› " } else { "  " },
                if selected {
                    theme::title()
                } else {
                    theme::normal()
                },
            ),
            Span::styled(label_column(label, LABEL_WIDTH), theme::muted()),
            Span::styled(
                shown,
                if selected {
                    theme::focus()
                } else if value.is_empty() {
                    theme::muted()
                } else {
                    theme::normal()
                },
            ),
        ]));
    }
    lines.push(Line::from(Span::styled(
        if app.alias_form_field == 3 {
            t(locale, UiText::ReviewAndSaveButton)
        } else {
            t(locale, UiText::ReviewAndSave)
        },
        if app.alias_form_field == 3 {
            theme::focus()
        } else {
            theme::normal()
        },
    )));
    frame.render_widget(Paragraph::new(lines), rows[0]);

    let status = if let Some(error) = &app.alias_form_error {
        Line::from(Span::styled(error, theme::failure()))
    } else if app.alias_form_editing {
        Line::from(Span::styled(
            t(
                locale,
                UiText::EditingAddressIsWrittenOnlyToLocalConfiguration,
            ),
            theme::focus(),
        ))
    } else {
        Line::from(t(locale, UiText::AFullConfirmationPageAppearsBeforeSaving))
    };
    frame.render_widget(Paragraph::new(status), rows[1]);
}

fn label_column(label: &str, width: usize) -> String {
    let mut column = text::truncate_cells(label, width);
    column.push_str(&" ".repeat(width.saturating_sub(column.width())));
    column
}
