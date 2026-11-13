import { canonicalMintPhase, semanticLabels, t } from './locale.js'

function safeValue(value) {
  if (value == null || value === '') return '—'
  if (Array.isArray(value)) return value.length ? value.map((item) => safeValue(item)).filter((item) => item !== '—').join(' · ') || '—' : '—'
  if (typeof value === 'object') return '—'
  return value
}

function normalizeStats(stats) {
  if (!Array.isArray(stats)) return []
  return stats.map((stat, index) => ({
    key: stat.key || `stat-${index}`,
    label: stat.label || stat.name || '',
    value: safeValue(stat.value),
    tone: stat.tone || 'neutral',
    ...(stat.hint ? { hint: stat.hint } : {}),
  })).filter((stat) => stat.label)
}

function normalizeSections(sections) {
  if (!Array.isArray(sections)) return []
  return sections.map((section, index) => ({
    key: section.key || `section-${index}`,
    title: section.title || '',
    description: section.description || '',
    fields: Array.isArray(section.fields) ? section.fields.map((field, fieldIndex) => ({
      key: field.key || `field-${fieldIndex}`,
      label: field.label || '',
      value: safeValue(field.value),
      tone: field.tone || 'neutral',
      copyable: Boolean(field.copyable),
    })).filter((field) => field.label) : [],
  })).filter((section) => section.title || section.fields.length)
}

function normalizeTables(tables) {
  if (!Array.isArray(tables)) return []
  return tables.map((table, index) => ({
    key: table.key || `table-${index}`,
    title: table.title || '',
    description: table.description || '',
    columns: Array.isArray(table.columns) ? table.columns.map((column) => ({
      key: column.key,
      label: column.label || column.key,
      ...(column.align ? { align: column.align } : {}),
    })).filter((column) => column.key) : [],
    rows: Array.isArray(table.rows) ? table.rows : [],
  })).filter((table) => table.columns.length)
}

export function normalizePage(data) {
  const source = data && typeof data === 'object' && !Array.isArray(data) ? data : {}
  return {
    stats: normalizeStats(source.stats),
    sections: normalizeSections(source.sections),
    tables: normalizeTables(source.tables),
    emptyState: source.empty_state && typeof source.empty_state === 'object'
      ? { key: source.empty_state.key || 'default', title: source.empty_state.title || '', detail: source.empty_state.detail || source.empty_state.message || '' }
      : null,
  }
}

export function displayValue(value) {
  const safe = safeValue(value)
  if (typeof safe === 'boolean') return safe ? 'Yes' : 'No'
  if (typeof safe === 'number') return new Intl.NumberFormat('en-US').format(safe)
  return String(safe)
}

export function compactValue(value, max = 56) {
  const text = displayValue(value)
  return text.length > max ? `${text.slice(0, Math.max(12, max - 18))}…${text.slice(-14)}` : text
}

export function semanticLabel(locale, slug, kind, item) {
  const key = `semantic.${slug}.${kind}.${item?.key || ''}`
  const translated = t(locale, key)
  const fallback = kind === 'empty_detail' ? item?.detail : kind === 'empty_title' ? item?.title : item?.label || item?.title
  return translated === key ? fallback || '' : translated
}

export function semanticValue(locale, slug, kind, key, value) {
  if (['notice', 'action', 'health_notice'].includes(key)) return value
  if (typeof value === 'boolean') return t(locale, `semantic.value.${value ? 'yes' : 'no'}`)
  if (slug === 'health' && key === 'worker_coverage' && typeof value === 'string' && value.endsWith('/unknown')) {
    return `${value.slice(0, -'unknown'.length)}${t(locale, 'semantic.health.value.unknown')}`
  }
  if (typeof value !== 'string' || value === '—') return value
  const statusKeys = [
    'error_conditions', 'warning_conditions', 'info_conditions', 'conditions_reported',
    'master_store_write_failed', 'worker_capacity', 'store_status', 'store_io_verdict',
    'severity', 'health_severity',
  ]
  if (!statusKeys.includes(key)) return value
  const translationKeys = [`semantic.${slug}.${kind}.${key}.value.${value}`, `semantic.${slug}.value.${value}`]
  if (['severity', 'health_severity'].includes(key)) translationKeys.push(`semantic.value.${value}`)
  for (const translationKey of translationKeys) {
    const translated = t(locale, translationKey)
    if (translated !== translationKey) return translated
  }
  return value
}

const MINT_STEPS = [
  { key: 'authorization', label: 'Authorization' },
  { key: 'witness', label: 'Witness' },
  { key: 'claim', label: 'Claim' },
  { key: 'confirmed', label: 'Confirmed' },
]

function completedStepCount(phase = '') {
  const value = canonicalMintPhase(phase)
  if (/ClaimConfirmed|claim_confirmed/.test(value)) return 4
  if (/ClaimAccepted|claim_accepted/.test(value)) return 3
  if (/ProvingClaim|ClaimPrepared|SubmittingClaim|ClaimSubmitted|ClaimRejected|ClaimUnknown|proving_claim|claim_prepared|submitting_claim|claim_submitted|claim_rejected|claim_unknown/.test(value)) return 2
  if (/AuthorizationPrepared|SubmittingAuthorization|AwaitingAuthorizationWitness|AuthorizationUnknown|authorization_prepared|submitting_authorization|awaiting_authorization_witness|authorization_unknown/.test(value)) return 1
  return 0
}

export function rewardProgress(phase) {
  const completed = completedStepCount(phase)
  return MINT_STEPS.map((step, index) => ({
    ...step,
    state: index < completed ? 'done' : index === completed && completed < 4 ? 'current' : 'pending',
  }))
}

export function semanticField(page, key) {
  for (const section of page.sections || []) {
    const field = section.fields.find((item) => item.key === key)
    if (field) return field.value
  }
  const stat = (page.stats || []).find((item) => item.key === key)
  if (stat) return stat.value
  const labels = semanticLabels(key)
  for (const section of page.sections || []) {
    const field = section.fields.find((item) => labels.includes(item.label))
    if (field) return field.value
  }
  return (page.stats || []).find((item) => labels.includes(item.label))?.value ?? null
}
