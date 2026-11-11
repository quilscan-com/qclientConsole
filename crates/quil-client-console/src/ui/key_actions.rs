use crate::i18n::UiText;
use ratatui::{
    layout::{Constraint, Layout, Margin},
    text::Line,
    widgets::{List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::{app::App, domain::protected_key_id, i18n::t};

use super::{chrome, text::compact_middle, theme};

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let locale = app.locale();
    let Some(screen) = chrome::screen(
        frame,
        t(locale, UiText::KeyActionsTitle),
        app,
        t(locale, UiText::SelectEnterContinueEscQBackCtrlCExit),
    ) else {
        return;
    };
    frame.render_widget(chrome::panel(t(locale, UiText::KeyActions)), screen.body);
    let inner = screen.body.inner(Margin::new(1, 1));
    let rows = Layout::vertical([Constraint::Length(3), Constraint::Min(3)]).split(inner);
    let selected = app.keys.get(app.key_index);
    let protected = selected.is_some_and(|key| protected_key_id(&key.id));
    let selected_id = selected.map_or_else(
        || t(locale, UiText::None).into(),
        |key| compact_middle(&key.id, usize::from(inner.width.saturating_sub(12))),
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(format!(
                "{}  {selected_id}",
                t(locale, UiText::SelectedChoice)
            )),
            Line::from(if protected {
                t(locale, UiText::ProtectedQNodeIdentityKeysCannotBeDeleted)
            } else {
                t(locale, UiText::CreatingAndDeletingBothRequireConfirmation)
            }),
        ])
        .style(theme::muted()),
        rows[0],
    );

    let mut actions = vec![ListItem::new(t(locale, UiText::CreateANewKey))];
    if selected.is_some() && !protected {
        actions.push(ListItem::new(t(locale, UiText::DeleteSelectedKey)));
    }
    let mut state = ListState::default().with_selected(Some(app.key_action_index));
    frame.render_stateful_widget(
        List::new(actions)
            .highlight_style(theme::focus())
            .highlight_symbol("›"),
        rows[1],
        &mut state,
    );
}
