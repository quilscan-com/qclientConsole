export const TABLE_KEYS = [
  'items', 'rows', 'records', 'entries', 'allocations', 'shards', 'logs',
  'archives', 'coins', 'escrows', 'keys', 'aliases', 'workers', 'lines',
]

export function unwrapPageResponse(response) {
  if (!response?.available) {
    return { data: null, error: response?.message || 'This feature is currently unavailable' }
  }
  return { data: response.data, error: '' }
}

export function pageTables(data) {
  if (Array.isArray(data)) return [{ key: 'data', rows: data }]
  const tables = []
  for (const key of TABLE_KEYS) {
    if (!Array.isArray(data?.[key])) continue
    const rows = key === 'lines' ? data[key].map((line) => ({ line })) : data[key]
    tables.push({ key, rows })
  }
  return tables
}

export function pageColumns(rows) {
  const first = rows.find((row) => row && typeof row === 'object' && !Array.isArray(row))
  return first ? Object.keys(first) : []
}
