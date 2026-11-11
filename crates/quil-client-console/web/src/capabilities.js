export const CAPABILITY_GROUPS = ['wallet', 'node', 'prover', 'identity', 'data', 'diagnostics', 'system']

export function createCapabilityGeneration() {
  let current = 0
  return {
    capture: () => current,
    invalidate: () => ++current,
    isCurrent: (generation) => generation === current,
  }
}

export function buildGroups(capabilities = []) {
  const buckets = new Map(CAPABILITY_GROUPS.map((id) => [id, []]))
  for (const capability of capabilities) {
    if (!buckets.has(capability.group)) buckets.set(capability.group, [])
    buckets.get(capability.group).push(capability)
  }
  return [...buckets.entries()]
    .map(([id, items]) => ({
      id,
      capabilities: [...items].sort((left, right) => String(left.id).localeCompare(String(right.id))),
    }))
    .filter((group) => group.capabilities.length > 0)
    .sort((left, right) => {
      const leftIndex = CAPABILITY_GROUPS.indexOf(left.id)
      const rightIndex = CAPABILITY_GROUPS.indexOf(right.id)
      return (leftIndex < 0 ? CAPABILITY_GROUPS.length : leftIndex) - (rightIndex < 0 ? CAPABILITY_GROUPS.length : rightIndex)
        || left.id.localeCompare(right.id)
    })
}

export function initialValues(spec = {}) {
  return Object.fromEntries((spec.fields || []).map((field) => {
    const defaults = Array.isArray(field.default_values) ? [...field.default_values] : []
    if (defaults.length) return [field.id, defaults]
    if (field.repeated) return [field.id, []]
    if (field.kind === 'boolean') return [field.id, ['false']]
    return [field.id, ['']]
  }))
}

export function abandonCapabilityDraft(mode, state = {}) {
  if (mode !== 'form') return state
  return {
    ...state,
    selectedCapability: null,
    capabilityValues: {},
    capabilityFieldErrors: {},
    capabilityReview: false,
    capabilityStartError: '',
    capabilityAnnouncement: '',
  }
}

export function normalizeCapabilityValues(spec = {}, values = {}) {
  const normalized = {}
  for (const field of spec.fields || []) {
    const entries = Array.isArray(values[field.id]) ? values[field.id] : []
    const present = field.kind === 'boolean'
      ? entries.map(String)
      : entries.filter((value) => String(value).trim() !== '').map(String)
    if (present.length) normalized[field.id] = present
  }
  return normalized
}

export function summarizeCapabilityValues(spec = {}, values = {}) {
  return (spec.fields || []).map((field) => {
    const entries = (values[field.id] || []).filter((value) => String(value).trim() !== '')
    return {
      id: field.id,
      label: field.label,
      value: entries.length ? field.sensitive ? '••••••••' : entries.join(', ') : '—',
    }
  })
}

export function firstCapabilityErrorFieldId(spec = {}, errors = {}) {
  return (spec.fields || []).find((field) => errors[field.id])?.id || ''
}

export function taskAnnouncement(previousState, nextState, pollFailed = false) {
  if (pollFailed) return 'poll_error'
  return nextState && previousState !== nextState ? nextState : ''
}

export function taskTone(state) {
  return ({ running: 'info', succeeded: 'success', failed: 'danger' })[state] || 'neutral'
}

export function isTerminalTask(task) {
  return task?.state === 'succeeded' || task?.state === 'failed'
}

export function validateCapabilityValues(spec = {}, values = {}) {
  const errors = {}
  for (const field of spec.fields || []) {
    const entries = Array.isArray(values[field.id]) ? values[field.id] : []
    const presentEntries = entries.filter((value) => String(value).trim() !== '')
    const present = presentEntries.length > 0
    if (field.required && !present) {
      errors[field.id] = 'required'
    } else if (field.kind === 'integer' && presentEntries.some((value) => !/^\d+$/.test(String(value).trim()))) {
      errors[field.id] = 'integer'
    } else if (field.kind === 'boolean' && entries.some((value) => !['true', 'false'].includes(String(value)))) {
      errors[field.id] = 'boolean'
    }
  }
  if (['node.manage', 'node.prover.manage'].includes(spec.id)
      && values.render?.[0] !== 'true') {
    errors.render = 'one_shot'
  }
  return errors
}
