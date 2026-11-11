const page = (slug, label, description, layout = 'table', actions = [], endpoint = `/api/v1/pages/${slug}`, capabilityId = null) => ({
  slug,
  label,
  labelKey: `page.${slug}`,
  descriptionKey: `description.${slug}`,
  description,
  layout,
  endpoint,
  actions,
  capabilityId,
})

const capabilityPage = (slug, label, description, capabilityId) => page(slug, label, description, 'capabilities', [], '/api/v1/capabilities', capabilityId)

export const NAV_GROUPS = [
  {
    id: 'overview', label: 'Overview', labelKey: 'group.overview', icon: '⌂',
    pages: [page('overview', 'System overview', 'Active configuration, node connection, frame height, and resource status.', 'overview')],
  },
  {
    id: 'tasks', label: 'QClient tasks', labelKey: 'group.tasks', icon: '▶',
    pages: [page('qclient-tasks', 'QClient task center', 'Find and run qclient operations supported by the active node configuration.', 'capabilities', [], '/api/v1/capabilities')],
  },
  {
    id: 'node', label: 'Node', labelKey: 'group.node', icon: '◫',
    pages: [
      page('node-status', 'Node status', 'Inspect node connectivity, synchronization height, peers, and workers.', 'details'),
      page('configs', 'Node configurations', 'Inspect and switch the active node configuration or configuration directory.'),
      page('runtime', 'Runtime', 'Inspect the local service, version, and automatic update status.', 'details'),
      page('archive-shards', 'Archive Shards', 'Compare archive endpoints and shard directory consistency.'),
    ],
  },
  {
    id: 'prover', label: 'Prover & shards', labelKey: 'group.prover', icon: '⌘',
    pages: [
      page('prover', 'Prover management', 'Inspect allocations and workers, and run lifecycle operations.'),
      page('shards', 'Shards', 'Inspect the shard directory, status, and reward estimates.'),
    ],
  },
  {
    id: 'wallet', label: 'Wallet & Token', labelKey: 'group.wallet', icon: '◈',
    pages: [
      page('rewards', 'Rewards / Mint', 'Inspect claimable rewards and mint them to the active confidential wallet.', 'details', [
        { id: 'mint', label: 'Mint rewards', endpoint: '/api/v1/rewards/mint' },
      ]),
      capabilityPage('transfer', 'Transfer', 'Send confidential tokens from the active wallet.', 'token.transfer'),
      page('legacy-assets', 'Legacy Assets', 'Inspect legacy accounts and coins, and migrate legacy QUIL into a confidential wallet.'),
      page('confidential-wallet', 'Confidential Wallet', 'Scan confidential-wallet coins and unspent status.'),
      page('pending-escrows', 'Pending Escrows', 'Inspect pending transfers, status, and expiry.'),
    ],
  },
  {
    id: 'identity', label: 'Identity', labelKey: 'group.identity', icon: '◇',
    pages: [
      page('keys', 'Keys', 'Inspect, create, and delete local keys.'),
      page('aliases', 'Aliases', 'Inspect, add, update, and delete local address aliases.'),
    ],
  },
  {
    id: 'data', label: 'Data', labelKey: 'group.data', icon: '⬡',
    pages: [
      page('hypergraph', 'Hypergraph', 'Query a vertex or hyperedge by address.', 'form'),
      capabilityPage('deploy-token', 'Deploy Token', 'Create and submit a new token application.', 'deploy.token'),
    ],
  },
  {
    id: 'diagnostics', label: 'Diagnostics & logs', labelKey: 'group.diagnostics', icon: '⌁',
    pages: [
      page('health', 'Health', 'Read the active node health report.', 'details'),
      page('diagnostics', 'Diagnostics', 'Probe a listen address, identity, and network latency.', 'form'),
      page('diagnostics-identity', 'Diagnostics Identity', 'Inspect or create an independent diagnostics identity.', 'details'),
      page('peer-resolve', 'Peer Resolve', 'Resolve peer addresses from the node directory.', 'form'),
      page('logs', 'Logs', 'Inspect master or worker logs.'),
      page('log-export', 'Log Export', 'Preview redacted logs and export them locally.', 'form'),
      page('remote-health', 'Remote Health', 'Read a remote health report with the diagnostics identity.', 'form'),
    ],
  },
  {
    id: 'other', label: 'Other', labelKey: 'group.other', icon: '⚙',
    pages: [page('other', 'Console settings', 'Manage Web, port, language, and local Console settings.', 'settings')],
  },
  {
    id: 'help', label: 'Help', labelKey: 'group.help', icon: '?',
    pages: [page('help', 'Help', 'Read qclient-console feature and operation guidance.', 'help')],
  },
]

export const pages = NAV_GROUPS.flatMap((group) => group.pages.map((entry) => ({ ...entry, groupId: group.id, groupLabel: group.label })))

export function findPage(slug) {
  return pages.find((entry) => entry.slug === slug) || pages[0]
}
