use crate::i18n::UiText;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame,
};

use crate::{
    app::App,
    domain::{format_quil_amount, RewardIdentity, RewardMintPhase, RewardSnapshot},
    i18n::{t, Locale},
};

use super::{chrome, text::compact_middle, theme};

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let locale = app.locale();
    let footer = if app
        .reward_mint
        .status
        .as_ref()
        .is_some_and(|status| status.can_rebroadcast)
    {
        mt(locale, MintText::FooterRecovery)
    } else if app.reward_mint.status.is_none()
        && app.reward_snapshot.as_ref().is_some_and(|snapshot| {
            snapshot.proof_verified
                && snapshot
                    .claimable_base_units
                    .is_some_and(|amount| amount > 0)
        })
    {
        mt(locale, MintText::FooterReady)
    } else {
        mt(locale, MintText::FooterStatus)
    };
    let Some(screen) = chrome::screen(frame, t(locale, UiText::WalletRewards), app, footer) else {
        return;
    };

    let content = if app.reward_in_flight {
        Paragraph::new(t(locale, UiText::LoadingAndVerifyingRewardProof))
            .alignment(Alignment::Center)
            .block(chrome::panel(t(locale, UiText::ProverRewards)))
    } else if let Some(error) = &app.reward_error {
        Paragraph::new(crate::i18n::localize_service_error(locale, error).into_owned())
            .style(Style::default().fg(Color::Red))
            .wrap(Wrap { trim: true })
            .block(chrome::panel(t(locale, UiText::LoadFailed)))
    } else if let Some(snapshot) = &app.reward_snapshot {
        let width = usize::from(screen.body.width.saturating_sub(4));
        let mut lines = reward_lines(snapshot, width, locale);
        if app.reward_mint.status.is_some() {
            lines.truncate(4);
        } else if snapshot.proof_verified
            && snapshot
                .claimable_base_units
                .is_some_and(|amount| amount > 0)
        {
            lines.truncate(10);
        }
        lines.extend(mint_lines(app, width));
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(chrome::panel(t(locale, UiText::ProverRewards)))
    } else {
        Paragraph::new(t(locale, UiText::NoDataPressRToLoad))
            .alignment(Alignment::Center)
            .block(chrome::panel(t(locale, UiText::ProverRewards)))
    };
    frame.render_widget(content, screen.body);
}

pub fn draw_confirm(frame: &mut Frame<'_>, app: &App) {
    let locale = app.locale();
    let Some(screen) = chrome::screen(
        frame,
        mt(locale, MintText::ConfirmTitle),
        app,
        mt(locale, MintText::ConfirmFooter),
    ) else {
        return;
    };
    let Some(preflight) = app.reward_mint.preflight.as_ref() else {
        frame.render_widget(
            Paragraph::new(mt(locale, MintText::PreflightMissing))
                .alignment(Alignment::Center)
                .block(chrome::panel(mt(locale, MintText::ConfirmTitle))),
            screen.body,
        );
        return;
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(12), Constraint::Length(3)])
        .split(screen.body);
    let working = app.reward_mint.in_flight;
    let lines = vec![
        Line::from(Span::styled(
            format!(
                "{} {} QUIL",
                mt(locale, MintText::ClaimAll),
                format_quil_amount(preflight.reward_amount)
            ),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        label_value(locale, MintText::Network, &preflight.network),
        label_value(
            locale,
            MintText::Configuration,
            &preflight.config_dir.to_string_lossy(),
        ),
        label_value(
            locale,
            MintText::Recipient,
            &compact_middle(
                &preflight.recipient,
                usize::from(chunks[0].width).saturating_sub(18),
            ),
        ),
        label_value(
            locale,
            MintText::Fee,
            &format!("{} QUIL", format_quil_amount(preflight.estimated_fee)),
        ),
        label_value(
            locale,
            MintText::Net,
            &format!("{} QUIL", format_quil_amount(preflight.net_amount)),
        ),
        label_value(
            locale,
            MintText::Frame,
            &preflight.cited_global_frame.to_string(),
        ),
        Line::from(""),
        Line::from(mt(locale, MintText::TwoStages)),
        Line::from(Span::styled(
            mt(locale, MintText::NotFinality),
            theme::muted(),
        )),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(chrome::panel(mt(locale, MintText::Summary))),
        chunks[0],
    );
    let choice = if working {
        mt(locale, MintText::Working)
    } else if app.reward_mint.confirmation == crate::app::ConfirmChoice::Confirm {
        mt(locale, MintText::ChoiceConfirm)
    } else {
        mt(locale, MintText::ChoiceCancel)
    };
    frame.render_widget(
        Paragraph::new(choice)
            .alignment(Alignment::Center)
            .style(if working {
                theme::attention()
            } else {
                Style::default().add_modifier(Modifier::BOLD)
            })
            .block(chrome::panel(mt(locale, MintText::Action))),
        chunks[1],
    );
}

fn mint_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    let locale = app.locale();
    let mut lines = vec![Line::from("")];
    if let Some(error) = &app.reward_mint.error {
        lines.push(Line::from(Span::styled(
            crate::i18n::localize_service_error(locale, error).into_owned(),
            Style::default().fg(Color::Red),
        )));
        return lines;
    }
    let Some(status) = app.reward_mint.status.as_ref() else {
        if !app.reward_mint.in_flight {
            lines.push(Line::from(Span::styled(
                mt(locale, MintText::PrimaryAction),
                theme::success().add_modifier(Modifier::BOLD),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                mt(locale, MintText::Preparing),
                theme::attention(),
            )));
        }
        return lines;
    };
    lines.push(Line::from(Span::styled(
        mt(locale, MintText::Progress),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    for (phase, text) in [
        (
            RewardMintPhase::AuthorizationPrepared,
            MintText::Authorization,
        ),
        (
            RewardMintPhase::AwaitingAuthorizationWitness,
            MintText::Witness,
        ),
        (RewardMintPhase::ClaimPrepared, MintText::Claim),
        (RewardMintPhase::ClaimSubmitted, MintText::Finality),
    ] {
        let marker = progress_marker(status.phase, phase);
        lines.push(Line::from(format!("  {marker} {}", mt(locale, text))));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        mt(locale, MintText::SafeToClose),
        theme::muted(),
    )));
    lines.push(Line::from(format!(
        "{} {}",
        mt(locale, MintText::Receipt),
        compact_middle(&status.receipt, width.saturating_sub(12))
    )));
    if status.detail.is_some() {
        let detail = match status.phase {
            RewardMintPhase::AuthorizationUnknown | RewardMintPhase::ClaimUnknown => {
                mt(locale, MintText::ReadOnlyRecovery).to_owned()
            }
            RewardMintPhase::RecoverableError => mt(locale, MintText::Retrying).to_owned(),
            RewardMintPhase::ClaimAccepted => mt(locale, MintText::AcceptedWaiting).to_owned(),
            RewardMintPhase::ClaimRejected => mt(locale, MintText::RejectedRecovery).to_owned(),
            _ => status.detail.clone().unwrap_or_default(),
        };
        lines.push(Line::from(Span::styled(detail, theme::attention())));
    }
    if status.can_rebroadcast {
        lines.push(Line::from(Span::styled(
            if app.reward_mint.rebroadcast_armed {
                mt(locale, MintText::RecoveryConfirm)
            } else {
                mt(locale, MintText::RecoveryReady)
            },
            theme::attention().add_modifier(Modifier::BOLD),
        )));
    }
    lines
}

fn progress_marker(current: RewardMintPhase, target: RewardMintPhase) -> &'static str {
    let rank = |phase| match phase {
        RewardMintPhase::Preflighting | RewardMintPhase::ReadyToConfirm => 0,
        RewardMintPhase::ProvingAuthorization => 1,
        RewardMintPhase::AuthorizationPrepared | RewardMintPhase::SubmittingAuthorization => 2,
        RewardMintPhase::AwaitingAuthorizationWitness | RewardMintPhase::AuthorizationUnknown => 3,
        RewardMintPhase::ProvingClaim | RewardMintPhase::ClaimPrepared => 4,
        RewardMintPhase::SubmittingClaim
        | RewardMintPhase::ClaimSubmitted
        | RewardMintPhase::ClaimUnknown => 5,
        RewardMintPhase::ClaimAccepted => 6,
        RewardMintPhase::ClaimConfirmed => 7,
        RewardMintPhase::ClaimRejected
        | RewardMintPhase::RecoverableError
        | RewardMintPhase::PermanentError => 0,
    };
    if current == RewardMintPhase::ClaimConfirmed || rank(current) > rank(target) {
        "[x]"
    } else if rank(current) == rank(target) {
        "[>]"
    } else {
        "[ ]"
    }
}

fn label_value(locale: Locale, label: MintText, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{}  ", mt(locale, label)), theme::muted()),
        Span::raw(value.to_owned()),
    ])
}

#[derive(Clone, Copy)]
enum MintText {
    FooterReady,
    FooterStatus,
    FooterRecovery,
    ConfirmTitle,
    ConfirmFooter,
    PreflightMissing,
    ClaimAll,
    Network,
    Configuration,
    Recipient,
    CurrentWallet,
    RewardReadWarning,
    Fee,
    Net,
    Frame,
    TwoStages,
    NotFinality,
    Summary,
    Working,
    ChoiceConfirm,
    ChoiceCancel,
    Action,
    PrimaryAction,
    Preparing,
    Progress,
    Authorization,
    Witness,
    Claim,
    Finality,
    SafeToClose,
    Receipt,
    ReadOnlyRecovery,
    Retrying,
    AcceptedWaiting,
    RejectedRecovery,
    RecoveryReady,
    RecoveryConfirm,
}

fn mt(locale: Locale, key: MintText) -> &'static str {
    use Locale::*;
    use MintText::*;
    match (locale, key) {
        (English, FooterReady) => "M Claim all rewards   R Reload   Esc/Q Back   Ctrl+C Exit",
        (TraditionalChinese, FooterReady) => {
            "M 領取全部獎勵   R 重新載入   Esc/Q 返回   Ctrl+C 離開"
        }
        (Japanese, FooterReady) => "M 報酬を全て受取   R 再読込   Esc/Q 戻る   Ctrl+C 終了",
        (French, FooterReady) => "M Réclamer tout   R Recharger   Esc/Q Retour   Ctrl+C Quitter",
        (English, FooterStatus) => "R Reload   Esc/Q Back   Ctrl+C Exit",
        (TraditionalChinese, FooterStatus) => "R 重新載入   Esc/Q 返回   Ctrl+C 離開",
        (Japanese, FooterStatus) => "R 再読込   Esc/Q 戻る   Ctrl+C 終了",
        (French, FooterStatus) => "R Recharger   Esc/Q Retour   Ctrl+C Quitter",
        (English, FooterRecovery) => "B Recover saved claim   R Reload   Esc/Q Back   Ctrl+C Exit",
        (TraditionalChinese, FooterRecovery) => "B 復原已儲存領取   R 重新載入   Esc/Q 返回   Ctrl+C 離開",
        (Japanese, FooterRecovery) => "B 保存済み請求を復旧   R 再読込   Esc/Q 戻る   Ctrl+C 終了",
        (French, FooterRecovery) => "B Récupérer la réclamation   R Recharger   Esc/Q Retour   Ctrl+C Quitter",
        (English, ConfirmTitle) => "Confirm Reward Mint",
        (TraditionalChinese, ConfirmTitle) => "確認領取獎勵",
        (Japanese, ConfirmTitle) => "報酬 Mint の確認",
        (French, ConfirmTitle) => "Confirmer le Mint de récompense",
        (English, ConfirmFooter) => "Arrows Choose   Enter Continue   Esc Cancel   Ctrl+C Exit",
        (TraditionalChinese, ConfirmFooter) => "方向鍵 選擇   Enter 繼續   Esc 取消   Ctrl+C 離開",
        (Japanese, ConfirmFooter) => "矢印 選択   Enter 続行   Esc キャンセル   Ctrl+C 終了",
        (French, ConfirmFooter) => {
            "Flèches Choisir   Entrée Continuer   Esc Annuler   Ctrl+C Quitter"
        }
        (English, PreflightMissing) => {
            "Mint preflight is no longer available. Return and try again."
        }
        (TraditionalChinese, PreflightMissing) => "Mint 預檢已失效，請返回後重試。",
        (Japanese, PreflightMissing) => "Mint 事前確認が失効しました。戻って再試行してください。",
        (French, PreflightMissing) => "La prévalidation Mint a expiré. Revenez et réessayez.",
        (English, ClaimAll) => "Claim",
        (TraditionalChinese, ClaimAll) => "領取",
        (Japanese, ClaimAll) => "受取",
        (French, ClaimAll) => "Réclamer",
        (English, Network) => "Network",
        (TraditionalChinese, Network) => "網路",
        (Japanese, Network) => "ネットワーク",
        (French, Network) => "Réseau",
        (English, Configuration) => "Configuration",
        (TraditionalChinese, Configuration) => "設定目錄",
        (Japanese, Configuration) => "設定ディレクトリ",
        (French, Configuration) => "Configuration",
        (English, Recipient) => "Recipient (current wallet)",
        (TraditionalChinese, Recipient) => "收款（目前錢包）",
        (Japanese, Recipient) => "受取先（現在のウォレット）",
        (French, Recipient) => "Destinataire (portefeuille actuel)",
        (English, CurrentWallet) => "Current confidential wallet",
        (TraditionalChinese, CurrentWallet) => "目前機密錢包",
        (Japanese, CurrentWallet) => "現在の機密ウォレット",
        (French, CurrentWallet) => "Portefeuille confidentiel actuel",
        (English, RewardReadWarning) => "Reward state unavailable; check the read RPC",
        (TraditionalChinese, RewardReadWarning) => "獎勵狀態暫不可用；請檢查唯讀 RPC",
        (Japanese, RewardReadWarning) => "報酬状態を取得できません。読取 RPC を確認してください",
        (French, RewardReadWarning) => "État de récompense indisponible ; vérifiez le RPC de lecture",
        (English, Fee) => "Estimated fee",
        (TraditionalChinese, Fee) => "預估費用",
        (Japanese, Fee) => "推定手数料",
        (French, Fee) => "Frais estimés",
        (English, Net) => "Estimated net",
        (TraditionalChinese, Net) => "預估到帳",
        (Japanese, Net) => "推定受取額",
        (French, Net) => "Montant net estimé",
        (English, Frame) => "Cited GLOBAL Frame",
        (TraditionalChinese, Frame) => "引用 GLOBAL Frame",
        (Japanese, Frame) => "参照 GLOBAL Frame",
        (French, Frame) => "Frame GLOBAL cité",
        (English, TwoStages) => {
            "One confirmation starts both protocol stages automatically: authorization, then claim."
        }
        (TraditionalChinese, TwoStages) => {
            "確認一次後，Console 會自動完成 authorization 與 claim 兩個協議階段。"
        }
        (Japanese, TwoStages) => "1 回の確認で authorization と claim の両段階を自動実行します。",
        (French, TwoStages) => "Une confirmation lance automatiquement authorization puis claim.",
        (English, NotFinality) => {
            "Node acceptance is not finality; completion requires the claimed output on chain."
        }
        (TraditionalChinese, NotFinality) => {
            "節點接受不代表最終確認；必須在鏈上看到領取輸出才算完成。"
        }
        (Japanese, NotFinality) => {
            "ノード受理は確定ではありません。チェーン上の出力確認で完了します。"
        }
        (French, NotFinality) => {
            "L’acceptation du nœud n’est pas finale ; la sortie doit être observée sur chaîne."
        }
        (English, Summary) => "Mint Summary",
        (TraditionalChinese, Summary) => "Mint 摘要",
        (Japanese, Summary) => "Mint 概要",
        (French, Summary) => "Résumé Mint",
        (English, Working) => "Preparing proof and submitting… input is locked",
        (TraditionalChinese, Working) => "正在產生證明並提交…輸入已鎖定",
        (Japanese, Working) => "証明を生成して送信中…入力はロックされています",
        (French, Working) => "Preuve et envoi en cours… saisie verrouillée",
        (English, ChoiceConfirm) => "[ Confirm and claim all ]    Cancel",
        (TraditionalChinese, ChoiceConfirm) => "[ 確認並領取全部 ]    取消",
        (Japanese, ChoiceConfirm) => "[ 確認して全て受取 ]    キャンセル",
        (French, ChoiceConfirm) => "[ Confirmer et tout réclamer ]    Annuler",
        (English, ChoiceCancel) => "Confirm and claim all    [ Cancel (default) ]",
        (TraditionalChinese, ChoiceCancel) => "確認並領取全部    [ 取消（預設） ]",
        (Japanese, ChoiceCancel) => "確認して全て受取    [ キャンセル（既定） ]",
        (French, ChoiceCancel) => "Confirmer et tout réclamer    [ Annuler (défaut) ]",
        (English, Action) => "Action",
        (TraditionalChinese, Action) => "操作",
        (Japanese, Action) => "操作",
        (French, Action) => "Action",
        (English, PrimaryAction) => "M  Claim all rewards to the current confidential wallet",
        (TraditionalChinese, PrimaryAction) => "M  領取全部獎勵到目前機密錢包",
        (Japanese, PrimaryAction) => "M  報酬を全て現在の機密ウォレットへ受取",
        (French, PrimaryAction) => {
            "M  Réclamer toutes les récompenses vers le portefeuille confidentiel actuel"
        }
        (English, Preparing) => "Preparing the latest reward, fee, and recipient…",
        (TraditionalChinese, Preparing) => "正在重新確認獎勵、費用與收款錢包…",
        (Japanese, Preparing) => "最新の報酬、手数料、受取先を確認中…",
        (French, Preparing) => "Vérification de la récompense, des frais et du destinataire…",
        (English, Progress) => "Reward Mint progress",
        (TraditionalChinese, Progress) => "獎勵 Mint 進度",
        (Japanese, Progress) => "報酬 Mint の進行状況",
        (French, Progress) => "Progression du Mint de récompense",
        (English, Authorization) => "Generate and submit authorization",
        (TraditionalChinese, Authorization) => "產生並提交 authorization",
        (Japanese, Authorization) => "authorization を生成・送信",
        (French, Authorization) => "Générer et envoyer authorization",
        (English, Witness) => "Wait for GLOBAL authorization witness",
        (TraditionalChinese, Witness) => "等待 GLOBAL authorization witness",
        (Japanese, Witness) => "GLOBAL authorization witness を待機",
        (French, Witness) => "Attendre le witness GLOBAL",
        (English, Claim) => "Generate and submit claim",
        (TraditionalChinese, Claim) => "產生並提交 claim",
        (Japanese, Claim) => "claim を生成・送信",
        (French, Claim) => "Générer et envoyer claim",
        (English, Finality) => "Observe claimed output on chain",
        (TraditionalChinese, Finality) => "在鏈上確認領取輸出",
        (Japanese, Finality) => "チェーン上の受取出力を確認",
        (French, Finality) => "Observer la sortie réclamée sur chaîne",
        (English, SafeToClose) => {
            "Safe to close: this configuration resumes safely; unknown submissions are never resent."
        }
        (TraditionalChinese, SafeToClose) => {
            "可以安全關閉：重新開啟會安全續行；結果未知的提交絕不重送。"
        }
        (Japanese, SafeToClose) => {
            "安全に終了できます。再開時も安全に続行し、結果不明の送信は再送しません。"
        }
        (French, SafeToClose) => {
            "Vous pouvez fermer : reprise sûre, sans renvoi d’une soumission au résultat inconnu."
        }
        (English, Receipt) => "Receipt",
        (TraditionalChinese, Receipt) => "Receipt",
        (Japanese, Receipt) => "Receipt",
        (French, Receipt) => "Receipt",
        (English, ReadOnlyRecovery) => {
            "Submission outcome is unknown; checking chain state only and never resending."
        }
        (TraditionalChinese, ReadOnlyRecovery) => {
            "提交結果未知；目前只查詢鏈上狀態，絕不重新送出。"
        }
        (Japanese, ReadOnlyRecovery) => {
            "送信結果は不明です。チェーン状態のみ確認し、再送信はしません。"
        }
        (French, ReadOnlyRecovery) => {
            "Résultat d’envoi inconnu : vérification en lecture seule, sans nouvel envoi."
        }
        (English, Retrying) => "Temporary issue; retrying the safe step automatically.",
        (TraditionalChinese, Retrying) => "暫時發生問題；正在自動重試安全步驟。",
        (Japanese, Retrying) => "一時的な問題です。安全な手順を自動再試行します。",
        (French, Retrying) => "Problème temporaire : nouvelle tentative automatique et sûre.",
        (English, AcceptedWaiting) => "Claim accepted on GLOBAL; Coin is not visible yet. Verify the configured read RPC.",
        (TraditionalChinese, AcceptedWaiting) => "GLOBAL 已接受領取，但 Coin 尚不可見；請確認目前的唯讀 RPC。",
        (Japanese, AcceptedWaiting) => "GLOBAL が請求を受理しましたが Coin は未反映です。読取 RPC を確認してください。",
        (French, AcceptedWaiting) => "Réclamation acceptée sur GLOBAL, mais Coin reste invisible. Vérifiez le RPC de lecture.",
        (English, RejectedRecovery) => "The saved claim was rejected on chain and may be rebuilt from its authorization.",
        (TraditionalChinese, RejectedRecovery) => "已儲存的領取在鏈上遭拒，可從原授權重新建立。",
        (Japanese, RejectedRecovery) => "保存済み請求はチェーンで拒否されました。元の承認から再構築できます。",
        (French, RejectedRecovery) => "La réclamation enregistrée a été rejetée et peut être reconstruite depuis son autorisation.",
        (English, RecoveryReady) => "B  Prepare manual recovery (requires a second B confirmation)",
        (TraditionalChinese, RecoveryReady) => "B  準備手動復原（需要再按一次 B 確認）",
        (Japanese, RecoveryReady) => "B  手動復旧を準備（もう一度 B で確認）",
        (French, RecoveryReady) => "B  Préparer la récupération manuelle (confirmer avec un second B)",
        (English, RecoveryConfirm) => "Press B again to rebuild and rebroadcast the saved claim; Esc cancels.",
        (TraditionalChinese, RecoveryConfirm) => "再按一次 B 以重建並重新廣播已儲存領取；Esc 取消。",
        (Japanese, RecoveryConfirm) => "もう一度 B を押して保存済み請求を再構築・再送信します。Esc で取消。",
        (French, RecoveryConfirm) => "Appuyez encore sur B pour reconstruire et rediffuser ; Esc annule.",
    }
}

fn reward_lines(snapshot: &RewardSnapshot, width: usize, locale: Locale) -> Vec<Line<'static>> {
    match &snapshot.identity {
        RewardIdentity::Unavailable => {
            return vec![
                Line::from(Span::styled(
                    t(locale, UiText::ThisNodeHasNoFalconProverIdentity),
                    Style::default().add_modifier(Modifier::BOLD),
                )),
                Line::from(t(locale, UiText::RewardLookupDoesNotApplyTheConsoleNeverCreatesOrMigratesNodeKeysAutomatically)),
                Line::from(""),
                Line::from(Span::styled(t(locale, UiText::LegacyAccountAddressEd448), theme::muted())),
                Line::from(
                    snapshot
                        .legacy_account_address_hex
                        .clone()
                        .map(|value| compact_middle(&value, width))
                        .unwrap_or_else(|| t(locale, UiText::None).into()),
                ),
            ];
        }
        RewardIdentity::Ambiguous { candidates } => {
            return vec![
                Line::from(Span::styled(
                    t(locale, UiText::AmbiguousProverIdentity),
                    Style::default().add_modifier(Modifier::BOLD),
                )),
                Line::from(format!(
                    "{} {candidates} {}",
                    t(locale, UiText::Detected),
                    t(
                        locale,
                        UiText::Falcon512CandidateKeysTheConsoleWillNotGuessRewardOwnership
                    )
                )),
                Line::from(t(
                    locale,
                    UiText::ConfirmTheProverIdentityActuallyUsedByTheNodeFirst,
                )),
            ];
        }
        RewardIdentity::Standard | RewardIdentity::Alternate { .. } => {}
    }

    let amount = snapshot
        .claimable_base_units
        .map(|value| format!("{} QUIL", format_quil_amount(value)))
        .unwrap_or_else(|| {
            t(
                locale,
                UiText::UnavailableNodeReturnedNoClaimableRewardProof,
            )
            .into()
        });
    let frame = snapshot
        .cited_frame
        .map(|value| value.to_string())
        .unwrap_or_else(|| "—".into());
    let verification = if snapshot.proof_verified {
        Span::styled(
            t(locale, UiText::ProofVerified),
            theme::success().add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(t(locale, UiText::NoVerifiableProof), theme::muted())
    };
    let legacy_account = snapshot
        .legacy_account_address_hex
        .clone()
        .unwrap_or_else(|| {
            t(
                locale,
                UiText::NoneCurrentConfigurationIsNotALegacyEd448Identity,
            )
            .into()
        });
    let identity = match &snapshot.identity {
        RewardIdentity::Standard => "q-prover-key".into(),
        RewardIdentity::Alternate { key_id } => {
            format!("{key_id} ({})", t(locale, UiText::AutoDetected))
        }
        RewardIdentity::Unavailable | RewardIdentity::Ambiguous { .. } => unreachable!(),
    };
    let mut lines = vec![
        Line::from(vec![
            Span::styled(t(locale, UiText::ProverIdentity), theme::muted()),
            Span::raw(identity),
        ]),
        Line::from(vec![
            Span::styled(t(locale, UiText::ClaimableReward), theme::muted()),
            Span::styled(amount, Style::default().add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled(t(locale, UiText::ProofStatus), theme::muted()),
            verification,
        ]),
        Line::from(vec![
            Span::styled(t(locale, UiText::CitedGlobalFrame), theme::muted()),
            Span::raw(frame),
        ]),
    ];
    if let Some(address) = snapshot.recipient_address_hex.as_deref() {
        lines.push(Line::from(vec![
            Span::styled(
                format!("{}  ", mt(locale, MintText::CurrentWallet)),
                theme::muted(),
            ),
            Span::raw(compact_middle(address, width.saturating_sub(24))),
        ]));
    }
    if let Some(error) = snapshot.witness_error.as_deref() {
        lines.extend([
            Line::from(""),
            Line::from(Span::styled(
                mt(locale, MintText::RewardReadWarning),
                theme::attention().add_modifier(Modifier::BOLD),
            )),
            Line::from(compact_middle(error, width)),
        ]);
    }
    lines.extend([
        Line::from(Span::styled(
            t(locale, UiText::ProverRewardAddress),
            theme::muted(),
        )),
        Line::from(
            snapshot
                .prover_address_hex
                .as_deref()
                .map_or_else(|| "—".into(), |value| compact_middle(value, width)),
        ),
        Line::from(""),
        Line::from(Span::styled(
            t(locale, UiText::LegacyAccountAddressEd448),
            theme::muted(),
        )),
        Line::from(compact_middle(&legacy_account, width)),
        Line::from(""),
        Line::from(vec![
            Span::styled(t(locale, UiText::FalconPublicKey), theme::muted()),
            Span::raw(
                snapshot
                    .public_key_hex
                    .as_deref()
                    .map(|value| compact_middle(value, width.min(36)))
                    .unwrap_or_else(|| "—".into()),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            t(locale, UiText::Conversion8000000000BaseUnits1QUIL),
            theme::muted(),
        )),
    ]);
    lines
}
