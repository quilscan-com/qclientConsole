use crate::allocation::{ActionHint, ActionTiming, AllocationAction, AllocationLifecycleStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MessageKey {
    ProductName,
    MainTitle,
    MenuOverview,
    MenuNodeManagement,
    MenuProverShards,
    MenuWalletToken,
    MenuIdentity,
    MenuDataApps,
    MenuDiagnosticsLogs,
    MenuCapabilities,
    MenuConfigs,
    MenuRuntime,
    MenuProver,
    MenuShards,
    MenuArchiveShards,
    MenuRewards,
    MenuTransfer,
    MenuConfidentialWallet,
    MenuPendingEscrows,
    MenuLegacyAssets,
    MenuHypergraph,
    MenuDeployToken,
    MenuDiagnostics,
    MenuDiagnosticsIdentity,
    MenuRemoteHealth,
    MenuHealth,
    MenuPeerResolve,
    MenuLogs,
    MenuLogExport,
    MenuKeys,
    MenuAliases,
    MenuOther,
    MenuHelp,
    StatusConnected,
    StatusDisconnected,
    StatusLoading,
    StatusNoData,
    FooterMain,
}

pub fn text(locale: Locale, key: MessageKey) -> &'static str {
    t(locale, message_text(key))
}

const fn message_text(key: MessageKey) -> UiText {
    match key {
        MessageKey::ProductName => UiText::QUILIBRIUM,
        MessageKey::MainTitle => UiText::QuilibriumConsole,
        MessageKey::MenuOverview => UiText::Overview,
        MessageKey::MenuNodeManagement => UiText::NodeManagement,
        MessageKey::MenuProverShards => UiText::ProverShards,
        MessageKey::MenuWalletToken => UiText::WalletToken,
        MessageKey::MenuIdentity => UiText::KeysAddresses,
        MessageKey::MenuDataApps => UiText::DataApplications,
        MessageKey::MenuDiagnosticsLogs => UiText::DiagnosticsLogs,
        MessageKey::MenuCapabilities => UiText::QclientCommands,
        MessageKey::MenuConfigs => UiText::NodeConfiguration,
        MessageKey::MenuRuntime => UiText::RuntimeUpdates,
        MessageKey::MenuProver => UiText::ProverAllocation,
        MessageKey::MenuShards => UiText::ShardsRewards,
        MessageKey::MenuArchiveShards => UiText::ArchiveConsistency,
        MessageKey::MenuRewards => UiText::ClaimableRewards,
        MessageKey::MenuTransfer => UiText::TransferTokens,
        MessageKey::MenuConfidentialWallet => UiText::ConfidentialWallet,
        MessageKey::MenuPendingEscrows => UiText::PendingTransfers,
        MessageKey::MenuLegacyAssets => UiText::LegacyAssets,
        MessageKey::MenuHypergraph => UiText::Hypergraph,
        MessageKey::MenuDeployToken => UiText::DeployToken,
        MessageKey::MenuDiagnostics => UiText::NetworkDial,
        MessageKey::MenuDiagnosticsIdentity => UiText::DiagnosticIdentity,
        MessageKey::MenuRemoteHealth => UiText::RemoteHealth,
        MessageKey::MenuHealth => UiText::NodeHealthReport,
        MessageKey::MenuPeerResolve => UiText::ResolvePeerAddress,
        MessageKey::MenuLogs => UiText::NodeLogs,
        MessageKey::MenuLogExport => UiText::RedactedLogExportTitle,
        MessageKey::MenuKeys => UiText::Keys,
        MessageKey::MenuAliases => UiText::AddressAliases,
        MessageKey::MenuOther => UiText::Other,
        MessageKey::MenuHelp => UiText::Help,
        MessageKey::StatusConnected => UiText::Connected,
        MessageKey::StatusDisconnected => UiText::Disconnected,
        MessageKey::StatusLoading => UiText::LoadingStatus,
        MessageKey::StatusNoData => UiText::NoData,
        MessageKey::FooterMain => UiText::SelectEnterOpenEscQBackRRefreshHelpCtrlCExit,
    }
}

pub const fn t(locale: Locale, key: UiText) -> &'static str {
    catalog::get(locale, key)
}

pub fn capability_required_error(locale: Locale, label: &str) -> String {
    match locale {
        Locale::English => format!("{label} is required"),
        Locale::TraditionalChinese => format!("{label} 為必填欄位"),
        Locale::Japanese => format!("{label} は必須です"),
        Locale::French => format!("{label} est obligatoire"),
    }
}

pub fn capability_unsigned_error(locale: Locale, label: &str) -> String {
    match locale {
        Locale::English => format!("{label} must be an unsigned integer"),
        Locale::TraditionalChinese => format!("{label} 必須是非負整數"),
        Locale::Japanese => format!("{label} には符号なし整数を入力してください"),
        Locale::French => format!("{label} doit être un entier non signé"),
    }
}

pub fn capability_boolean_error(locale: Locale, label: &str) -> String {
    match locale {
        Locale::English => format!("{label} must be true or false"),
        Locale::TraditionalChinese => format!("{label} 必須是 true 或 false"),
        Locale::Japanese => format!("{label} は true または false にしてください"),
        Locale::French => format!("{label} doit valoir true ou false"),
    }
}

pub fn seat_concentration_warning(locale: Locale, at_risk: usize, reported: usize) -> String {
    match locale {
        Locale::English => format!(
            "[!] {at_risk}/{reported} committees depend on this site's margin; spread provers across sites"
        ),
        Locale::TraditionalChinese => format!(
            "[!] {at_risk}/{reported} 個委員會依賴本站點的安全餘量；請將 Prover 分散到不同站點"
        ),
        Locale::Japanese => format!(
            "[!] {at_risk}/{reported} 委員会がこのサイトの余裕に依存しています。Prover を別サイトへ分散してください"
        ),
        Locale::French => format!(
            "[!] {at_risk}/{reported} comités dépendent de la marge de ce site ; répartissez les provers"
        ),
    }
}

pub fn capability_route_error(locale: Locale, route: &str) -> String {
    match locale {
        Locale::English => format!("This Console page is unavailable: {route}"),
        Locale::TraditionalChinese => format!("此 Console 頁面無法使用：{route}"),
        Locale::Japanese => format!("この Console ページは利用できません：{route}"),
        Locale::French => format!("Cette page Console est indisponible : {route}"),
    }
}

pub fn allocation_lifecycle_status_label(
    locale: Locale,
    status: AllocationLifecycleStatus,
) -> &'static str {
    let key = match status {
        AllocationLifecycleStatus::Unknown => UiText::Unknown,
        AllocationLifecycleStatus::Joining => UiText::Joining,
        AllocationLifecycleStatus::Activating => UiText::Activating,
        AllocationLifecycleStatus::Active => UiText::ActiveStatus,
        AllocationLifecycleStatus::Paused => UiText::Paused,
        AllocationLifecycleStatus::Leaving => UiText::Leaving,
        AllocationLifecycleStatus::Departing => UiText::Departing,
        AllocationLifecycleStatus::Rejected => UiText::Rejected,
        AllocationLifecycleStatus::Kicked => UiText::Kicked,
        AllocationLifecycleStatus::Historic => UiText::Historic,
        AllocationLifecycleStatus::ExpiredJoining => UiText::ExpiredJoining,
        AllocationLifecycleStatus::ExpiredLeaving => UiText::ExpiredLeaving,
        AllocationLifecycleStatus::ExpiredEpoch => UiText::ExpiredEpoch,
    };
    t(locale, key)
}

pub fn allocation_action_label(locale: Locale, action: AllocationAction) -> &'static str {
    let key = match action {
        AllocationAction::Reject => UiText::Reject,
        AllocationAction::Confirm => UiText::Confirm,
        AllocationAction::Pause => UiText::Pause,
        AllocationAction::Resume => UiText::Resume,
        AllocationAction::Leave => UiText::Leave,
        AllocationAction::Renew => UiText::Renew,
        AllocationAction::Activate => UiText::Activate,
        AllocationAction::Depart => UiText::Depart,
        AllocationAction::Expire => UiText::Expire,
    };
    t(locale, key)
}

pub fn format_allocation_action_hint(locale: Locale, hint: &ActionHint, localized: bool) -> String {
    let actions = hint
        .actions
        .iter()
        .map(|action| {
            if localized {
                allocation_action_label(locale, *action)
            } else {
                raw_allocation_action_label(*action)
            }
        })
        .collect::<Vec<_>>()
        .join("/");
    match hint.timing {
        ActionTiming::Now => format!("{actions} · {}", t(locale, UiText::Now)),
        ActionTiming::At {
            frame,
            frames_remaining,
        } => format!(
            "{actions} · Frame {frame} · {} {frames_remaining}",
            t(locale, UiText::FramesRemaining),
        ),
        ActionTiming::Unknown => format!("{actions} · {}", t(locale, UiText::TimeUnknown)),
    }
}

const fn raw_allocation_action_label(action: AllocationAction) -> &'static str {
    match action {
        AllocationAction::Reject => "reject",
        AllocationAction::Confirm => "confirm",
        AllocationAction::Pause => "pause",
        AllocationAction::Resume => "resume",
        AllocationAction::Leave => "leave",
        AllocationAction::Renew => "renew",
        AllocationAction::Activate => "activate",
        AllocationAction::Depart => "depart",
        AllocationAction::Expire => "expire",
    }
}

pub fn localize_legacy_migration_warning(locale: Locale, warning: &str) -> String {
    const OUTSIDE_LIVE_SHARDS: &str = " legacy outputs are outside current live shards";
    let detail = if let Some(count) = warning.strip_suffix(OUTSIDE_LIVE_SHARDS) {
        format!(
            "{count}{}",
            t(locale, UiText::LegacyMigrationWarningOutsideLiveShards)
        )
    } else {
        warning.to_owned()
    };
    format!("{}{detail}", t(locale, UiText::LegacyMigrationWarningLabel))
}

pub fn localize_legacy_migration_error(locale: Locale, error: &str) -> String {
    if error == "Recipient config path is required" {
        return t(locale, UiText::LegacyMigrationRecipientConfigPathRequired).into();
    }

    let invalid_detail = error.strip_prefix("Invalid input: ").unwrap_or(error);
    let stable_key = match invalid_detail {
        "old peer key must contain exactly 114 bytes" => Some(UiText::LegacyMigrationOldKeyLength),
        "old peer key is not hex" => Some(UiText::LegacyMigrationOldKeyNotHex),
        "old peer seed and public key do not match" => {
            Some(UiText::LegacyMigrationOldKeyPairMismatch)
        }
        "This old identity has no unshielded QUIL" => Some(UiText::LegacyMigrationNoUnshieldedQuil),
        "No legacy QUIL is currently covered by a live shard" => {
            Some(UiText::LegacyMigrationNoLiveShardCoveredQuil)
        }
        "Legacy QUIL amount overflow" => Some(UiText::LegacyMigrationAmountOverflow),
        "Migration fee overflow" => Some(UiText::LegacyMigrationFeeOverflow),
        "Legacy QUIL does not cover migration fees" => {
            Some(UiText::LegacyMigrationFeesExceedAmount)
        }
        "Migration approval expired" => Some(UiText::LegacyMigrationApprovalExpired),
        "Migration approval contents changed" => Some(UiText::LegacyMigrationApprovalChanged),
        "Old key does not belong to this migration" => Some(UiText::LegacyMigrationOldKeyMismatch),
        "Invalid migration source" => Some(UiText::LegacyMigrationInvalidSource),
        "Invalid legacy migration fingerprint" => Some(UiText::LegacyMigrationInvalidFingerprint),
        "Migration has no recoverable saved submission" => {
            Some(UiText::LegacyMigrationNoRecoverableSubmission)
        }
        "A pending migration source is no longer available" => {
            Some(UiText::LegacyMigrationPendingSourceUnavailable)
        }
        _ => None,
    };
    if let Some(key) = stable_key {
        return t(locale, key).into();
    }
    if error
        .strip_prefix("Cannot read node configuration: ")
        .is_some_and(|detail| detail == "Configuration path is not a directory")
        || error == "Configuration path is not a directory"
    {
        return t(locale, UiText::LegacyMigrationConfigPathNotDirectory).into();
    }

    let with_detail = |key, detail: &str| format!("{}{detail}", t(locale, key));
    for (prefix, key) in [
        (
            "Cannot resolve migration directory: ",
            UiText::LegacyMigrationCannotResolveDirectory,
        ),
        (
            "File viewer task failed: ",
            UiText::LegacyMigrationFileTaskFailed,
        ),
    ] {
        if let Some(detail) = error.strip_prefix(prefix) {
            return with_detail(key, detail);
        }
    }
    for name in ["config.yml", "keys.yml"] {
        for (action, key) in [
            ("Cannot inspect", UiText::LegacyMigrationCannotInspectFile),
            ("Cannot read", UiText::LegacyMigrationCannotReadFile),
            ("Cannot resolve", UiText::LegacyMigrationCannotResolveFile),
        ] {
            let prefix = format!("{action} {name}: ");
            if let Some(detail) = error.strip_prefix(&prefix) {
                return with_detail(key, &format!("{name}: {detail}"));
            }
        }
        if error == format!("{name} is not a regular file") {
            return with_detail(UiText::LegacyMigrationFileNotRegular, name);
        }
        if error == format!("{name} is outside the migration directory") {
            return with_detail(UiText::LegacyMigrationFileOutsideDirectory, name);
        }
    }

    let service_error = localize_service_error(locale, error);
    if service_error != error {
        return service_error.into_owned();
    }

    format!(
        "{}: {}",
        t(locale, UiText::LegacyMigrationErrorCategory),
        error
    )
}

pub fn localize_service_error<'a>(locale: Locale, error: &'a str) -> std::borrow::Cow<'a, str> {
    if locale == Locale::English {
        return error.into();
    }

    const PREFIXES: [(&str, &str, &str, &str); 19] = [
        (
            "Invalid input: ",
            "輸入無效：",
            "入力が無効です：",
            "Entrée non valide : ",
        ),
        (
            "Cannot read node configuration: ",
            "無法讀取節點設定：",
            "ノード設定を読み込めません：",
            "Impossible de lire la configuration du nœud : ",
        ),
        (
            "Cannot connect to node: ",
            "無法連線至節點：",
            "ノードに接続できません：",
            "Impossible de se connecter au nœud : ",
        ),
        (
            "Node status query failed: ",
            "節點狀態查詢失敗：",
            "ノード状態の取得に失敗しました：",
            "Échec de la lecture de l’état du nœud : ",
        ),
        (
            "Cannot read node key: ",
            "無法讀取節點金鑰：",
            "ノードキーを読み込めません：",
            "Impossible de lire la clé du nœud : ",
        ),
        (
            "Operation submission failed: ",
            "操作提交失敗：",
            "操作の送信に失敗しました：",
            "Échec de l’envoi de l’opération : ",
        ),
        (
            "Network diagnostics failed: ",
            "網路診斷失敗：",
            "ネットワーク診断に失敗しました：",
            "Échec du diagnostic réseau : ",
        ),
        (
            "Diagnostic identity operation failed: ",
            "診斷身分操作失敗：",
            "診断 ID 操作に失敗しました：",
            "Échec de l’opération d’identité de diagnostic : ",
        ),
        (
            "Cannot read node health report: ",
            "無法讀取節點健康報告：",
            "ノード健全性レポートを読み込めません：",
            "Impossible de lire le rapport de santé du nœud : ",
        ),
        (
            "Cannot read node logs: ",
            "無法讀取節點日誌：",
            "ノードログを読み込めません：",
            "Impossible de lire les journaux du nœud : ",
        ),
        (
            "Cannot export node logs: ",
            "無法匯出節點日誌：",
            "ノードログをエクスポートできません：",
            "Impossible d’exporter les journaux du nœud : ",
        ),
        (
            "Cannot inspect runtime environment: ",
            "無法檢查執行環境：",
            "実行環境を確認できません：",
            "Impossible d’inspecter l’environnement d’exécution : ",
        ),
        (
            "Address alias operation failed: ",
            "地址別名操作失敗：",
            "アドレス別名の操作に失敗しました：",
            "Échec de l’opération sur l’alias d’adresse : ",
        ),
        (
            "Cannot read rewards: ",
            "無法讀取獎勵：",
            "報酬を読み込めません：",
            "Impossible de lire les récompenses : ",
        ),
        (
            "Cannot read legacy assets: ",
            "無法讀取舊版資產：",
            "従来の資産を読み込めません：",
            "Impossible de lire les anciens actifs : ",
        ),
        (
            "Cannot read Hypergraph: ",
            "無法讀取 Hypergraph：",
            "Hypergraph を読み込めません：",
            "Impossible de lire Hypergraph : ",
        ),
        (
            "Cannot read shard information: ",
            "無法讀取分片資訊：",
            "シャード情報を読み込めません：",
            "Impossible de lire les informations du shard : ",
        ),
        (
            "Cannot read archive shards: ",
            "無法讀取 Archive 分片：",
            "Archive シャードを読み込めません：",
            "Impossible de lire les shards d’Archive : ",
        ),
        (
            "Cannot read Confidential wallet: ",
            "無法讀取 Confidential 錢包：",
            "Confidential ウォレットを読み込めません：",
            "Impossible de lire le portefeuille Confidential : ",
        ),
    ];

    if error == "Node status query timed out" {
        return match locale {
            Locale::English => unreachable!(),
            Locale::TraditionalChinese => "節點狀態查詢逾時",
            Locale::Japanese => "ノード状態の取得がタイムアウトしました",
            Locale::French => "La lecture de l’état du nœud a expiré",
        }
        .into();
    }
    for (english, traditional, japanese, french) in PREFIXES {
        if let Some(detail) = error.strip_prefix(english) {
            let prefix = match locale {
                Locale::English => unreachable!(),
                Locale::TraditionalChinese => traditional,
                Locale::Japanese => japanese,
                Locale::French => french,
            };
            return format!("{prefix}{detail}").into();
        }
    }
    error.into()
}

pub fn localize_operation_message(locale: Locale, message: &str) -> String {
    if locale == Locale::English {
        return message.into();
    }

    let service_error = localize_service_error(locale, message);
    if service_error != message {
        return service_error.into_owned();
    }

    for (english, traditional, japanese, french) in [
        ("Response timed out; the result is unknown. The node may still be working. Refresh status and do not submit again", "回應逾時，結果未知。節點可能仍在處理；請重新整理狀態，不要重複提交", "応答がタイムアウトし、結果は不明です。ノードはまだ処理中の可能性があります。状態を更新し、再送信しないでください", "La réponse a expiré et le résultat est inconnu. Le nœud travaille peut-être encore. Actualisez l’état sans renvoyer la demande"),
        ("Shard join request submitted; the node is computing the proof", "分片加入請求已提交；節點正在計算證明", "シャード参加リクエストを送信しました。ノードが証明を計算しています", "Demande de participation au shard envoyée ; le nœud calcule la preuve"),
        ("Log export cancelled; temporary files were removed", "已取消日誌匯出，暫存檔案已移除", "ログのエクスポートをキャンセルし、一時ファイルを削除しました", "Export des journaux annulé ; les fichiers temporaires ont été supprimés"),
    ] {
        if message == english {
            return match locale {
                Locale::English => unreachable!(),
                Locale::TraditionalChinese => traditional,
                Locale::Japanese => japanese,
                Locale::French => french,
            }
            .into();
        }
    }

    for (prefix, suffix, traditional, japanese, french) in [
        (
            "Key ",
            " created",
            ("金鑰 ", " 已建立"),
            ("キー ", " を作成しました"),
            ("Clé ", " créée"),
        ),
        (
            "Key ",
            " deleted",
            ("金鑰 ", " 已刪除"),
            ("キー ", " を削除しました"),
            ("Clé ", " supprimée"),
        ),
        (
            "Alias ",
            " saved",
            ("別名 ", " 已儲存"),
            ("別名 ", " を保存しました"),
            ("Alias ", " enregistré"),
        ),
        (
            "Alias ",
            " deleted",
            ("別名 ", " 已刪除"),
            ("別名 ", " を削除しました"),
            ("Alias ", " supprimé"),
        ),
    ] {
        if let Some(value) = message
            .strip_prefix(prefix)
            .and_then(|value| value.strip_suffix(suffix))
        {
            let (translated_prefix, translated_suffix) = match locale {
                Locale::English => unreachable!(),
                Locale::TraditionalChinese => traditional,
                Locale::Japanese => japanese,
                Locale::French => french,
            };
            return format!("{translated_prefix}{value}{translated_suffix}");
        }
    }

    if let Some(action) =
        message.strip_suffix(" request submitted; waiting for on-chain state update")
    {
        let action = match (locale, action) {
            (Locale::TraditionalChinese, "Join shard") => "加入分片",
            (Locale::TraditionalChinese, "Leave shard") => "離開分片",
            (Locale::TraditionalChinese, "Confirm proposal") => "確認提案",
            (Locale::TraditionalChinese, "Reject proposal") => "拒絕提案",
            (Locale::TraditionalChinese, "Pause shard") => "暫停分片",
            (Locale::TraditionalChinese, "Resume shard") => "恢復分片",
            (Locale::Japanese, "Join shard") => "シャード参加",
            (Locale::Japanese, "Leave shard") => "シャード退出",
            (Locale::Japanese, "Confirm proposal") => "提案の確認",
            (Locale::Japanese, "Reject proposal") => "提案の拒否",
            (Locale::Japanese, "Pause shard") => "シャードの一時停止",
            (Locale::Japanese, "Resume shard") => "シャードの再開",
            (Locale::French, "Join shard") => "Participer au shard",
            (Locale::French, "Leave shard") => "Quitter le shard",
            (Locale::French, "Confirm proposal") => "Confirmer la proposition",
            (Locale::French, "Reject proposal") => "Rejeter la proposition",
            (Locale::French, "Pause shard") => "Suspendre le shard",
            (Locale::French, "Resume shard") => "Reprendre le shard",
            _ => return message.into(),
        };
        return match locale {
            Locale::English => unreachable!(),
            Locale::TraditionalChinese => format!("{action}請求已提交；正在等待鏈上狀態更新"),
            Locale::Japanese => {
                format!("{action}リクエストを送信しました。オンチェーン状態の更新を待っています")
            }
            Locale::French => format!(
                "Demande « {action} » envoyée ; attente de la mise à jour de l’état on-chain"
            ),
        };
    }

    message.into()
}

pub fn en(key: MessageKey) -> &'static str {
    text(Locale::English, key)
}

pub fn zh_tw(key: MessageKey) -> &'static str {
    text(Locale::TraditionalChinese, key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::allocation::{
        ActionHint, ActionTiming, AllocationAction, AllocationLifecycleStatus,
    };

    fn project_text_sources(root: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                if path.ends_with("target")
                    || path.ends_with(".git")
                    || path.ends_with("node_modules")
                    || path.ends_with("web/dist")
                    || path.ends_with("docs")
                    || path.ends_with("src/i18n/catalog")
                {
                    continue;
                }
                project_text_sources(&path, files);
            } else if path.extension().is_some_and(|extension| {
                matches!(extension.to_str(), Some("rs" | "js" | "vue" | "md"))
            }) && !path.ends_with("web/src/locale.js")
            {
                files.push(path);
            }
        }
    }

    #[test]
    fn every_key_has_text() {
        let keys = [
            MessageKey::ProductName,
            MessageKey::MainTitle,
            MessageKey::MenuOverview,
            MessageKey::MenuNodeManagement,
            MessageKey::MenuProverShards,
            MessageKey::MenuWalletToken,
            MessageKey::MenuIdentity,
            MessageKey::MenuDataApps,
            MessageKey::MenuDiagnosticsLogs,
            MessageKey::MenuConfigs,
            MessageKey::MenuRuntime,
            MessageKey::MenuProver,
            MessageKey::MenuShards,
            MessageKey::MenuArchiveShards,
            MessageKey::MenuRewards,
            MessageKey::MenuConfidentialWallet,
            MessageKey::MenuPendingEscrows,
            MessageKey::MenuLegacyAssets,
            MessageKey::MenuHypergraph,
            MessageKey::MenuDiagnostics,
            MessageKey::MenuDiagnosticsIdentity,
            MessageKey::MenuRemoteHealth,
            MessageKey::MenuHealth,
            MessageKey::MenuPeerResolve,
            MessageKey::MenuLogs,
            MessageKey::MenuLogExport,
            MessageKey::MenuKeys,
            MessageKey::MenuAliases,
            MessageKey::MenuOther,
            MessageKey::MenuHelp,
            MessageKey::StatusConnected,
            MessageKey::StatusDisconnected,
            MessageKey::StatusLoading,
            MessageKey::StatusNoData,
            MessageKey::FooterMain,
        ];

        assert!(keys.into_iter().all(|key| {
            !text(Locale::English, key).is_empty()
                && !text(Locale::TraditionalChinese, key).is_empty()
        }));
    }

    #[test]
    fn static_catalog_supports_all_four_locales() {
        assert_eq!(t(Locale::English, UiText::Language), "Language");
        assert_eq!(t(Locale::TraditionalChinese, UiText::Language), "語言");
        assert_eq!(t(Locale::Japanese, UiText::Language), "言語");
        assert_eq!(t(Locale::French, UiText::Language), "Langue");

        for &key in UiText::ALL {
            let values = [
                Locale::English,
                Locale::TraditionalChinese,
                Locale::Japanese,
                Locale::French,
            ]
            .map(|locale| t(locale, key));
            assert!(
                values.iter().any(|value| !value.is_empty()),
                "empty {key:?}"
            );
        }
    }

    #[test]
    fn allocation_lifecycle_labels_are_localized() {
        assert_eq!(
            allocation_lifecycle_status_label(
                Locale::TraditionalChinese,
                AllocationLifecycleStatus::Departing,
            ),
            "退出"
        );
        assert_eq!(
            allocation_lifecycle_status_label(
                Locale::English,
                AllocationLifecycleStatus::ExpiredEpoch,
            ),
            "Renewal required"
        );
        assert_eq!(
            allocation_lifecycle_status_label(Locale::English, AllocationLifecycleStatus::Rejected,),
            "Rejected"
        );
    }

    #[test]
    fn scheduled_allocation_action_names_the_frame_and_remaining_frames() {
        let hint = ActionHint {
            actions: vec![AllocationAction::Reject, AllocationAction::Confirm],
            timing: ActionTiming::At {
                frame: 1_007_280,
                frames_remaining: 58,
            },
        };

        let text = format_allocation_action_hint(Locale::TraditionalChinese, &hint, true);

        assert!(text.contains("拒絕/確認"), "{text}");
        assert!(text.contains("Frame 1007280"), "{text}");
        assert!(text.contains("剩餘 Frame 58"), "{text}");
    }

    #[test]
    fn dynamic_errors_preserve_details_in_each_language() {
        let error = "Cannot connect to node: connection refused";
        assert_eq!(
            localize_service_error(Locale::Japanese, error),
            "ノードに接続できません：connection refused"
        );
        assert_eq!(
            localize_service_error(Locale::French, error),
            "Impossible de se connecter au nœud : connection refused"
        );
    }

    #[test]
    fn legacy_migration_error_categories_do_not_steal_node_configuration_errors() {
        let error = "Cannot read node configuration: permission denied";
        for (locale, node_category) in [
            (Locale::TraditionalChinese, "無法讀取節點設定"),
            (Locale::Japanese, "ノード設定を読み込めません"),
            (
                Locale::French,
                "Impossible de lire la configuration du nœud",
            ),
        ] {
            let localized = localize_legacy_migration_error(locale, error);
            assert!(localized.contains(node_category), "{locale:?}: {localized}");
            assert!(!localized.contains(t(locale, UiText::LegacyMigrationCannotReadFile)));
        }
    }

    #[test]
    fn stable_legacy_migration_errors_are_fully_localized() {
        let details = [
            "old peer key must contain exactly 114 bytes",
            "old peer key is not hex",
            "old peer seed and public key do not match",
            "This old identity has no unshielded QUIL",
            "No legacy QUIL is currently covered by a live shard",
            "Legacy QUIL amount overflow",
            "Migration fee overflow",
            "Legacy QUIL does not cover migration fees",
            "Migration approval expired",
            "Migration approval contents changed",
            "Old key does not belong to this migration",
            "Invalid migration source",
            "Invalid legacy migration fingerprint",
            "Migration has no recoverable saved submission",
            "A pending migration source is no longer available",
        ];
        for locale in [Locale::TraditionalChinese, Locale::Japanese, Locale::French] {
            for detail in details {
                let error = format!("Invalid input: {detail}");
                let localized = localize_legacy_migration_error(locale, &error);
                assert!(!localized.contains(detail), "{locale:?}: {localized}");
            }
            let detail = "Configuration path is not a directory";
            let error = format!("Cannot read node configuration: {detail}");
            let localized = localize_legacy_migration_error(locale, &error);
            assert!(!localized.contains(detail), "{locale:?}: {localized}");
        }
    }

    #[test]
    fn operation_results_follow_the_current_language() {
        let message = "Shard join request submitted; the node is computing the proof";
        assert_eq!(
            localize_operation_message(Locale::Japanese, message),
            "シャード参加リクエストを送信しました。ノードが証明を計算しています"
        );
        assert_eq!(
            localize_operation_message(Locale::French, message),
            "Demande de participation au shard envoyée ; le nœud calcule la preuve"
        );
        assert_eq!(
            localize_operation_message(Locale::French, "Key demo created"),
            "Clé demo créée"
        );
    }

    #[test]
    fn footer_describes_q_as_back_and_control_c_as_exit() {
        let footer = zh_tw(MessageKey::FooterMain);

        assert!(footer.contains("Q 返回"));
        assert!(footer.contains("Ctrl+C 離開"));
        assert!(!footer.contains("Q 離開"));
    }

    #[test]
    fn project_sources_have_no_simplified_chinese_only_phrases() {
        let mut files = Vec::new();
        project_text_sources(std::path::Path::new(env!("CARGO_MANIFEST_DIR")), &mut files);
        let forbidden = [
            "\u{8282}\u{70b9}",
            "\u{8bfb}\u{53d6}",
            "\u{5931}\u{8d25}",
            "\u{8bbe}\u{7f6e}",
            "\u{8fde}\u{63a5}",
            "\u{5bc6}\u{94a5}",
            "\u{65e5}\u{5fd7}",
            "\u{94b1}\u{5305}",
            "\u{5956}\u{52b1}",
            "\u{5220}\u{9664}",
            "\u{521b}\u{5efa}",
            "\u{786e}\u{8ba4}",
            "\u{72b6}\u{6001}",
            "\u{6570}\u{636e}",
            "\u{6587}\u{4ef6}",
            "\u{65e0}\u{6cd5}",
            "\u{5f53}\u{524d}",
            "\u{6682}\u{65e0}",
            "\u{8d44}\u{4ea7}",
            "\u{4f59}\u{989d}",
            "\u{81ea}\u{52a8}",
            "\u{5b9a}\u{4e49}",
            "\u{9636}\u{6bb5}",
            "\u{516c}\u{94a5}",
            "\u{6682}\u{505c}",
            "\u{516c}\u{7f51}",
            "\u{79c1}\u{7f51}",
            "\u{672a}\u{89c2}\u{6d4b}",
            "\u{73af}\u{5883}",
            "\u{5bfc}\u{822a}",
            "\u{6d3b}\u{8dc3}",
            "\u{952e}",
        ];
        for path in files {
            let source = std::fs::read_to_string(&path).unwrap();
            for phrase in forbidden {
                assert!(
                    !source.contains(phrase),
                    "{} contains Simplified Chinese phrase {phrase:?}",
                    path.display()
                );
            }
        }
    }
}
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Locale {
    #[default]
    #[serde(rename = "en")]
    English,
    #[serde(rename = "zh-TW")]
    TraditionalChinese,
    #[serde(rename = "ja")]
    Japanese,
    #[serde(rename = "fr")]
    French,
}

impl Locale {
    pub const fn next(self) -> Self {
        match self {
            Self::English => Self::TraditionalChinese,
            Self::TraditionalChinese => Self::Japanese,
            Self::Japanese => Self::French,
            Self::French => Self::English,
        }
    }

    pub const fn previous(self) -> Self {
        match self {
            Self::English => Self::French,
            Self::TraditionalChinese => Self::English,
            Self::Japanese => Self::TraditionalChinese,
            Self::French => Self::Japanese,
        }
    }

    pub const fn display_name(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::TraditionalChinese => "繁體中文",
            Self::Japanese => "日本語",
            Self::French => "Français",
        }
    }
}
mod catalog;
pub mod web;

pub use catalog::UiText;
