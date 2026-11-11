use crate::i18n::UiText;
use ratatui::{
    layout::Margin,
    widgets::{Paragraph, Wrap},
    Frame,
};

use crate::{
    app::App,
    i18n::{localize_operation_message, t},
};

use super::{chrome, theme};

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let locale = app.locale();
    let Some(screen) = chrome::screen(
        frame,
        t(locale, UiText::OperationResult),
        app,
        t(locale, UiText::EnterEscQBackCtrlCExit),
    ) else {
        return;
    };
    let (title, style) = match app.operation_succeeded {
        Some(true) => (
            t(locale, UiText::SuccessOperationSubmitted),
            theme::healthy(),
        ),
        Some(false) => (
            t(locale, UiText::FailedOperationIncomplete),
            theme::failure(),
        ),
        None => (t(locale, UiText::UnknownNotConfirmed), theme::attention()),
    };
    frame.render_widget(chrome::panel(format!(" {title} ")), screen.body);
    let message = app.operation_message.as_deref().map_or_else(
        || t(locale, UiText::NoOperationResult).into(),
        |message| localize_operation_message(locale, message),
    );
    frame.render_widget(
        Paragraph::new(message)
            .style(style)
            .wrap(Wrap { trim: true }),
        screen.body.inner(Margin::new(1, 1)),
    );
}
