use crate::i18n::UiText;
use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::{
    app::App,
    domain::{HypergraphEntrySnapshot, HypergraphSnapshot},
    i18n::{t, Locale},
};

use super::{
    chrome,
    text::{compact_middle, tail_cells},
    theme,
};
use unicode_width::UnicodeWidthStr;

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let locale = app.locale();
    let has_entries = app
        .hypergraph
        .snapshot
        .as_ref()
        .is_some_and(|snapshot| !snapshot.entries.is_empty());
    let footer = if app.hypergraph.editing {
        t(
            locale,
            UiText::EnterAddressAliasEnterQueryEscCancelCtrlCExit,
        )
    } else if app.narrow_read_detail {
        t(locale, UiText::EEditQueryEscQListCtrlCExit)
    } else if app.viewport_width <= 69 && has_entries {
        t(
            locale,
            UiText::EnterDetailsEEditQuerySelectEscQBackCtrlCExit,
        )
    } else if has_entries {
        t(locale, UiText::EntryEnterEditQueryRRetryEscQBackCtrlCExit)
    } else {
        t(
            locale,
            UiText::FieldTypeEnterEditQueryRRetryEscQBackCtrlCExit,
        )
    };
    let Some(screen) = chrome::screen(frame, t(locale, UiText::HypergraphQuery), app, footer)
    else {
        return;
    };

    if app.narrow_read_detail {
        if let Some(snapshot) = &app.hypergraph.snapshot {
            draw_entry_detail(frame, app, snapshot, screen.body);
            return;
        }
    }

    let rows = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(4),
        Constraint::Min(3),
    ])
    .split(screen.body);
    frame.render_widget(
        Paragraph::new(format!(
            "{}    {}",
            app.hypergraph.kind.label(),
            t(locale, UiText::SwitchType)
        ))
        .style(if app.hypergraph.field_index == 0 {
            theme::focus()
        } else {
            theme::normal()
        })
        .block(chrome::focus_panel(
            t(locale, UiText::ObjectType),
            app.hypergraph.field_index == 0,
        )),
        rows[0],
    );
    draw_address(frame, app, rows[1]);
    draw_result(frame, app, rows[2]);
}

fn draw_address(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let locale = app.locale();
    let placeholder = t(locale, UiText::Text64ByteHexadecimalAddressOrLocalAlias);
    let (value, cursor) = input_window(
        &app.hypergraph.address,
        area.width.saturating_sub(2) as usize,
        app.hypergraph.editing,
    );
    let text = if value.is_empty() && !app.hypergraph.editing {
        placeholder.into()
    } else {
        value
    };
    let focused = app.hypergraph.field_index == 1;
    let hint = if area.width < 50 {
        t(locale, UiText::Ex0x0001128HexMyWallet)
    } else if area.width < 80 {
        t(locale, UiText::Address0x0001128HexAliasMyWallet)
    } else {
        t(
            locale,
            UiText::AddressExample0x0001Full128HexAliasExampleMyWallet,
        )
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                text,
                if focused {
                    theme::focus()
                } else {
                    theme::normal()
                },
            )),
            Line::from(Span::styled(hint, theme::muted())),
        ])
        .block(chrome::focus_panel(
            t(locale, UiText::AddressOrAlias),
            focused,
        )),
        area,
    );
    if app.hypergraph.editing {
        frame.set_cursor_position((area.x + 1 + cursor as u16, area.y + 1));
    }
}

fn draw_result(frame: &mut Frame<'_>, app: &App, area: Rect) {
    if app.hypergraph.in_flight {
        frame.render_widget(
            Paragraph::new(t(app.locale(), UiText::LoadingNodeData))
                .alignment(Alignment::Center)
                .block(chrome::panel(t(app.locale(), UiText::QueryResults))),
            area,
        );
        return;
    }
    if let Some(error) = &app.hypergraph.error {
        frame.render_widget(
            Paragraph::new(crate::i18n::localize_service_error(app.locale(), error).into_owned())
                .style(Style::default().fg(Color::Red))
                .wrap(Wrap { trim: true })
                .block(chrome::panel(t(app.locale(), UiText::QueryFailed))),
            area,
        );
        return;
    }
    let Some(snapshot) = &app.hypergraph.snapshot else {
        frame.render_widget(
            Paragraph::new(t(app.locale(), UiText::PressEnterToEnterAnAddressAndQuery))
                .alignment(Alignment::Center)
                .block(chrome::panel(t(app.locale(), UiText::QueryResults))),
            area,
        );
        return;
    };

    if snapshot.present == Some(false) {
        frame.render_widget(
            Paragraph::new(t(
                app.locale(),
                UiText::TheNodeConfirmsThisVertexDoesNotExist,
            ))
            .alignment(Alignment::Center)
            .block(chrome::panel(t(app.locale(), UiText::QueryResults))),
            area,
        );
        return;
    }

    let rows = Layout::vertical([Constraint::Length(4), Constraint::Min(2)]).split(area);
    draw_metadata(frame, snapshot, rows[0], app.locale());
    if snapshot.entries.is_empty() {
        let message = if snapshot.present == Some(true) {
            t(app.locale(), UiText::ObjectExistsButHasNoEntries)
        } else {
            t(
                app.locale(),
                UiText::NodeReturned0EntriesThisAPIProvidesNoExistenceFlag,
            )
        };
        frame.render_widget(
            Paragraph::new(message)
                .alignment(Alignment::Center)
                .block(chrome::panel(t(app.locale(), UiText::Entries))),
            rows[1],
        );
    } else if area.width >= 78 {
        let columns = Layout::horizontal([Constraint::Percentage(48), Constraint::Percentage(52)])
            .split(rows[1]);
        draw_entries(frame, app, snapshot, columns[0]);
        draw_entry_detail(frame, app, snapshot, columns[1]);
    } else {
        draw_entries(frame, app, snapshot, rows[1]);
    }
}

fn draw_metadata(frame: &mut Frame<'_>, snapshot: &HypergraphSnapshot, area: Rect, locale: Locale) {
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(format!(
                "{} · {}/{} · {} {}",
                snapshot.kind.label(),
                fallback(&snapshot.set_type, locale),
                fallback(&snapshot.phase_type, locale),
                snapshot.entries.len(),
                t(locale, UiText::Entries)
            )),
            Line::from(format!(
                "Shard L1 {}   L2 {}",
                compact_middle(&snapshot.shard_l1_hex, 12),
                compact_middle(&snapshot.shard_l2_hex, 24)
            )),
        ])
        .block(chrome::panel(t(locale, UiText::ObjectInformation))),
        area,
    );
}

fn draw_entries(frame: &mut Frame<'_>, app: &App, snapshot: &HypergraphSnapshot, area: Rect) {
    let items = snapshot.entries.iter().enumerate().map(|(index, entry)| {
        ListItem::new(format!(
            "[{index}] {} = {}",
            compact_middle(&entry.key_hex, 14),
            compact_middle(&entry.value_hex, 18)
        ))
    });
    let mut state = ListState::default().with_selected(Some(app.hypergraph.entry_index));
    frame.render_stateful_widget(
        List::new(items)
            .block(chrome::panel(t(app.locale(), UiText::Entries)))
            .highlight_style(theme::selected())
            .highlight_symbol("› "),
        area,
        &mut state,
    );
}

fn draw_entry_detail(frame: &mut Frame<'_>, app: &App, snapshot: &HypergraphSnapshot, area: Rect) {
    let lines = snapshot
        .entries
        .get(app.hypergraph.entry_index)
        .map(|entry| {
            entry_lines(
                entry,
                usize::from(area.width.saturating_sub(2)),
                app.locale(),
            )
        })
        .unwrap_or_else(|| vec![Line::from(t(app.locale(), UiText::NoEntry))]);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(chrome::panel(t(app.locale(), UiText::EntryDetails))),
        area,
    );
}

fn entry_lines(
    entry: &HypergraphEntrySnapshot,
    width: usize,
    locale: Locale,
) -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(t(locale, UiText::Key), theme::muted())),
        Line::from(compact_middle(&entry.key_hex, width)),
        Line::from(""),
        Line::from(Span::styled(t(locale, UiText::Value), theme::muted())),
        Line::from(compact_middle(&entry.value_hex, width)),
    ]
}

fn input_window(value: &str, width: usize, editing: bool) -> (String, usize) {
    let width = width.max(1);
    if !editing || UnicodeWidthStr::width(value) <= width {
        return (value.into(), UnicodeWidthStr::width(value).min(width));
    }
    (tail_cells(value, width), width)
}

fn fallback(value: &str, locale: Locale) -> &str {
    if value.is_empty() {
        t(locale, UiText::Unknown)
    } else {
        value
    }
}
