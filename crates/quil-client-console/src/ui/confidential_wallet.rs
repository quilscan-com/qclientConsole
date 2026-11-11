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
    domain::{format_quil_amount, ConfidentialWalletSnapshot},
    i18n::{t, Locale},
};

use super::{chrome, text::compact_middle, theme, wallet_operations};

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let locale = app.locale();
    let footer: std::borrow::Cow<'_, str> = if app.narrow_read_detail {
        t(locale, UiText::EscQBackToListCtrlCQuit).into()
    } else if app.viewport_width <= 69 {
        match locale {
            Locale::TraditionalChinese => {
                "A 錢包操作  Enter 詳情  R 重掃  Esc/Q 返回  Ctrl+C 離開".into()
            }
            Locale::Japanese => "A 操作  Enter 詳細  R 再読込  Esc/Q 戻る  Ctrl+C 終了".into(),
            Locale::French => {
                "A Opérations  Entrée détails  R relire  Échap/Q retour  Ctrl+C quitter".into()
            }
            Locale::English => {
                "A Wallet actions  Enter details  R Rescan  Esc/Q Back  Ctrl+C Exit".into()
            }
        }
    } else {
        match locale {
            Locale::TraditionalChinese => {
                "A 錢包操作  ↑↓ 選擇 Coin  R 重新掃描  Esc/Q 返回  Ctrl+C 離開".into()
            }
            _ => "A Wallet actions  ↑↓ Select Coin  R Rescan  Esc/Q Back  Ctrl+C Exit".into(),
        }
    };
    let Some(screen) = chrome::screen(
        frame,
        t(locale, UiText::ConfidentialWallet),
        app,
        footer.as_ref(),
    ) else {
        return;
    };

    if app.confidential_wallet.in_flight {
        frame.render_widget(
            centered(
                t(locale, UiText::ScanningCoinsAndCheckingUnspentStatus),
                locale,
            ),
            screen.body,
        );
    } else if let Some(error) = &app.confidential_wallet.error {
        frame.render_widget(
            Paragraph::new(crate::i18n::localize_service_error(locale, error).into_owned())
                .style(Style::default().fg(Color::Red))
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true })
                .block(chrome::panel(t(locale, UiText::LoadFailedTitle))),
            screen.body,
        );
    } else if let Some(snapshot) = &app.confidential_wallet.snapshot {
        if app.narrow_read_detail {
            draw_coin_detail(
                frame,
                snapshot,
                app.confidential_wallet.coin_index,
                locale,
                screen.body,
            );
        } else {
            if snapshot.recent_operations.is_empty() {
                let rows = Layout::vertical([Constraint::Length(4), Constraint::Min(3)])
                    .split(screen.body);
                draw_summary(frame, snapshot, locale, rows[0]);
                draw_coins(
                    frame,
                    snapshot,
                    app.confidential_wallet.coin_index,
                    locale,
                    rows[1],
                );
            } else {
                let rows = Layout::vertical([
                    Constraint::Length(4),
                    Constraint::Min(3),
                    Constraint::Length(wallet_operations::panel_height(
                        snapshot.recent_operations.len(),
                    )),
                ])
                .split(screen.body);
                draw_summary(frame, snapshot, locale, rows[0]);
                draw_coins(
                    frame,
                    snapshot,
                    app.confidential_wallet.coin_index,
                    locale,
                    rows[1],
                );
                wallet_operations::draw(frame, &snapshot.recent_operations, locale, rows[2]);
            }
        }
    } else {
        frame.render_widget(
            centered(
                t(
                    locale,
                    UiText::NoWalletSnapshotPressRToLoadReadingDoesNotSubmitTransactions,
                ),
                locale,
            ),
            screen.body,
        );
    }
}

fn centered(text: &'static str, locale: Locale) -> Paragraph<'static> {
    Paragraph::new(text)
        .alignment(Alignment::Center)
        .block(chrome::panel(t(locale, UiText::WalletStatus)))
}

fn draw_summary(
    frame: &mut Frame<'_>,
    snapshot: &ConfidentialWalletSnapshot,
    locale: Locale,
    area: Rect,
) {
    let address_width = usize::from(area.width.saturating_sub(14));
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(
                    t(locale, UiText::NodeReportedUnspentBalance),
                    theme::muted(),
                ),
                Span::styled(
                    format!("{} QUIL", format_quil_amount(snapshot.total_base_units)),
                    theme::title(),
                ),
                Span::raw(format!(
                    "    {} {}",
                    snapshot.coins.len(),
                    t(locale, UiText::Coins)
                )),
            ]),
            Line::from(vec![
                Span::styled(t(locale, UiText::ReceiveAddress), theme::muted()),
                Span::raw(compact_middle(&snapshot.address_hex, address_width)),
            ]),
        ])
        .block(chrome::panel(t(locale, UiText::WalletOverview))),
        area,
    );
}

fn draw_coins(
    frame: &mut Frame<'_>,
    snapshot: &ConfidentialWalletSnapshot,
    index: usize,
    locale: Locale,
    area: Rect,
) {
    if snapshot.coins.is_empty() {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(t(
                    locale,
                    UiText::TheCurrentNodeReportsNoUnspentCoinsForThisWallet,
                )),
                Line::from(Span::styled(
                    t(
                        locale,
                        UiText::ThisReflectsTheConfiguredNodeSCurrentStateNotAFinalOnChainBalance,
                    ),
                    theme::muted(),
                )),
            ])
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true })
            .block(chrome::panel(t(locale, UiText::UnspentCoins))),
            area,
        );
        return;
    }
    let selected = index.min(snapshot.coins.len().saturating_sub(1));
    if area.width >= 78 {
        let columns = Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(area);
        draw_coin_list(frame, snapshot, selected, locale, columns[0]);
        draw_coin_detail(frame, snapshot, selected, locale, columns[1]);
    } else {
        draw_coin_list(frame, snapshot, selected, locale, area);
    }
}

fn draw_coin_detail(
    frame: &mut Frame<'_>,
    snapshot: &ConfidentialWalletSnapshot,
    index: usize,
    locale: Locale,
    area: Rect,
) {
    let Some(coin) = snapshot.coins.get(index) else {
        frame.render_widget(
            Paragraph::new(t(locale, UiText::NoCoins))
                .block(chrome::panel(t(locale, UiText::CoinDetails))),
            area,
        );
        return;
    };
    let width = usize::from(area.width.saturating_sub(2));
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(t(locale, UiText::CoinAddress), theme::muted())),
            Line::from(compact_middle(&format!("0x{}", coin.address_hex), width)),
            Line::from(""),
            Line::from(Span::styled(t(locale, UiText::Amount), theme::muted())),
            Line::from(format!(
                "{} QUIL",
                format_quil_amount(coin.amount_base_units)
            )),
            Line::from(format!("{} base units", coin.amount_base_units)),
        ])
        .block(chrome::panel(t(locale, UiText::CoinDetails))),
        area,
    );
}

fn draw_coin_list(
    frame: &mut Frame<'_>,
    snapshot: &ConfidentialWalletSnapshot,
    index: usize,
    locale: Locale,
    area: Rect,
) {
    let items = snapshot.coins.iter().map(|coin| {
        ListItem::new(format!(
            "{}  {} QUIL",
            compact_middle(&coin.address_hex, 18),
            format_quil_amount(coin.amount_base_units)
        ))
    });
    let mut state = ListState::default().with_selected(Some(index));
    frame.render_stateful_widget(
        List::new(items)
            .block(chrome::panel(t(locale, UiText::UnspentCoins)))
            .highlight_style(theme::selected())
            .highlight_symbol("› "),
        area,
        &mut state,
    );
}
