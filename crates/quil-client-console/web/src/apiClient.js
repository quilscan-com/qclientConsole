async function request(fetcher, path, options = {}) {
  const response = await fetcher(path, {
    ...options,
    headers: { 'Content-Type': 'application/json', ...(options.headers || {}) },
  })
  const data = await response.json().catch(() => ({}))
  if (!response.ok) {
    const error = new Error(data.error || 'Request failed')
    error.status = response.status
    throw error
  }
  return data
}

function json(method, body) {
  return { method, body: JSON.stringify(body ?? {}) }
}

export function createApiClient(fetcher = globalThis.fetch) {
  return {
    bootstrap: () => request(fetcher, '/api/v1/bootstrap'),
    session: () => request(fetcher, '/api/v1/session'),
    setup: (payload) => request(fetcher, '/api/v1/setup', json('POST', payload)),
    login: (payload) => request(fetcher, '/api/v1/login', json('POST', payload)),
    logout: () => request(fetcher, '/api/v1/logout', json('POST')),
    context: () => request(fetcher, '/api/v1/context'),
    getCapabilities: () => request(fetcher, '/api/v1/capabilities'),
    startCapability: (id, values, confirmed) => request(fetcher, `/api/v1/capabilities/${encodeURIComponent(id)}/start`, json('POST', { values, confirmed })),
    getCapabilityTask: (id) => request(fetcher, `/api/v1/capability-tasks/${encodeURIComponent(id)}`),
    page: (slug, query = '', language = '') => {
      const parameters = []
      if (query) parameters.push(`q=${encodeURIComponent(query)}`)
      if (language) parameters.push(`lang=${encodeURIComponent(language)}`)
      return request(fetcher, `/api/v1/pages/${encodeURIComponent(slug)}${parameters.length ? `?${parameters.join('&')}` : ''}`)
    },
    mint: () => request(fetcher, '/api/v1/rewards/mint', json('POST')),
    rebroadcastMint: (confirmed) => request(fetcher, '/api/v1/rewards/mint/rebroadcast', json('POST', { confirmed })),
    lookupLegacyMigration: (oldPeerPrivateKey) => request(fetcher, '/api/v1/legacy-migrations/lookup', json('POST', { old_peer_private_key: oldPeerPrivateKey })),
    preflightLegacyMigration: (payload) => request(fetcher, '/api/v1/legacy-migrations/preflight', json('POST', payload)),
    startLegacyMigration: (approved) => request(fetcher, '/api/v1/legacy-migrations/start', json('POST', { approved })),
    legacyMigrationStatus: (fingerprint) => request(fetcher, `/api/v1/legacy-migrations/${encodeURIComponent(fingerprint)}`),
    advanceLegacyMigration: (fingerprint, oldPeerPrivateKey = null) => request(fetcher, `/api/v1/legacy-migrations/${encodeURIComponent(fingerprint)}/advance`, json('POST', { old_peer_private_key: oldPeerPrivateKey })),
    rebroadcastLegacyMigration: (fingerprint, confirmed) => request(fetcher, `/api/v1/legacy-migrations/${encodeURIComponent(fingerprint)}/rebroadcast`, json('POST', { confirmed })),
    legacyMigrationFileUrl: (fingerprint, kind) => `/api/v1/legacy-migrations/${encodeURIComponent(fingerprint)}/files/${kind}`,
    settings: () => request(fetcher, '/api/v1/settings'),
    updateSettings: (payload) => request(fetcher, '/api/v1/settings', json('PUT', payload)),
  }
}
