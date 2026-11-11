use ratatui::{
    layout::{Alignment, Constraint, Layout, Margin},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::{
    app::{App, ConfirmChoice, Page},
    domain::{WalletAction, WalletOperationStatus},
    i18n::Locale,
};

use super::{chrome, text::compact_middle, theme};

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    match app.page {
        Page::WalletActions => draw_actions(frame, app),
        Page::WalletForm => draw_form(frame, app),
        Page::WalletConfirm => draw_confirm(frame, app),
        Page::WalletProgress => draw_progress(frame, app),
        Page::WalletResult => draw_result(frame, app),
        _ => {}
    }
}

fn draw_actions(frame: &mut Frame<'_>, app: &App) {
    let Some(screen) = chrome::screen(
        frame,
        words(app, "Wallet actions", "錢包操作"),
        app,
        words(
            app,
            "↑↓ Select  Enter Open  Esc/Q Back  Ctrl+C Exit",
            "↑↓ 選擇  Enter 開啟  Esc/Q 返回  Ctrl+C 離開",
        ),
    ) else {
        return;
    };
    let mut state = ListState::default().with_selected(Some(app.wallet.action_index.min(1)));
    frame.render_stateful_widget(
        List::new([
            ListItem::new(words(app, "Transfer confidential QUIL", "轉帳機密 QUIL")),
            ListItem::new(words(
                app,
                "Pending transfer with Accept / Reject",
                "可接受／退款的待處理轉帳",
            )),
        ])
        .block(chrome::panel(words(app, "Available now", "目前可用")))
        .highlight_style(theme::selected())
        .highlight_symbol("› "),
        screen.body,
        &mut state,
    );
}

fn draw_form(frame: &mut Frame<'_>, app: &App) {
    let footer = if app.wallet.in_flight {
        words(
            app,
            "Checking wallet and fee… Ctrl+C Exit",
            "正在檢查錢包與手續費… Ctrl+C 離開",
        )
    } else if app.wallet.editing {
        words(
            app,
            "Type value  Enter/Tab Done  Esc Cancel edit",
            "輸入內容  Enter/Tab 完成  Esc 取消編輯",
        )
    } else {
        words(
            app,
            "↑↓ Select  Enter Edit/Continue  Esc/Q Back",
            "↑↓ 選擇  Enter 編輯/繼續  Esc/Q 返回",
        )
    };
    let title = match app.wallet.action {
        WalletAction::Transfer => words(app, "New transfer", "新增轉帳"),
        WalletAction::PendingTransfer => words(app, "New pending transfer", "新增待處理轉帳"),
        WalletAction::Accept => words(app, "Accept pending transfer", "接受待處理轉帳"),
        WalletAction::Reject => words(app, "Refund pending transfer", "退款待處理轉帳"),
    };
    let Some(screen) = chrome::screen(frame, title, app, footer) else {
        return;
    };
    let width = usize::from(screen.body.width.saturating_sub(24)).max(8);
    let mut fields = vec![
        (
            words(app, "Application", "應用"),
            compact_middle(&app.wallet.form_application, width),
        ),
        (
            if matches!(
                app.wallet.action,
                WalletAction::Accept | WalletAction::Reject
            ) {
                words(app, "Selected Escrow", "已選 Escrow")
            } else {
                words(app, "Recipient", "收款地址")
            },
            compact_middle(&app.wallet.form_target, width),
        ),
        (
            words(app, "Amount (base units)", "金額（base units）"),
            if matches!(
                app.wallet.action,
                WalletAction::Accept | WalletAction::Reject
            ) {
                words(app, "Derived from selected Escrow", "由所選 Escrow 決定").into()
            } else {
                app.wallet.form_amount.clone()
            },
        ),
        (
            words(app, "Explicit fee (blank=auto)", "指定手續費（留空=自動）"),
            app.wallet.form_explicit_fee.clone(),
        ),
        (
            words(app, "Maximum fee", "手續費上限"),
            app.wallet.form_maximum_fee.clone(),
        ),
    ];
    if app.wallet.action == WalletAction::PendingTransfer {
        fields.push((
            words(
                app,
                "Refund global frame (blank=auto)",
                "退款 Global Frame（留空=自動）",
            ),
            app.wallet.form_refund_frame.clone(),
        ));
    }
    fields.push((
        words(app, "Continue", "繼續"),
        words(app, "Review fee and inputs", "檢查手續費與輸入").into(),
    ));
    let last_field = fields.len().saturating_sub(1);
    let items = fields
        .into_iter()
        .enumerate()
        .map(|(index, (label, value))| {
            let marker = if app.wallet.editing && app.wallet.form_field == index {
                "✎"
            } else {
                " "
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{marker} {label:<25}"), theme::muted()),
                Span::raw(value),
            ]))
        });
    let mut state = ListState::default().with_selected(Some(app.wallet.form_field.min(last_field)));
    frame.render_stateful_widget(
        List::new(items)
            .block(chrome::panel(words(app, "Transfer details", "轉帳資料")))
            .highlight_style(theme::selected())
            .highlight_symbol("› "),
        screen.body,
        &mut state,
    );
    if let Some(error) = &app.wallet.error {
        let area = screen.body.inner(Margin::new(2, 1));
        let row = ratatui::layout::Rect::new(
            area.x,
            area.bottom().saturating_sub(2),
            area.width,
            2.min(area.height),
        );
        frame.render_widget(
            Paragraph::new(error.as_str())
                .style(Style::default().fg(Color::Red))
                .wrap(Wrap { trim: true }),
            row,
        );
    }
}

fn draw_confirm(frame: &mut Frame<'_>, app: &App) {
    let title = match app.wallet.action {
        WalletAction::Transfer => words(app, "Confirm transfer", "確認轉帳"),
        WalletAction::PendingTransfer => words(app, "Confirm pending transfer", "確認待處理轉帳"),
        WalletAction::Accept => words(app, "Confirm acceptance", "確認接受轉帳"),
        WalletAction::Reject => words(app, "Confirm refund", "確認退款"),
    };
    let Some(screen) = chrome::screen(
        frame,
        title,
        app,
        words(
            app,
            "←→ Select  Enter Continue  Esc/Q Cancel",
            "←→ 選擇  Enter 繼續  Esc/Q 取消",
        ),
    ) else {
        return;
    };
    let Some(summary) = &app.wallet.preflight else {
        frame.render_widget(
            Paragraph::new(words(app, "No preflight result", "沒有預檢結果")),
            screen.body,
        );
        return;
    };
    let [details, choices] = Layout::vertical([Constraint::Min(4), Constraint::Length(2)])
        .areas(screen.body.inner(Margin::new(1, 1)));
    let mut lines = vec![
        Line::from(format!(
            "{}  {}",
            words(app, "Application", "應用"),
            summary.application_hex
        )),
        Line::from(format!(
            "{}  {}",
            if matches!(summary.action, WalletAction::Accept | WalletAction::Reject) {
                words(app, "Escrow", "Escrow")
            } else {
                words(app, "Recipient", "收款地址")
            },
            summary.target
        )),
        Line::from(format!(
            "{}  {}",
            words(app, "Amount", "金額"),
            summary.amount_base_units
        )),
        Line::from(format!(
            "{}  {}",
            words(app, "Quoted fee", "預估手續費"),
            summary.fee_base_units
        )),
        Line::from(format!(
            "{}  {}",
            words(app, "Maximum fee", "手續費上限"),
            summary.maximum_fee_base_units
        )),
        Line::from(format!(
            "{}  {}",
            if matches!(summary.action, WalletAction::Accept | WalletAction::Reject) {
                words(app, "Claim output", "領取輸出")
            } else {
                words(app, "Change", "找零")
            },
            summary.change_base_units
        )),
        Line::from(format!(
            "{}  {}",
            words(app, "Inputs", "輸入 Coin 數"),
            summary.input_count
        )),
    ];
    if let Some(frame) = summary.refund_after_global_frame {
        lines.push(Line::from(format!(
            "{}  {frame}",
            words(app, "Refund global frame", "退款 Global Frame")
        )));
    }
    lines.push(Line::from(Span::styled(
        words(
            app,
            "Submitted does not mean finalized.",
            "已提交不代表已最終確認。",
        ),
        theme::attention(),
    )));
    frame.render_widget(
        Paragraph::new(lines)
            .block(chrome::panel(words(
                app,
                "Review exact spend",
                "檢查實際支出",
            )))
            .wrap(Wrap { trim: false }),
        details,
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            choice(
                words(app, "Cancel (default)", "取消（預設）"),
                app.wallet.confirmation == ConfirmChoice::Cancel,
            ),
            Span::raw("    "),
            choice(
                words(app, "Prove and submit", "建立證明並提交"),
                app.wallet.confirmation == ConfirmChoice::Confirm,
            ),
        ])),
        choices,
    );
}

fn draw_progress(frame: &mut Frame<'_>, app: &App) {
    let Some(screen) = chrome::screen(
        frame,
        words(app, "Transfer in progress", "轉帳處理中"),
        app,
        words(
            app,
            "Proof generation may take several minutes  Ctrl+C Exit",
            "建立證明可能需要數分鐘  Ctrl+C 離開",
        ),
    ) else {
        return;
    };
    frame.render_widget(
        Paragraph::new(words(
            app,
            "Scanning current Coins, fetching canonical witnesses, generating and verifying the native proof, then submitting once.",
            "正在掃描最新 Coin、取得規範見證、建立並驗證原生證明，之後只提交一次。",
        ))
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true })
        .block(chrome::panel(words(app, "Please wait", "請稍候"))),
        screen.body,
    );
}

fn draw_result(frame: &mut Frame<'_>, app: &App) {
    let Some(screen) = chrome::screen(
        frame,
        wallet_result_page_title(app.locale()),
        app,
        wallet_result_footer(app.locale()),
    ) else {
        return;
    };
    let (title, message, style) = if let Some(result) = &app.wallet.result {
        match result.operation.phase {
            WalletOperationStatus::Submitting => (
                wallet_result_status(app.locale(), WalletOperationStatus::Submitting),
                result.message.as_str(),
                theme::attention(),
            ),
            WalletOperationStatus::Submitted => (
                wallet_result_status(app.locale(), WalletOperationStatus::Submitted),
                result.message.as_str(),
                theme::healthy(),
            ),
            WalletOperationStatus::Unknown => (
                wallet_result_status(app.locale(), WalletOperationStatus::Unknown),
                result.message.as_str(),
                theme::attention(),
            ),
            WalletOperationStatus::Committed => (
                wallet_result_status(app.locale(), WalletOperationStatus::Committed),
                result.message.as_str(),
                theme::healthy(),
            ),
            WalletOperationStatus::Rejected => (
                wallet_result_status(app.locale(), WalletOperationStatus::Rejected),
                result.message.as_str(),
                theme::failure(),
            ),
        }
    } else {
        (
            words(
                app,
                "Failed before confirmed submission",
                "在確認提交前失敗",
            ),
            app.wallet.error.as_deref().unwrap_or("Unknown error"),
            theme::failure(),
        )
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(title, style)),
            Line::from(""),
            Line::from(message),
        ])
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true })
        .block(chrome::panel(wallet_result_panel_title(app.locale()))),
        screen.body,
    );
}

fn wallet_result_page_title(locale: Locale) -> &'static str {
    match locale {
        Locale::English => "Transfer result",
        Locale::TraditionalChinese => "轉帳結果",
        Locale::Japanese => "送金結果",
        Locale::French => "Résultat du transfert",
    }
}

fn wallet_result_footer(locale: Locale) -> &'static str {
    match locale {
        Locale::English => "Enter/Esc Back  R Refresh wallet  Ctrl+C Exit",
        Locale::TraditionalChinese => "Enter/Esc 返回  R 重新整理錢包  Ctrl+C 離開",
        Locale::Japanese => "Enter/Esc 戻る  R ウォレット更新  Ctrl+C 終了",
        Locale::French => "Entrée/Échap Retour  R Actualiser  Ctrl+C Quitter",
    }
}

fn wallet_result_panel_title(locale: Locale) -> &'static str {
    match locale {
        Locale::English => "Outcome",
        Locale::TraditionalChinese => "結果",
        Locale::Japanese => "結果",
        Locale::French => "Résultat",
    }
}

fn wallet_result_status(locale: Locale, status: WalletOperationStatus) -> &'static str {
    match (locale, status) {
        (Locale::TraditionalChinese, WalletOperationStatus::Submitting) => "提交中",
        (Locale::TraditionalChinese, WalletOperationStatus::Submitted) => "已提交",
        (Locale::TraditionalChinese, WalletOperationStatus::Unknown) => "結果未知",
        (Locale::TraditionalChinese, WalletOperationStatus::Committed) => "已確認",
        (Locale::TraditionalChinese, WalletOperationStatus::Rejected) => "已拒絕",
        (Locale::Japanese, WalletOperationStatus::Submitting) => "送信中",
        (Locale::Japanese, WalletOperationStatus::Submitted) => "送信済み",
        (Locale::Japanese, WalletOperationStatus::Unknown) => "結果不明",
        (Locale::Japanese, WalletOperationStatus::Committed) => "確定済み",
        (Locale::Japanese, WalletOperationStatus::Rejected) => "拒否",
        (Locale::French, WalletOperationStatus::Submitting) => "Envoi en cours",
        (Locale::French, WalletOperationStatus::Submitted) => "Envoyée",
        (Locale::French, WalletOperationStatus::Unknown) => "Résultat inconnu",
        (Locale::French, WalletOperationStatus::Committed) => "Confirmée",
        (Locale::French, WalletOperationStatus::Rejected) => "Rejetée",
        (Locale::English, WalletOperationStatus::Submitting) => "Submitting",
        (Locale::English, WalletOperationStatus::Submitted) => "Submitted",
        (Locale::English, WalletOperationStatus::Unknown) => "Result unknown",
        (Locale::English, WalletOperationStatus::Committed) => "Committed",
        (Locale::English, WalletOperationStatus::Rejected) => "Rejected",
    }
}

fn choice(label: &'static str, selected: bool) -> Span<'static> {
    Span::styled(
        if selected {
            format!("› [ {label} ]")
        } else {
            format!("  [ {label} ]")
        },
        if selected {
            theme::focus()
        } else {
            theme::muted()
        },
    )
}

fn words(app: &App, english: &'static str, chinese: &'static str) -> &'static str {
    match app.locale() {
        Locale::TraditionalChinese => chinese,
        Locale::English | Locale::Japanese | Locale::French => english,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wallet_result_status_and_chrome_are_localized_in_all_four_languages() {
        for (locale, status, page, footer) in [
            (
                Locale::English,
                "Committed",
                "Transfer result",
                "Refresh wallet",
            ),
            (
                Locale::TraditionalChinese,
                "已確認",
                "轉帳結果",
                "重新整理錢包",
            ),
            (Locale::Japanese, "確定済み", "送金結果", "ウォレット更新"),
            (
                Locale::French,
                "Confirmée",
                "Résultat du transfert",
                "Actualiser",
            ),
        ] {
            assert_eq!(
                wallet_result_status(locale, WalletOperationStatus::Committed),
                status
            );
            assert_eq!(wallet_result_page_title(locale), page);
            assert!(wallet_result_footer(locale).contains(footer));
            assert!(!wallet_result_panel_title(locale).is_empty());
        }
    }
}
