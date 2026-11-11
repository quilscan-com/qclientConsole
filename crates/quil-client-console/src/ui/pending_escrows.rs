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
    domain::{format_quil_amount, PendingEscrowsSnapshot},
    i18n::{t, Locale},
};

use super::{chrome, text::compact_middle, theme, wallet_operations};

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let locale = app.locale();
    let base_footer = if app.narrow_read_detail {
        t(locale, UiText::EscQBackToListCtrlCQuit)
    } else if app.viewport_width <= 69
        && app
            .pending_escrows
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.identity_available && !snapshot.escrows.is_empty())
    {
        t(locale, UiText::SelectEnterDetailsRRescanEscQBackCtrlCQuit)
    } else {
        t(locale, UiText::SelectRRescanEscQBackCtrlCQuit)
    };
    let selected = app
        .pending_escrows
        .snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.escrows.get(app.pending_escrows.index));
    let action_hint = selected.and_then(|escrow| match escrow.branch {
        crate::domain::PendingEscrowBranch::Recipient => Some(match locale {
            Locale::TraditionalChinese => "A 接受",
            Locale::Japanese => "A 承認",
            Locale::French => "A Accepter",
            Locale::English => "A Accept",
        }),
        crate::domain::PendingEscrowBranch::Refund => {
            let eligible = app.snapshot.as_ref().is_some_and(|snapshot| {
                snapshot.last_global_head_frame.saturating_sub(
                    quil_execution::token_intrinsic::constants::GLOBAL_ANCHOR_SAFETY_MARGIN,
                ) >= escrow.refund_after_global_frame
            });
            eligible.then_some(match locale {
                Locale::TraditionalChinese => "X 退款",
                Locale::Japanese => "X 返金",
                Locale::French => "X Rembourser",
                Locale::English => "X Refund",
            })
        }
    });
    let footer = action_hint
        .map(|hint| format!("{hint}  {base_footer}"))
        .unwrap_or_else(|| base_footer.to_owned());
    let Some(screen) = chrome::screen(frame, t(locale, UiText::PendingTransfers), app, &footer)
    else {
        return;
    };

    if app.pending_escrows.in_flight {
        frame.render_widget(
            centered(t(locale, UiText::ScanningPendingTransfers), locale),
            screen.body,
        );
    } else if let Some(error) = &app.pending_escrows.error {
        frame.render_widget(
            Paragraph::new(crate::i18n::localize_service_error(locale, error).into_owned())
                .style(Style::default().fg(Color::Red))
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true })
                .block(chrome::panel(t(locale, UiText::LoadFailedTitle))),
            screen.body,
        );
    } else if let Some(snapshot) = &app.pending_escrows.snapshot {
        if app.narrow_read_detail {
            draw_detail(
                frame,
                snapshot,
                app.pending_escrows.index,
                locale,
                screen.body,
            );
        } else {
            draw_snapshot(
                frame,
                snapshot,
                app.pending_escrows.index,
                locale,
                screen.body,
            );
        }
    } else {
        frame.render_widget(
            centered(t(locale, UiText::NoSnapshotPressRToLoad), locale),
            screen.body,
        );
    }
}

fn centered(text: &'static str, locale: Locale) -> Paragraph<'static> {
    Paragraph::new(text)
        .alignment(Alignment::Center)
        .block(chrome::panel(t(locale, UiText::TransferStatus)))
}

fn draw_snapshot(
    frame: &mut Frame<'_>,
    snapshot: &PendingEscrowsSnapshot,
    index: usize,
    locale: Locale,
    area: Rect,
) {
    if !snapshot.recent_operations.is_empty() {
        let rows = Layout::vertical([
            Constraint::Min(3),
            Constraint::Length(wallet_operations::panel_height(
                snapshot.recent_operations.len(),
            )),
        ])
        .split(area);
        draw_escrows(frame, snapshot, index, locale, rows[0]);
        wallet_operations::draw(frame, &snapshot.recent_operations, locale, rows[1]);
        return;
    }
    draw_escrows(frame, snapshot, index, locale, area);
}

fn draw_escrows(
    frame: &mut Frame<'_>,
    snapshot: &PendingEscrowsSnapshot,
    index: usize,
    locale: Locale,
    area: Rect,
) {
    if !snapshot.identity_available {
        frame.render_widget(
            Paragraph::new(snapshot.unavailable_reason.as_deref().unwrap_or(t(
                locale,
                UiText::TheCurrentConfigurationHasNoUsableProverIdentity,
            )))
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true })
            .block(chrome::panel(t(locale, UiText::CurrentIdentity))),
            area,
        );
        return;
    }
    if snapshot.escrows.is_empty() {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(t(
                    locale,
                    UiText::TheCurrentNodeReportsNoPendingTransfersForThisProver,
                )),
                Line::from(Span::styled(
                    t(
                        locale,
                        UiText::ThisIsTheNodeSCurrentStateThisPageDoesNotAcceptOrRefundTransfers,
                    ),
                    theme::muted(),
                )),
            ])
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true })
            .block(chrome::panel(t(locale, UiText::PendingTransfers))),
            area,
        );
        return;
    }

    let selected = index.min(snapshot.escrows.len().saturating_sub(1));
    if area.width >= 78 {
        let columns = Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(area);
        draw_list(frame, snapshot, selected, locale, columns[0]);
        draw_detail(frame, snapshot, selected, locale, columns[1]);
    } else {
        draw_list(frame, snapshot, selected, locale, area);
    }
}

fn draw_detail(
    frame: &mut Frame<'_>,
    snapshot: &PendingEscrowsSnapshot,
    index: usize,
    locale: Locale,
    area: Rect,
) {
    let Some(escrow) = snapshot.escrows.get(index) else {
        frame.render_widget(
            Paragraph::new(t(locale, UiText::NoTransfers))
                .block(chrome::panel(t(locale, UiText::TransferDetails))),
            area,
        );
        return;
    };
    let detail_width = usize::from(area.width.saturating_sub(2));
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                t(locale, UiText::EscrowAddress),
                theme::muted(),
            )),
            Line::from(compact_middle(
                &format!("0x{}", escrow.address_hex),
                detail_width,
            )),
            Line::from(""),
            Line::from(format!(
                "{}  {}",
                t(locale, UiText::Branch),
                escrow.branch.localized_label(locale)
            )),
            Line::from(format!(
                "{}  {} QUIL",
                t(locale, UiText::Amount),
                format_quil_amount(escrow.amount_base_units)
            )),
            Line::from(format!("{} base units", escrow.amount_base_units)),
            Line::from(format!(
                "{}  {}",
                t(locale, UiText::RefundStartsAtGlobalFrame),
                escrow.refund_after_global_frame
            )),
        ])
        .block(chrome::panel(t(locale, UiText::TransferDetails))),
        area,
    );
}

fn draw_list(
    frame: &mut Frame<'_>,
    snapshot: &PendingEscrowsSnapshot,
    index: usize,
    locale: Locale,
    area: Rect,
) {
    let items = snapshot.escrows.iter().map(|escrow| {
        ListItem::new(format!(
            "{}  {}  {} QUIL",
            compact_middle(&escrow.address_hex, 18),
            escrow.branch.localized_label(locale),
            format_quil_amount(escrow.amount_base_units)
        ))
    });
    let mut state = ListState::default().with_selected(Some(index));
    frame.render_stateful_widget(
        List::new(items)
            .block(chrome::panel(format!(
                "{} {}",
                t(locale, UiText::PendingTransfers),
                snapshot.escrows.len()
            )))
            .highlight_style(theme::selected())
            .highlight_symbol("› "),
        area,
        &mut state,
    );
}
