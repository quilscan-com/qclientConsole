use crate::i18n::MessageKey;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    Home,
    NodeStatus,
    Section(Section),
    Configs,
    ConfigPath,
    Runtime,
    Diagnostics,
    DiagnosticsIdentity,
    RemoteHealth,
    Health,
    PeerResolve,
    Logs,
    LogExport,
    Keys,
    KeyActions,
    KeyCreate,
    Aliases,
    AliasActions,
    AliasEdit,
    Prover,
    ProverActions,
    Shards,
    ArchiveShards,
    Rewards,
    RewardMintConfirm,
    ConfidentialWallet,
    PendingEscrows,
    WalletActions,
    WalletForm,
    WalletConfirm,
    WalletProgress,
    WalletResult,
    TokenTransfer,
    Capabilities,
    CapabilityForm,
    CapabilityConfirm,
    CapabilityTask,
    LegacyAssets,
    Hypergraph,
    DeployToken,
    Confirm,
    OperationResult,
    Other,
    Help,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Node,
    Prover,
    Wallet,
    Identity,
    Data,
    Diagnostics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectionItem {
    pub label: MessageKey,
    pub page: Page,
}

impl SectionItem {
    pub const fn capability_id(self) -> Option<&'static str> {
        match self.page {
            Page::TokenTransfer => Some("token.transfer"),
            Page::DeployToken => Some("deploy.token"),
            _ => None,
        }
    }
}

const NODE_ITEMS: [SectionItem; 4] = [
    SectionItem {
        label: MessageKey::MenuOverview,
        page: Page::NodeStatus,
    },
    SectionItem {
        label: MessageKey::MenuConfigs,
        page: Page::Configs,
    },
    SectionItem {
        label: MessageKey::MenuRuntime,
        page: Page::Runtime,
    },
    SectionItem {
        label: MessageKey::MenuArchiveShards,
        page: Page::ArchiveShards,
    },
];

const PROVER_ITEMS: [SectionItem; 2] = [
    SectionItem {
        label: MessageKey::MenuProver,
        page: Page::Prover,
    },
    SectionItem {
        label: MessageKey::MenuShards,
        page: Page::Shards,
    },
];

const WALLET_ITEMS: [SectionItem; 5] = [
    SectionItem {
        label: MessageKey::MenuRewards,
        page: Page::Rewards,
    },
    SectionItem {
        label: MessageKey::MenuTransfer,
        page: Page::TokenTransfer,
    },
    SectionItem {
        label: MessageKey::MenuLegacyAssets,
        page: Page::LegacyAssets,
    },
    SectionItem {
        label: MessageKey::MenuConfidentialWallet,
        page: Page::ConfidentialWallet,
    },
    SectionItem {
        label: MessageKey::MenuPendingEscrows,
        page: Page::PendingEscrows,
    },
];

const IDENTITY_ITEMS: [SectionItem; 2] = [
    SectionItem {
        label: MessageKey::MenuKeys,
        page: Page::Keys,
    },
    SectionItem {
        label: MessageKey::MenuAliases,
        page: Page::Aliases,
    },
];

const DATA_ITEMS: [SectionItem; 2] = [
    SectionItem {
        label: MessageKey::MenuHypergraph,
        page: Page::Hypergraph,
    },
    SectionItem {
        label: MessageKey::MenuDeployToken,
        page: Page::DeployToken,
    },
];

const DIAGNOSTICS_ITEMS: [SectionItem; 7] = [
    SectionItem {
        label: MessageKey::MenuHealth,
        page: Page::Health,
    },
    SectionItem {
        label: MessageKey::MenuDiagnostics,
        page: Page::Diagnostics,
    },
    SectionItem {
        label: MessageKey::MenuDiagnosticsIdentity,
        page: Page::DiagnosticsIdentity,
    },
    SectionItem {
        label: MessageKey::MenuPeerResolve,
        page: Page::PeerResolve,
    },
    SectionItem {
        label: MessageKey::MenuLogs,
        page: Page::Logs,
    },
    SectionItem {
        label: MessageKey::MenuLogExport,
        page: Page::LogExport,
    },
    SectionItem {
        label: MessageKey::MenuRemoteHealth,
        page: Page::RemoteHealth,
    },
];

impl Section {
    pub fn title_key(self) -> MessageKey {
        match self {
            Self::Node => MessageKey::MenuNodeManagement,
            Self::Prover => MessageKey::MenuProverShards,
            Self::Wallet => MessageKey::MenuWalletToken,
            Self::Identity => MessageKey::MenuIdentity,
            Self::Data => MessageKey::MenuDataApps,
            Self::Diagnostics => MessageKey::MenuDiagnosticsLogs,
        }
    }

    pub fn items(self) -> &'static [SectionItem] {
        match self {
            Self::Node => &NODE_ITEMS,
            Self::Prover => &PROVER_ITEMS,
            Self::Wallet => &WALLET_ITEMS,
            Self::Identity => &IDENTITY_ITEMS,
            Self::Data => &DATA_ITEMS,
            Self::Diagnostics => &DIAGNOSTICS_ITEMS,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootRoute {
    Overview,
    Section(Section),
    Capabilities,
    Other,
    Help,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RootDestination {
    pub label: MessageKey,
    pub route: RootRoute,
}

pub const ROOT_MENU: [RootDestination; 10] = [
    RootDestination {
        label: MessageKey::MenuOverview,
        route: RootRoute::Overview,
    },
    RootDestination {
        label: MessageKey::MenuNodeManagement,
        route: RootRoute::Section(Section::Node),
    },
    RootDestination {
        label: MessageKey::MenuProverShards,
        route: RootRoute::Section(Section::Prover),
    },
    RootDestination {
        label: MessageKey::MenuWalletToken,
        route: RootRoute::Section(Section::Wallet),
    },
    RootDestination {
        label: MessageKey::MenuIdentity,
        route: RootRoute::Section(Section::Identity),
    },
    RootDestination {
        label: MessageKey::MenuDataApps,
        route: RootRoute::Section(Section::Data),
    },
    RootDestination {
        label: MessageKey::MenuDiagnosticsLogs,
        route: RootRoute::Section(Section::Diagnostics),
    },
    RootDestination {
        label: MessageKey::MenuCapabilities,
        route: RootRoute::Capabilities,
    },
    RootDestination {
        label: MessageKey::MenuOther,
        route: RootRoute::Other,
    },
    RootDestination {
        label: MessageKey::MenuHelp,
        route: RootRoute::Help,
    },
];
