use crate::i18n::UiText;
use ratatui::{
    layout::{Constraint, Layout},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::{app::App, i18n::t};

use super::{
    chrome,
    text::{tail_cells, truncate_cells},
    theme,
};
use unicode_width::UnicodeWidthStr;

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let locale = app.locale();
    let footer = if app.diagnostics_editing {
        t(locale, UiText::EnterAddressEnterDiagnoseEscCancelCtrlCExit)
    } else {
        t(locale, UiText::EnterEditRetryEscQBackCtrlCExit)
    };
    let Some(screen) = chrome::screen(frame, t(locale, UiText::NetworkDiagnostics), app, footer)
    else {
        return;
    };
    let rows = Layout::vertical([Constraint::Length(3), Constraint::Min(4)]).split(screen.body);

    let input_width = usize::from(rows[0].width.saturating_sub(2));
    let value = if app.diagnostics_address.is_empty() && !app.diagnostics_editing {
        "/ip4/127.0.0.1/tcp/8336".into()
    } else if app.diagnostics_editing {
        tail_cells(&app.diagnostics_address, input_width)
    } else {
        truncate_cells(&app.diagnostics_address, input_width)
    };
    let cursor = UnicodeWidthStr::width(value.as_str()).min(input_width);
    let input_style = if app.diagnostics_editing {
        Style::default().fg(theme::ACCENT)
    } else {
        theme::normal()
    };
    frame.render_widget(
        Paragraph::new(value).style(input_style).block(
            Block::default()
                .title(t(locale, UiText::NodeMultiaddr))
                .borders(Borders::ALL),
        ),
        rows[0],
    );

    let result_width = usize::from(rows[1].width.saturating_sub(2));
    let result = if app.diagnostics_in_flight {
        vec![Line::from(Span::styled(
            t(locale, UiText::DialingAndWaitingForIdentifyPing),
            theme::title(),
        ))]
    } else if let Some(error) = &app.diagnostics_error {
        vec![
            Line::from(Span::styled(
                t(locale, UiText::DiagnosticsFailed),
                Style::default().fg(Color::Red),
            )),
            Line::from(crate::i18n::localize_service_error(locale, error).into_owned()),
        ]
    } else if let Some(report) = &app.diagnostics_report {
        let mut lines = vec![
            Line::from(Span::styled(
                t(locale, UiText::ConnectionSucceeded),
                theme::success(),
            )),
            field("Peer ID", &report.peer_id, result_width),
            field(
                t(locale, UiText::Transport),
                &report.transport,
                result_width,
            ),
            field(
                t(locale, UiText::ConnectionTime),
                &format!("{} ms", report.connection_ms),
                result_width,
            ),
            field(
                "Ping",
                &report
                    .ping_ms
                    .map(|value| format!("{value} ms"))
                    .unwrap_or_else(|| t(locale, UiText::NotReturned).into()),
                result_width,
            ),
            field(
                t(locale, UiText::NodeVersion),
                report
                    .agent_version
                    .as_deref()
                    .unwrap_or(t(locale, UiText::NotReturned)),
                result_width,
            ),
            field(
                t(locale, UiText::ProtocolVersion),
                report
                    .protocol_version
                    .as_deref()
                    .unwrap_or(t(locale, UiText::NotReturned)),
                result_width,
            ),
            field(
                "Observed",
                report
                    .observed_address
                    .as_deref()
                    .unwrap_or(t(locale, UiText::NotReturned)),
                result_width,
            ),
            counted_field(
                t(locale, UiText::ListenAddresses),
                report.listen_addresses.len(),
                "",
                result_width,
                locale,
            ),
        ];
        for (index, address) in report.listen_addresses.iter().take(3).enumerate() {
            lines.push(field(
                &format!("{} {}", t(locale, UiText::ListenAddress), index + 1),
                &format!(
                    "{}  {}",
                    address.reach.localized_label(locale),
                    address.multiaddr
                ),
                result_width,
            ));
        }
        let protocols = if report.protocols.is_empty() {
            t(locale, UiText::NotReturned).into()
        } else {
            report
                .protocols
                .iter()
                .take(4)
                .cloned()
                .collect::<Vec<_>>()
                .join("  ")
        };
        lines.push(counted_field(
            t(locale, UiText::Protocols),
            report.protocols.len(),
            &protocols,
            result_width,
            locale,
        ));
        lines
    } else {
        vec![
            Line::from(t(locale, UiText::PressEnterToEditTheAddressThenEnterAgainToDiagnose)),
            Line::from(t(locale, UiText::ChecksLibp2pMultiaddrIdentifyAndPingDirectly)),
            Line::from(Span::styled(
                t(locale, UiText::DiagnosticsUseATemporaryIdentityNeverReadNodePrivateKeysAndNeverModifyConfiguration),
                theme::muted(),
            )),
        ]
    };
    let result = Paragraph::new(result).block(chrome::panel(t(locale, UiText::DiagnosticResults)));
    let result = if app.diagnostics_report.is_some()
        && app.diagnostics_error.is_none()
        && !app.diagnostics_in_flight
    {
        result
    } else {
        result.wrap(Wrap { trim: true })
    };
    frame.render_widget(result, rows[1]);

    if app.diagnostics_editing {
        frame.set_cursor_position((rows[0].x + 1 + cursor as u16, rows[0].y + 1));
    }
}

fn field(label: &str, value: &str, width: usize) -> Line<'static> {
    let prefix = format!("{label}：");
    let value_width = width.saturating_sub(UnicodeWidthStr::width(prefix.as_str()));
    Line::from(vec![
        Span::styled(prefix, theme::muted()),
        Span::raw(truncate_cells(value, value_width)),
    ])
}

fn counted_field(
    label: &str,
    count: usize,
    value: &str,
    width: usize,
    locale: crate::i18n::Locale,
) -> Line<'static> {
    let prefix = format!("{label} {count} {}", t(locale, UiText::EmptySuffix));
    let gap = usize::from(!value.is_empty()) * 2;
    let value_width = width.saturating_sub(UnicodeWidthStr::width(prefix.as_str()) + gap);
    Line::from(vec![
        Span::styled(prefix, theme::muted()),
        Span::raw(" ".repeat(gap)),
        Span::raw(truncate_cells(value, value_width)),
    ])
}
