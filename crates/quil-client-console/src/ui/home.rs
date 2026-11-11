use crate::i18n::UiText;
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::{
    app::{App, RootDestination, RootRoute, Section, ROOT_MENU},
    i18n::{t, text, MessageKey},
};

use super::{
    chrome::{self, Breakpoint},
    text::truncate_cells,
    theme,
};

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let locale = app.locale();
    let footer = match frame.area().width {
        0..=49 => t(locale, UiText::HomeCompactFooter),
        50..=69 => t(locale, UiText::EnterEscQBackRRefreshHelpCtrlCExit),
        _ => text(locale, MessageKey::FooterMain),
    };
    let Some(areas) = chrome::screen(frame, text(locale, MessageKey::ProductName), app, footer)
    else {
        return;
    };
    match areas.breakpoint {
        Breakpoint::Wide | Breakpoint::Standard => {
            let [menu, detail] =
                Layout::horizontal([Constraint::Length(32), Constraint::Min(0)]).areas(areas.body);
            draw_menu(frame, app, menu);
            draw_description(frame, app, detail);
        }
        Breakpoint::Narrow => {
            let [menu, description] =
                Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(areas.body);
            draw_menu(frame, app, menu);
            frame.render_widget(
                Paragraph::new(truncate_cells(
                    root_description(locale, selected_destination(app).route).purpose,
                    usize::from(description.width),
                ))
                .style(theme::muted()),
                description,
            );
        }
    }
}

fn draw_menu(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let items = ROOT_MENU.iter().enumerate().map(|(index, destination)| {
        ListItem::new(truncate_cells(
            &format!("{}. {}", index + 1, text(app.locale(), destination.label)),
            usize::from(area.width.saturating_sub(4)),
        ))
    });
    let mut state = ListState::default().with_selected(Some(app.menu_index));
    frame.render_stateful_widget(
        List::new(items)
            .block(chrome::panel(t(app.locale(), UiText::MainNavigation)))
            .highlight_style(theme::focus())
            .highlight_symbol(theme::FOCUS_MARKER),
        area,
        &mut state,
    );
}

struct RootDescription {
    purpose: &'static str,
    includes: &'static [&'static str],
}

fn selected_destination(app: &App) -> &'static RootDestination {
    &ROOT_MENU[app.menu_index.min(ROOT_MENU.len() - 1)]
}

fn root_description(locale: crate::i18n::Locale, route: RootRoute) -> RootDescription {
    match route {
        RootRoute::Overview => RootDescription {
            purpose: t(locale, UiText::ViewNodeStatusAndConsoleEntryPoints),
            includes: match locale {
                crate::i18n::Locale::English => &[
                    "Current configuration and node connection",
                    "Feature navigation",
                ],
                crate::i18n::Locale::TraditionalChinese => &["目前設定與節點連線狀態", "功能導覽"],
                crate::i18n::Locale::Japanese => &["現在の設定とノード接続", "機能ナビゲーション"],
                crate::i18n::Locale::French => &[
                    "Configuration actuelle et connexion au nœud",
                    "Navigation des fonctions",
                ],
            },
        },
        RootRoute::Section(Section::Node) => RootDescription {
            purpose: t(locale, UiText::ManageNodeConfigurationAndLocalRuntime),
            includes: match locale {
                crate::i18n::Locale::English => &[
                    "Overview and node configuration",
                    "Runtime and Archive shards",
                ],
                crate::i18n::Locale::TraditionalChinese => {
                    &["系統概覽與節點設定", "執行環境與 Archive 分片"]
                }
                crate::i18n::Locale::Japanese => {
                    &["概要とノード設定", "実行環境と Archive シャード"]
                }
                crate::i18n::Locale::French => &[
                    "Vue d’ensemble et configuration du nœud",
                    "Exécution et shards d’Archive",
                ],
            },
        },
        RootRoute::Section(Section::Prover) => RootDescription {
            purpose: t(locale, UiText::ManageProverLifecycleAndShards),
            includes: match locale {
                crate::i18n::Locale::English => &["Prover management and shard information"],
                crate::i18n::Locale::TraditionalChinese => &["Prover 管理與分片資訊"],
                crate::i18n::Locale::Japanese => &["Prover 管理とシャード情報"],
                crate::i18n::Locale::French => {
                    &["Gestion du Prover et informations sur les shards"]
                }
            },
        },
        RootRoute::Section(Section::Wallet) => RootDescription {
            purpose: t(locale, UiText::InspectRewardsAssetsAndPendingTransfers),
            includes: match locale {
                crate::i18n::Locale::English => &[
                    "Rewards and legacy assets",
                    "Confidential wallet and pending transfers",
                ],
                crate::i18n::Locale::TraditionalChinese => {
                    &["獎勵與舊版資產", "隱私錢包與待處理轉帳"]
                }
                crate::i18n::Locale::Japanese => {
                    &["報酬と従来の資産", "Confidential ウォレットと保留中の送金"]
                }
                crate::i18n::Locale::French => &[
                    "Récompenses et anciens actifs",
                    "Portefeuille Confidential et transferts en attente",
                ],
            },
        },
        RootRoute::Section(Section::Identity) => RootDescription {
            purpose: t(locale, UiText::ManageLocalKeysAndAddressAliases),
            includes: match locale {
                crate::i18n::Locale::English => &["Keys and aliases"],
                crate::i18n::Locale::TraditionalChinese => &["金鑰與別名"],
                crate::i18n::Locale::Japanese => &["キーと別名"],
                crate::i18n::Locale::French => &["Clés et alias"],
            },
        },
        RootRoute::Section(Section::Data) => RootDescription {
            purpose: t(locale, UiText::QueryHypergraphObjects),
            includes: &["Vertex、Hyperedge"],
        },
        RootRoute::Section(Section::Diagnostics) => RootDescription {
            purpose: t(
                locale,
                UiText::TroubleshootConnectionsAndInspectOrExportLogs,
            ),
            includes: match locale {
                crate::i18n::Locale::English => &[
                    "Health reports and diagnostics",
                    "Diagnostic identity and Peer resolution",
                    "Log viewer, redacted export, and remote health",
                ],
                crate::i18n::Locale::TraditionalChinese => &[
                    "健康報告與節點診斷",
                    "診斷身分與 Peer 解析",
                    "日誌查看、脫敏匯出與遠端健康",
                ],
                crate::i18n::Locale::Japanese => &[
                    "健全性レポートと診断",
                    "診断 ID と Peer 解決",
                    "ログ表示、マスキング済みエクスポート、リモートヘルス",
                ],
                crate::i18n::Locale::French => &[
                    "Rapports de santé et diagnostics",
                    "Identité de diagnostic et résolution de Peer",
                    "Consultation des journaux, export masqué et santé distante",
                ],
            },
        },
        RootRoute::Capabilities => RootDescription {
            purpose: t(locale, UiText::QclientCommands),
            includes: match locale {
                crate::i18n::Locale::English => &[
                    "Every qclient command",
                    "Forms, confirmation, and task output",
                ],
                crate::i18n::Locale::TraditionalChinese => {
                    &["全部 qclient 指令", "表單、確認與任務輸出"]
                }
                crate::i18n::Locale::Japanese => {
                    &["すべての qclient コマンド", "フォーム、確認、タスク出力"]
                }
                crate::i18n::Locale::French => &[
                    "Toutes les commandes qclient",
                    "Formulaires, confirmation et sortie des tâches",
                ],
            },
        },
        RootRoute::Other => RootDescription {
            purpose: t(locale, UiText::ManageConsoleLanguageAndReservedWebPort),
            includes: match locale {
                crate::i18n::Locale::English => {
                    &["Web management and port", "Language and local settings"]
                }
                crate::i18n::Locale::TraditionalChinese => &["Web 管理與連接埠", "語言與本機設定"],
                crate::i18n::Locale::Japanese => &["Web 管理とポート", "言語とローカル設定"],
                crate::i18n::Locale::French => {
                    &["Gestion Web et port", "Langue et paramètres locaux"]
                }
            },
        },
        RootRoute::Help => RootDescription {
            purpose: t(locale, UiText::ReadKeyboardShortcutsAndUsageGuidance),
            includes: match locale {
                crate::i18n::Locale::English => &["Keyboard shortcuts and instructions"],
                crate::i18n::Locale::TraditionalChinese => &["快速鍵與操作說明"],
                crate::i18n::Locale::Japanese => &["キーボードショートカットと操作説明"],
                crate::i18n::Locale::French => &["Raccourcis clavier et instructions"],
            },
        },
    }
}

fn draw_description(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let destination = selected_destination(app);
    let locale = app.locale();
    let description = root_description(locale, destination.route);
    let width = usize::from(area.width.saturating_sub(2));
    let line = |value: String, style: Style| {
        Line::from(Span::styled(truncate_cells(&value, width), style))
    };
    let (availability, style) = availability(app, destination.route);
    let mut lines = vec![
        line(text(locale, destination.label).into(), theme::label()),
        line(
            format!("{}: {}", t(locale, UiText::Purpose), description.purpose),
            theme::normal(),
        ),
        line(
            format!("{}: {availability}", t(locale, UiText::Status)),
            style,
        ),
        line(format!("{}:", t(locale, UiText::Includes)), theme::muted()),
    ];
    lines.extend(
        description
            .includes
            .iter()
            .map(|item| line(format!("  {item}"), theme::normal())),
    );
    if matches!(destination.route, RootRoute::Overview) {
        if let Some(error) = &app.error {
            let error = crate::i18n::localize_service_error(locale, error);
            lines.push(line(
                format!("{}: {error}", t(locale, UiText::RefreshFailed)),
                theme::failure(),
            ));
        }
    }
    frame.render_widget(
        Paragraph::new(lines).block(chrome::panel(t(locale, UiText::Navigation))),
        area,
    );
}

fn availability(app: &App, route: RootRoute) -> (&'static str, Style) {
    let locale = app.locale();
    match route {
        RootRoute::Overview => match (&app.snapshot, &app.error, app.loading) {
            (Some(_), _, true) => (t(locale, UiText::RefreshingStale), theme::attention()),
            (None, _, true) => (t(locale, UiText::LoadingNodeStatus), theme::attention()),
            (Some(_), Some(_), false) => (t(locale, UiText::StaleRToRefresh), theme::attention()),
            (None, Some(_), false) => (t(locale, UiText::LoadFailedRToRefresh), theme::failure()),
            (Some(node), None, false) if node.connected => {
                (t(locale, UiText::NodeConnected), theme::healthy())
            }
            (Some(_), None, false) => (t(locale, UiText::NodeDisconnected), theme::attention()),
            (None, None, false) if app.active_config().is_none() => (
                t(locale, UiText::NoNodeConfigurationSelected),
                theme::attention(),
            ),
            (None, None, false) => (
                t(locale, UiText::NodeStatusNotLoadedRToRefresh),
                theme::muted(),
            ),
        },
        RootRoute::Section(Section::Node | Section::Diagnostics)
            if app.active_config().is_none() =>
        {
            (
                t(locale, UiText::AvailableSomeFeaturesNeedNodeConfiguration),
                theme::attention(),
            )
        }
        RootRoute::Section(_) if app.active_config().is_none() => (
            t(locale, UiText::AvailableRequiresNodeConfiguration),
            theme::attention(),
        ),
        RootRoute::Section(_) => (
            t(locale, UiText::AvailableUsesCurrentNodeConfiguration),
            theme::muted(),
        ),
        RootRoute::Capabilities if app.active_config().is_none() => (
            t(locale, UiText::AvailableRequiresNodeConfiguration),
            theme::attention(),
        ),
        RootRoute::Capabilities => (
            t(locale, UiText::AvailableUsesCurrentNodeConfiguration),
            theme::muted(),
        ),
        RootRoute::Other => (
            t(locale, UiText::AvailableSavesLocalSettings),
            theme::muted(),
        ),
        RootRoute::Help => (t(locale, UiText::AvailableViewInstructions), theme::muted()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::{reduce_input, Page, RootRoute, Section},
        event::InputEvent,
    };

    #[test]
    fn visible_menu_routes_match_app_pages() {
        let expected = [
            (MessageKey::MenuOverview, RootRoute::Overview, Page::Home),
            (
                MessageKey::MenuNodeManagement,
                RootRoute::Section(Section::Node),
                Page::Section(Section::Node),
            ),
            (
                MessageKey::MenuProverShards,
                RootRoute::Section(Section::Prover),
                Page::Section(Section::Prover),
            ),
            (
                MessageKey::MenuWalletToken,
                RootRoute::Section(Section::Wallet),
                Page::Section(Section::Wallet),
            ),
            (
                MessageKey::MenuIdentity,
                RootRoute::Section(Section::Identity),
                Page::Section(Section::Identity),
            ),
            (
                MessageKey::MenuDataApps,
                RootRoute::Section(Section::Data),
                Page::Section(Section::Data),
            ),
            (
                MessageKey::MenuDiagnosticsLogs,
                RootRoute::Section(Section::Diagnostics),
                Page::Section(Section::Diagnostics),
            ),
            (
                MessageKey::MenuCapabilities,
                RootRoute::Capabilities,
                Page::Capabilities,
            ),
            (MessageKey::MenuOther, RootRoute::Other, Page::Other),
            (MessageKey::MenuHelp, RootRoute::Help, Page::Help),
        ];

        assert_eq!(ROOT_MENU.len(), expected.len());
        for (index, (label, route, page)) in expected.into_iter().enumerate() {
            assert_eq!(ROOT_MENU[index].route, route, "route index {index}");
            assert_eq!(ROOT_MENU[index].label, label, "visible menu index {index}");
            assert!(!text(crate::i18n::Locale::English, label).is_empty());

            let mut app = App {
                menu_index: index,
                ..Default::default()
            };
            reduce_input(&mut app, InputEvent::Enter);
            assert_eq!(app.page(), page, "Enter route at index {index}");
        }
    }
}
