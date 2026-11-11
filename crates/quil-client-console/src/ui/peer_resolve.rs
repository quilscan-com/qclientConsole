use crate::i18n::UiText;
use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::{
    app::App,
    domain::PeerAddressReport,
    i18n::{t, Locale},
};

use super::{
    chrome,
    text::{compact_middle, tail_cells, truncate_cells},
    theme,
};
use unicode_width::UnicodeWidthStr;

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let locale = app.locale();
    let footer = if app.peer_resolve.editing {
        t(locale, UiText::EnterPeerIDEnterSearchEscCancelCtrlCQuit)
    } else if app.narrow_read_detail {
        t(locale, UiText::EEditQueryEscQBackToListCtrlCQuit)
    } else if app.viewport_width <= 69
        && app
            .peer_resolve
            .report
            .as_ref()
            .is_some_and(|report| !report.addresses.is_empty())
    {
        t(
            locale,
            UiText::EnterDetailsEEditQueryHHealthReportEscQBackCtrlCQuit,
        )
    } else if app.peer_resolve.report.is_some() {
        t(
            locale,
            UiText::SelectStreamHHealthReportEnterEditRetryRRefreshEscQBackCtrlCQuit,
        )
    } else {
        t(
            locale,
            UiText::SelectEnterEditRetryRRefreshEscQBackCtrlCQuit,
        )
    };
    let Some(screen) = chrome::screen(frame, t(locale, UiText::PeerAddressResolution), app, footer)
    else {
        return;
    };

    if app.narrow_read_detail {
        if let Some(report) = &app.peer_resolve.report {
            if !report.addresses.is_empty() {
                draw_detail(frame, report, app.peer_resolve.index, locale, screen.body);
                return;
            }
        }
    }
    let rows = Layout::vertical([Constraint::Length(3), Constraint::Min(4)]).split(screen.body);

    let input_style = if app.peer_resolve.editing {
        Style::default().fg(theme::ACCENT)
    } else {
        theme::normal()
    };
    let input_width = usize::from(rows[0].width.saturating_sub(2));
    let input = if app.peer_resolve.peer_id.is_empty() && !app.peer_resolve.editing {
        t(locale, UiText::EnterTargetPeerID).into()
    } else if app.peer_resolve.editing {
        tail_cells(&app.peer_resolve.peer_id, input_width)
    } else {
        truncate_cells(&app.peer_resolve.peer_id, input_width)
    };
    let input_cursor = UnicodeWidthStr::width(input.as_str()).min(input_width);
    frame.render_widget(
        Paragraph::new(input)
            .style(input_style)
            .block(Block::default().title(" Peer ID ").borders(Borders::ALL)),
        rows[0],
    );

    if app.peer_resolve.in_flight {
        frame.render_widget(
            centered(t(locale, UiText::LoadingLocalPeerAnnouncements), locale),
            rows[1],
        );
    } else if let Some(error) = &app.peer_resolve.error {
        frame.render_widget(
            Paragraph::new(crate::i18n::localize_service_error(locale, error).into_owned())
                .style(Style::default().fg(Color::Red))
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true })
                .block(chrome::panel(t(locale, UiText::ResolutionFailed))),
            rows[1],
        );
    } else if let Some(report) = &app.peer_resolve.report {
        draw_report(frame, report, app.peer_resolve.index, locale, rows[1]);
    } else {
        frame.render_widget(
            centered(
                t(locale, UiText::PressEnterToProvideAPeerIDReadsLocalAnnouncementsWithoutDialingTheTarget),
                locale,
            ),
            rows[1],
        );
    }

    if app.peer_resolve.editing {
        frame.set_cursor_position((rows[0].x + 1 + input_cursor as u16, rows[0].y + 1));
    }
}

fn centered(text: &'static str, locale: Locale) -> Paragraph<'static> {
    Paragraph::new(text)
        .alignment(Alignment::Center)
        .block(chrome::panel(t(locale, UiText::PeerAnnouncements)))
}

fn draw_report(
    frame: &mut Frame<'_>,
    report: &PeerAddressReport,
    index: usize,
    locale: Locale,
    area: Rect,
) {
    if report.addresses.is_empty() {
        frame.render_widget(
            centered(
                t(locale, UiText::TheTargetPeerHasPublishedNoAddresses),
                locale,
            ),
            area,
        );
        return;
    }
    let selected = index.min(report.addresses.len().saturating_sub(1));
    if area.width >= 82 {
        let columns = Layout::horizontal([Constraint::Percentage(52), Constraint::Percentage(48)])
            .split(area);
        draw_list(frame, report, selected, locale, columns[0]);
        draw_detail(frame, report, selected, locale, columns[1]);
    } else {
        draw_list(frame, report, selected, locale, area);
    }
}

fn draw_list(
    frame: &mut Frame<'_>,
    report: &PeerAddressReport,
    index: usize,
    locale: Locale,
    area: Rect,
) {
    let items = report.addresses.iter().map(|address| {
        let host = address
            .host_port
            .as_deref()
            .unwrap_or(t(locale, UiText::HostAndPortUnavailable));
        ListItem::new(format!(
            "{:<7} {:<4} {}",
            address.kind,
            address.reach.localized_label(locale),
            truncate_cells(host, usize::from(area.width.saturating_sub(18)))
        ))
    });
    let mut state = ListState::default().with_selected(Some(index));
    frame.render_stateful_widget(
        List::new(items)
            .block(chrome::panel(format!(
                "{} · {} {}",
                compact_middle(&report.peer_id, 24),
                report.addresses.len(),
                t(locale, UiText::Addresses)
            )))
            .highlight_style(theme::selected())
            .highlight_symbol("› "),
        area,
        &mut state,
    );
}

fn draw_detail(
    frame: &mut Frame<'_>,
    report: &PeerAddressReport,
    index: usize,
    locale: Locale,
    area: Rect,
) {
    let Some(address) = report
        .addresses
        .get(index.min(report.addresses.len().saturating_sub(1)))
    else {
        frame.render_widget(
            centered(
                t(locale, UiText::TheTargetPeerHasPublishedNoAddresses),
                locale,
            ),
            area,
        );
        return;
    };
    let value_width = usize::from(area.width.saturating_sub(12));
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                t(
                    locale,
                    UiText::ReadsLocalAnnouncementsWithoutDialingTheTarget,
                ),
                theme::muted(),
            )),
            label(t(locale, UiText::Purpose), address.kind.clone()),
            label(
                t(locale, UiText::Reachability),
                address.reach.localized_label(locale).into(),
            ),
            label(
                t(locale, UiText::HostPort),
                address
                    .host_port
                    .as_deref()
                    .map(|value| compact_middle(value, value_width))
                    .unwrap_or_else(|| "—".into()),
            ),
            label("Filter", compact_middle(&address.filter, value_width)),
            Line::from(""),
            Line::from(Span::styled(
                t(locale, UiText::FullMultiaddr),
                theme::muted(),
            )),
            Line::from(compact_middle(
                &address.multiaddr,
                usize::from(area.width.saturating_sub(2)),
            )),
        ])
        .wrap(Wrap { trim: false })
        .block(chrome::panel(t(locale, UiText::AddressDetails))),
        area,
    );
}

fn label(name: &'static str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{name:<10}"), theme::muted()),
        Span::raw(value),
    ])
}
