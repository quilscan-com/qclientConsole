use crate::i18n::UiText;
use ratatui::{
    layout::{Constraint, Layout, Margin},
    widgets::{List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::{
    app::{available_actions, App},
    i18n::t,
};

use super::{chrome, text::compact_middle, theme};

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let locale = app.locale();
    let Some(screen) = chrome::screen(
        frame,
        t(locale, UiText::ProverActionsTitle),
        app,
        t(locale, UiText::SelectEnterContinueEscQBackCtrlCExit),
    ) else {
        return;
    };
    frame.render_widget(chrome::panel(t(locale, UiText::ProverActions)), screen.body);
    let inner = screen.body.inner(Margin::new(1, 1));
    let chunks = Layout::vertical([Constraint::Length(2), Constraint::Min(3)]).split(inner);
    let selection = app.snapshot.as_ref().and_then(|snapshot| {
        let allocation = snapshot.allocations.get(app.allocation_index)?;
        let worker = snapshot
            .workers
            .iter()
            .find(|worker| worker.filter_hex == allocation.filter_hex);
        Some((allocation, worker, snapshot.worker_info_error.is_some()))
    });
    let title = selection.map_or_else(
        || t(locale, UiText::NoAllocationSelected).to_string(),
        |(allocation, worker, worker_info_unavailable)| {
            format!(
                "{} · {}",
                worker.map_or_else(
                    || {
                        if worker_info_unavailable {
                            t(locale, UiText::BindingUnknownWorkerInformationUnavailable).into()
                        } else {
                            t(locale, UiText::NoCoreBound).into()
                        }
                    },
                    |worker| format!("Core {}", worker.core_id)
                ),
                compact_middle(
                    &allocation.filter_hex,
                    usize::from(inner.width.saturating_sub(20))
                )
            )
        },
    );
    frame.render_widget(
        Paragraph::new(format!(
            "{}  {title}\n{}",
            t(locale, UiText::SelectedObject),
            t(locale, UiText::AllWriteOperationsRequireConfirmation)
        ))
        .style(theme::muted()),
        chunks[0],
    );

    let actions = available_actions(app);
    let mut state = ListState::default().with_selected(
        (!actions.is_empty()).then_some(app.action_index.min(actions.len().saturating_sub(1))),
    );
    frame.render_stateful_widget(
        List::new(
            actions
                .iter()
                .map(|action| ListItem::new(format!("  {}", action.localized_label(locale)))),
        )
        .highlight_style(theme::focus())
        .highlight_symbol("›"),
        chunks[1],
        &mut state,
    );
    if actions.is_empty() {
        frame.render_widget(
            Paragraph::new(t(
                locale,
                UiText::NoActionsAreAvailableForTheCurrentAllocationStateRefreshAndCheckAgain,
            ))
            .style(theme::muted()),
            chunks[1].inner(ratatui::layout::Margin::new(2, 2)),
        );
    }
}
