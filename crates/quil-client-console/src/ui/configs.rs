use crate::i18n::UiText;
use ratatui::{
    layout::{Constraint, Layout, Margin, Rect},
    style::Modifier,
    text::{Line, Span},
    widgets::{List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::{app::App, i18n::t};

use super::{chrome, text::compact_middle, theme};

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let locale = app.locale();
    let Some(screen) = chrome::screen(
        frame,
        t(locale, UiText::NodeConfiguration),
        app,
        t(
            locale,
            UiText::SelectEnterSwitchNAddDirectoryEscQBackCtrlCExit,
        ),
    ) else {
        return;
    };

    if screen.body.width >= 70 {
        let columns = Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)])
            .split(screen.body);
        draw_list(frame, app, columns[0]);
        draw_detail(frame, app, columns[1]);
    } else {
        let body = Layout::vertical([Constraint::Percentage(52), Constraint::Percentage(48)])
            .split(screen.body);
        draw_list(frame, app, body[0]);
        draw_detail(frame, app, body[1]);
    }
}

fn draw_list(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let items = app.configs.iter().enumerate().map(|(index, config)| {
        let mut line = vec![Span::raw(compact_middle(
            &config.name,
            usize::from(area.width.saturating_sub(22)).max(8),
        ))];
        if index == app.active_config_index {
            line.push(Span::styled(
                t(app.locale(), UiText::Active),
                theme::success(),
            ));
        }
        if config.is_default {
            line.push(Span::styled(
                t(app.locale(), UiText::Default),
                theme::muted(),
            ));
        }
        ListItem::new(Line::from(line))
    });
    let mut state = ListState::default().with_selected(if app.configs.is_empty() {
        None
    } else {
        Some(app.config_index.min(app.configs.len() - 1))
    });
    frame.render_stateful_widget(
        List::new(items)
            .block(chrome::panel(t(app.locale(), UiText::ConfigurationList)))
            .highlight_style(theme::selected())
            .highlight_symbol("› "),
        area,
        &mut state,
    );
    if app.configs.is_empty() {
        frame.render_widget(
            Paragraph::new(t(
                app.locale(),
                UiText::NoCompleteConfigurationFoundBothConfigYmlAndKeysYmlAreRequired,
            ))
            .style(theme::muted())
            .wrap(Wrap { trim: true }),
            area.inner(Margin::new(2, 2)),
        );
    }
}

fn draw_detail(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let value_width = usize::from(area.width.saturating_sub(20));
    let selected = app.configs.get(app.config_index);
    let lines = selected.map_or_else(
        || vec![Line::from(t(app.locale(), UiText::NoConfigurationToDisplay))],
        |config| {
            vec![
                labelled(t(app.locale(), UiText::Name), &compact_middle(&config.name, value_width)),
                labelled(
                    t(app.locale(), UiText::Directory),
                    &compact_middle(&config.path.display().to_string(), value_width),
                ),
                labelled(t(app.locale(), UiText::DefaultConfiguration), if config.is_default { t(app.locale(), UiText::Yes) } else { t(app.locale(), UiText::No) }),
                labelled(
                    t(app.locale(), UiText::ActiveInConsole),
                    if app.config_index == app.active_config_index {
                        t(app.locale(), UiText::Yes)
                    } else {
                        t(app.locale(), UiText::No)
                    },
                ),
                Line::from(""),
                Line::from(t(app.locale(), UiText::SwitchingOnlyChangesTheNodeManagedByThisConsoleConfigurationFilesAreNotModified))
                    .style(theme::muted()),
            ]
        },
    );
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(chrome::panel(t(app.locale(), UiText::ConfigurationDetails))),
        area,
    );
}

fn labelled(label: &'static str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<16}"), theme::muted()),
        Span::styled(
            value.to_string(),
            theme::normal().add_modifier(Modifier::BOLD),
        ),
    ])
}
