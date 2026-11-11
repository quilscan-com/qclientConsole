use crate::i18n::UiText;
use ratatui::{
    layout::{Constraint, Layout, Margin},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::{app::App, domain::CREATABLE_KEY_TYPES, i18n::t};

use super::{chrome, text::tail_cells, theme};

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let locale = app.locale();
    let footer = if app.key_form_editing {
        t(locale, UiText::EditingEnterDoneEscCancelCtrlCExit)
    } else {
        t(locale, UiText::FieldTypeEnterEditContinueEscQBackCtrlCExit)
    };
    let Some(screen) = chrome::screen(frame, t(locale, UiText::CreateKey), app, footer) else {
        return;
    };
    frame.render_widget(
        chrome::panel(t(locale, UiText::CreateLocalKey)),
        screen.body,
    );
    let inner = screen.body.inner(Margin::new(1, 1));
    let rows = Layout::vertical([Constraint::Min(5), Constraint::Length(2)]).split(inner);

    let (_, type_name) = CREATABLE_KEY_TYPES
        .get(app.key_form_type_index)
        .copied()
        .unwrap_or(CREATABLE_KEY_TYPES[0]);
    let name_style = if app.key_form_field == 0 {
        theme::focus()
    } else if app.key_form_id.is_empty() {
        theme::muted()
    } else {
        theme::normal()
    };
    let name_budget = usize::from(inner.width.saturating_sub(18));
    let visible_name = if app.key_form_id.is_empty() {
        t(locale, UiText::ExampleWalletSigning).into()
    } else if app.key_form_editing {
        tail_cells(&app.key_form_id, name_budget)
    } else {
        super::text::truncate_cells(&app.key_form_id, name_budget)
    };
    let lines = vec![
        Line::from(vec![
            Span::styled(
                if app.key_form_field == 0 {
                    "› "
                } else {
                    "  "
                },
                theme::title(),
            ),
            Span::styled(t(locale, UiText::KeyNamePadded), theme::muted()),
            Span::styled(visible_name, name_style),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                if app.key_form_field == 1 {
                    "› "
                } else {
                    "  "
                },
                theme::title(),
            ),
            Span::styled(t(locale, UiText::KeyTypePadded), theme::muted()),
            Span::styled(
                format!("←  {type_name}  →"),
                if app.key_form_field == 1 {
                    theme::focus()
                } else {
                    theme::normal()
                },
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            if app.key_form_field == 2 {
                t(locale, UiText::ReviewBeforeCreatingButton)
            } else {
                t(locale, UiText::ReviewBeforeCreating)
            },
            if app.key_form_field == 2 {
                theme::focus()
            } else {
                theme::normal()
            },
        )),
    ];
    frame.render_widget(Paragraph::new(lines), rows[0]);

    let status = if let Some(error) = &app.key_form_error {
        Line::from(Span::styled(error, theme::failure()))
    } else if app.key_form_editing {
        Line::from(Span::styled(
            t(locale, UiText::EditingPrivateKeyIsNeverDisplayed),
            theme::focus(),
        ))
    } else {
        Line::from(t(
            locale,
            UiText::OnlyThePublicKeyIsDisplayedAfterCreationThePrivateKeyNeverAppearsInTheUI,
        ))
    };
    frame.render_widget(Paragraph::new(status), rows[1]);
}
