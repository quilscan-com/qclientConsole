use ratatui::{
    layout::Rect,
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame,
};

use crate::{
    domain::{WalletAction, WalletOperationSnapshot, WalletOperationStatus},
    i18n::Locale,
};

use super::{chrome, text::compact_middle, theme};

const DISPLAY_LIMIT: usize = 3;

pub(super) fn panel_height(count: usize) -> u16 {
    count.min(DISPLAY_LIMIT) as u16 + 2
}

pub(super) fn draw(
    frame: &mut Frame<'_>,
    operations: &[WalletOperationSnapshot],
    locale: Locale,
    area: Rect,
) {
    let width = usize::from(area.width.saturating_sub(4));
    let lines = operations.iter().take(DISPLAY_LIMIT).map(|operation| {
        let txid = operation.operation_id.chars().take(10).collect::<String>();
        let prefix = format!(
            "{}  {txid}…  {}",
            action_label(operation.action, locale),
            status_label(operation.phase, locale)
        );
        let detail = operation.detail.as_deref().unwrap_or_default();
        let text = if detail.is_empty() {
            prefix
        } else {
            format!(
                "{prefix}  {}",
                compact_middle(detail, width.saturating_sub(prefix.len() + 2))
            )
        };
        Line::from(vec![Span::styled(text, status_style(operation.phase))])
    });
    frame.render_widget(
        Paragraph::new(lines.collect::<Vec<_>>())
            .wrap(Wrap { trim: true })
            .block(chrome::panel(title(locale))),
        area,
    );
}

fn title(locale: Locale) -> &'static str {
    match locale {
        Locale::English => "Recent wallet operations",
        Locale::TraditionalChinese => "最近錢包操作",
        Locale::Japanese => "最近のウォレット操作",
        Locale::French => "Opérations récentes",
    }
}

fn action_label(action: WalletAction, locale: Locale) -> &'static str {
    match (locale, action) {
        (Locale::TraditionalChinese, WalletAction::Transfer) => "轉帳",
        (Locale::TraditionalChinese, WalletAction::PendingTransfer) => "待處理轉帳",
        (Locale::TraditionalChinese, WalletAction::Accept) => "接受",
        (Locale::TraditionalChinese, WalletAction::Reject) => "退款",
        (Locale::Japanese, WalletAction::Transfer) => "送金",
        (Locale::Japanese, WalletAction::PendingTransfer) => "保留送金",
        (Locale::Japanese, WalletAction::Accept) => "承認",
        (Locale::Japanese, WalletAction::Reject) => "返金",
        (Locale::French, WalletAction::Transfer) => "Transfert",
        (Locale::French, WalletAction::PendingTransfer) => "Transfert différé",
        (Locale::French, WalletAction::Accept) => "Accepter",
        (Locale::French, WalletAction::Reject) => "Rembourser",
        (Locale::English, WalletAction::Transfer) => "Transfer",
        (Locale::English, WalletAction::PendingTransfer) => "Pending transfer",
        (Locale::English, WalletAction::Accept) => "Accept",
        (Locale::English, WalletAction::Reject) => "Refund",
    }
}

fn status_label(status: WalletOperationStatus, locale: Locale) -> &'static str {
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
        (Locale::French, WalletOperationStatus::Submitting) => "Envoi",
        (Locale::French, WalletOperationStatus::Submitted) => "Envoyée",
        (Locale::French, WalletOperationStatus::Unknown) => "Résultat inconnu",
        (Locale::French, WalletOperationStatus::Committed) => "Confirmée",
        (Locale::French, WalletOperationStatus::Rejected) => "Rejetée",
        (Locale::English, WalletOperationStatus::Submitting) => "Submitting",
        (Locale::English, WalletOperationStatus::Submitted) => "Submitted",
        (Locale::English, WalletOperationStatus::Unknown) => "Unknown",
        (Locale::English, WalletOperationStatus::Committed) => "Committed",
        (Locale::English, WalletOperationStatus::Rejected) => "Rejected",
    }
}

fn status_style(status: WalletOperationStatus) -> ratatui::style::Style {
    match status {
        WalletOperationStatus::Committed => theme::healthy(),
        WalletOperationStatus::Rejected => theme::failure(),
        WalletOperationStatus::Submitting
        | WalletOperationStatus::Submitted
        | WalletOperationStatus::Unknown => theme::attention(),
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{backend::TestBackend, Terminal};

    use super::*;

    #[test]
    fn recent_operation_panel_is_localized_in_all_four_languages() {
        let operation = WalletOperationSnapshot {
            operation_id: "1234567890abcdef".repeat(4),
            action: WalletAction::Transfer,
            application_hex: "ab".repeat(32),
            phase: WalletOperationStatus::Committed,
            submitted_unix_ms: Some(42),
            detail: Some("Committed at GLOBAL frame 88".into()),
            console_version: "0.1.0".into(),
            build_commit: "test".into(),
        };
        for (locale, expected) in [
            (
                Locale::English,
                ["Recent wallet operations", "Transfer", "Committed"],
            ),
            (
                Locale::TraditionalChinese,
                ["最近錢包操作", "轉帳", "已確認"],
            ),
            (
                Locale::Japanese,
                ["最近のウォレット操作", "送金", "確定済み"],
            ),
            (
                Locale::French,
                ["Opérations récentes", "Transfert", "Confirmée"],
            ),
        ] {
            let mut terminal = Terminal::new(TestBackend::new(100, 6)).unwrap();
            terminal
                .draw(|frame| {
                    let area = frame.area();
                    draw(frame, std::slice::from_ref(&operation), locale, area);
                })
                .unwrap();
            let buffer = terminal.backend().buffer();
            let text = (0..buffer.area.height)
                .flat_map(|y| {
                    (0..buffer.area.width).map(move |x| buffer[(x, y)].symbol().to_owned())
                })
                .collect::<String>();
            let compact_text = text.replace(' ', "");
            for value in expected {
                assert!(
                    compact_text.contains(&value.replace(' ', "")),
                    "{locale:?} did not render {value}: {text}"
                );
            }
            assert!(text.contains("1234567890"));
            assert!(text.contains("GLOBAL frame 88"));
        }
    }

    #[test]
    fn all_operation_phases_have_four_language_labels() {
        let phases = [
            WalletOperationStatus::Submitting,
            WalletOperationStatus::Submitted,
            WalletOperationStatus::Unknown,
            WalletOperationStatus::Committed,
            WalletOperationStatus::Rejected,
        ];
        for locale in [
            Locale::English,
            Locale::TraditionalChinese,
            Locale::Japanese,
            Locale::French,
        ] {
            for phase in phases {
                assert!(!status_label(phase, locale).is_empty());
            }
        }
    }
}
