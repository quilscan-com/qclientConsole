use crate::i18n::UiText;
use ratatui::{
    layout::Margin,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::{app::App, i18n::t};

use super::{chrome, theme};

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let locale = app.locale();
    let Some(screen) = chrome::screen(
        frame,
        t(locale, UiText::Help),
        app,
        t(locale, UiText::EscQBackCtrlCExit),
    ) else {
        return;
    };
    frame.render_widget(chrome::panel(t(locale, UiText::KeyboardHelp)), screen.body);
    let lines = [
        ("↑↓", t(locale, UiText::SelectAListItemOrField)),
        ("←→", t(locale, UiText::AdjustAnOptionOrConfirmationChoice)),
        ("Enter", t(locale, UiText::OpenEditOrRunTheSelectedAction)),
        ("Esc/Q", t(locale, UiText::CancelTheCurrentLevelAndGoBack)),
        ("R", t(locale, UiText::RefreshTheCurrentPageWhenSupported)),
        (
            "A",
            t(locale, UiText::OpenActionsForTheSelectedProverWhenSupported),
        ),
        ("?", t(locale, UiText::OpenThisHelpPage)),
        ("Ctrl+C", t(locale, UiText::ExitTheProgram)),
    ]
    .into_iter()
    .map(|(key, purpose)| {
        Line::from(vec![
            Span::styled(format!("{key:<10}"), theme::focus()),
            Span::raw(purpose),
        ])
    })
    .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines), screen.body.inner(Margin::new(1, 1)));
}
