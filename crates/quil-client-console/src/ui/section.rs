use crate::i18n::UiText;
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::{
    app::{App, Page, Section, SectionItem},
    i18n::{t, text, Locale},
};

use super::{
    chrome::{self, Breakpoint},
    text::truncate_cells,
    theme,
};

pub fn draw(frame: &mut Frame<'_>, app: &App, section: Section) {
    let locale = app.locale();
    let footer = if frame.area().width < 60 {
        t(locale, UiText::EnterOpenEscQBackCtrlCExit)
    } else {
        t(locale, UiText::SelectEnterOpenEscQBackCtrlCExit)
    };
    let Some(areas) = chrome::screen(frame, text(locale, section.title_key()), app, footer) else {
        return;
    };
    let selected = app.section_index.min(section.items().len() - 1);
    let item = &section.items()[selected];
    match areas.breakpoint {
        Breakpoint::Wide | Breakpoint::Standard => {
            let [list, detail] =
                Layout::horizontal([Constraint::Percentage(30), Constraint::Percentage(70)])
                    .areas(areas.body);
            draw_list(frame, app, section, selected, list);
            draw_detail(frame, app, item, detail);
        }
        Breakpoint::Narrow => {
            let [list, description] =
                Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(areas.body);
            draw_list(frame, app, section, selected, list);
            frame.render_widget(
                Paragraph::new(truncate_cells(
                    purpose_and_category(locale, item.page).0,
                    usize::from(description.width),
                ))
                .style(theme::muted()),
                description,
            );
        }
    }
}

fn draw_list(frame: &mut Frame<'_>, app: &App, section: Section, selected: usize, area: Rect) {
    let items = section.items().iter().enumerate().map(|(index, item)| {
        ListItem::new(truncate_cells(
            &format!("{}. {}", index + 1, text(app.locale(), item.label)),
            usize::from(area.width.saturating_sub(4)),
        ))
    });
    let mut state = ListState::default().with_selected(Some(selected));
    frame.render_stateful_widget(
        List::new(items)
            .block(chrome::panel(t(app.locale(), UiText::Features)))
            .highlight_style(theme::focus())
            .highlight_symbol(theme::FOCUS_MARKER),
        area,
        &mut state,
    );
}

fn draw_detail(frame: &mut Frame<'_>, app: &App, item: &SectionItem, area: Rect) {
    let width = usize::from(area.width.saturating_sub(2));
    let locale = app.locale();
    let (purpose, category) = purpose_and_category(locale, item.page);
    let (availability, style) = availability(app, item.page);
    let line = |value: String, style: Style| {
        Line::from(Span::styled(truncate_cells(&value, width), style))
    };
    frame.render_widget(
        Paragraph::new(vec![
            line(text(locale, item.label).into(), theme::label()),
            line(
                format!("{}: {purpose}", t(locale, UiText::Purpose)),
                theme::normal(),
            ),
            line(
                format!("{}: {category}", t(locale, UiText::Type)),
                theme::muted(),
            ),
            line(
                format!("{}: {availability}", t(locale, UiText::Status)),
                style,
            ),
        ])
        .block(chrome::panel(t(locale, UiText::FeatureDetails))),
        area,
    );
}

pub(super) fn purpose_and_category(locale: Locale, page: Page) -> (&'static str, &'static str) {
    let read_only = t(locale, UiText::ReadOnly);
    match page {
        Page::NodeStatus => (
            t(locale, UiText::ViewNodeConnectionSyncAndWorkerStatus),
            read_only,
        ),
        Page::Configs => (
            t(locale, UiText::SelectANodeConfigurationOrEnterItsDirectory),
            t(locale, UiText::ReadOnlySwitchConfiguration),
        ),
        Page::Runtime => (
            t(locale, UiText::CheckSystemServicesAndAutomaticUpdates),
            t(locale, UiText::ReadOnlyLocalSystem),
        ),
        Page::ArchiveShards => (
            t(
                locale,
                UiText::CompareShardDirectoryConsistencyAcrossArchiveEndpoints,
            ),
            read_only,
        ),
        Page::Prover => (
            t(
                locale,
                UiText::ViewAllocationsAndWorkersAndManageTheirLifecycle,
            ),
            t(locale, UiText::ReadWriteNodeOperation),
        ),
        Page::Shards => (
            t(locale, UiText::ViewShardDirectoriesAndRewardEstimates),
            read_only,
        ),
        Page::Rewards => (
            t(locale, UiText::ReadAndVerifyClaimableProverRewards),
            read_only,
        ),
        Page::LegacyAssets => (t(locale, UiText::ViewLegacyAccountsAndCoins), read_only),
        Page::ConfidentialWallet => (
            t(locale, UiText::ScanLocalWalletCoinsAndVerifyUnspentState),
            read_only,
        ),
        Page::PendingEscrows => (t(locale, UiText::ViewPendingTransfersAndExpiry), read_only),
        Page::TokenTransfer => (
            t(locale, UiText::TransferConfidentialTokensToARecipient),
            t(locale, UiText::ReadWriteNodeOperation),
        ),
        Page::Keys => (
            t(locale, UiText::ViewPublicKeysAndCreateOrDeleteLocalKeys),
            t(locale, UiText::LocalReadWrite),
        ),
        Page::Aliases => (
            t(locale, UiText::ViewAddOrDeleteLocalAddressAliases),
            t(locale, UiText::LocalReadWrite),
        ),
        Page::Hypergraph => (
            t(locale, UiText::QueryAVertexOrHyperedgeByAddress),
            read_only,
        ),
        Page::DeployToken => (
            t(locale, UiText::DeployANewTokenApplication),
            t(locale, UiText::ReadWriteNodeOperation),
        ),
        Page::Health => (t(locale, UiText::ReadTheNodeHealthReport), read_only),
        Page::Diagnostics => (
            t(locale, UiText::ProbeNodeListenAddressesIdentityAndLatency),
            t(locale, UiText::ReadOnlyNetworkProbe),
        ),
        Page::DiagnosticsIdentity => (
            t(locale, UiText::ReadOrCreateAnIndependentDiagnosticIdentity),
            t(locale, UiText::LocalReadWrite),
        ),
        Page::PeerResolve => (
            t(locale, UiText::ResolvePeerAddressesFromTheNodeDirectory),
            read_only,
        ),
        Page::Logs => (t(locale, UiText::ReadMasterOrWorkerLogs), read_only),
        Page::LogExport => (
            t(
                locale,
                UiText::PreviewRedactedLogsAndExportThemLocallyAfterConfirmation,
            ),
            t(locale, UiText::ReadLocalWrite),
        ),
        Page::RemoteHealth => (
            t(
                locale,
                UiText::ReadARemoteHealthReportUsingAnIndependentIdentity,
            ),
            read_only,
        ),
        _ => (t(locale, UiText::OpenTheSelectedFeature), "—"),
    }
}

fn availability(app: &App, page: Page) -> (&'static str, Style) {
    let locale = app.locale();
    match page {
        Page::Configs => (
            t(locale, UiText::AvailableSelectOrAddAConfiguration),
            theme::muted(),
        ),
        Page::Runtime => (t(locale, UiText::AvailableLocalSystemCheck), theme::muted()),
        Page::Diagnostics => (
            t(locale, UiText::AvailableEnterAnAddressToProbe),
            theme::muted(),
        ),
        Page::DiagnosticsIdentity => (
            t(locale, UiText::AvailableReadOrCreateAnIdentity),
            theme::muted(),
        ),
        Page::RemoteHealth if app.diagnostics_identity.snapshot.is_none() => (
            t(locale, UiText::AvailableDiagnosticIdentityRequired),
            theme::attention(),
        ),
        Page::RemoteHealth => (
            t(locale, UiText::AvailableDiagnosticIdentityEnabled),
            theme::healthy(),
        ),
        Page::NodeStatus | Page::Prover => match (&app.snapshot, &app.error, app.loading) {
            (_, _, true) => (
                t(locale, UiText::AvailableLoadingNodeStatus),
                theme::attention(),
            ),
            (Some(_), Some(_), false) => (
                t(locale, UiText::AvailableStaleRefreshRequired),
                theme::attention(),
            ),
            (None, Some(_), false) => (
                t(locale, UiText::AvailableLoadFailedRefreshRequired),
                theme::failure(),
            ),
            (Some(node), None, false) if node.connected => {
                (t(locale, UiText::AvailableNodeConnected), theme::healthy())
            }
            (Some(_), None, false) => (
                t(locale, UiText::AvailableNodeDisconnected),
                theme::attention(),
            ),
            (None, None, false) => (
                t(locale, UiText::AvailableNodeStatusNotLoaded),
                theme::muted(),
            ),
        },
        _ if app.active_config().is_none() => (
            t(locale, UiText::AvailableNoNodeConfigurationSelected),
            theme::attention(),
        ),
        _ => (
            t(locale, UiText::AvailableUsesCurrentNodeConfiguration),
            theme::muted(),
        ),
    }
}
