use crate::i18n::UiText;
use ratatui::{
    layout::{Constraint, Layout, Margin},
    widgets::{List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::{app::App, i18n::t};

use super::{chrome, text::compact_middle, theme};

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let locale = app.locale();
    let Some(screen) = chrome::screen(
        frame,
        t(locale, UiText::AliasActionsTitle),
        app,
        t(locale, UiText::SelectEnterContinueEscQBackCtrlCExit),
    ) else {
        return;
    };
    frame.render_widget(chrome::panel(t(locale, UiText::AliasActions)), screen.body);
    let inner = screen.body.inner(Margin::new(1, 1));
    let rows = Layout::vertical([Constraint::Length(2), Constraint::Min(3)]).split(inner);
    let selected = app.aliases.get(app.alias_index);
    let selected_name = selected.map_or_else(
        || t(locale, UiText::None).into(),
        |alias| compact_middle(&alias.name, usize::from(inner.width.saturating_sub(12))),
    );
    frame.render_widget(
        Paragraph::new(format!(
            "{}  {selected_name}\n{}",
            t(locale, UiText::SelectedChoice),
            t(locale, UiText::AliasesOnlyModifyTheLocalAliasYml)
        ))
        .style(theme::muted()),
        rows[0],
    );

    let mut actions = vec![ListItem::new(t(locale, UiText::AddOrUpdateAlias))];
    if selected.is_some() {
        actions.push(ListItem::new(t(locale, UiText::DeleteSelectedAlias)));
    }
    let mut state = ListState::default().with_selected(Some(app.alias_action_index));
    frame.render_stateful_widget(
        List::new(actions)
            .highlight_style(theme::focus())
            .highlight_symbol("›"),
        rows[1],
        &mut state,
    );
}
