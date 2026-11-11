<script setup>
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from 'vue'
import { createApiClient } from './apiClient.js'
import { abandonCapabilityDraft, buildGroups, createCapabilityGeneration, firstCapabilityErrorFieldId, initialValues, isTerminalTask, normalizeCapabilityValues, summarizeCapabilityValues, taskAnnouncement, taskTone, validateCapabilityValues } from './capabilities.js'
import { createIdleTimer } from './idleTimer.js'
import { applyDocumentLocale, canonicalMintPhase, LOCALES, semanticProofVerified, semanticValueIsAbsent, t } from './locale.js'
import { findPage, NAV_GROUPS } from './navigation.js'
import { unwrapPageResponse } from './pageResponse.js'
import { compactValue, displayValue, normalizePage, rewardProgress, semanticField } from './semanticView.js'
import LegacyMigration from './components/LegacyMigration.vue'

const IDLE_TIMEOUT_MS = 30 * 60 * 1000
const USER_ACTIVITY_EVENTS = ['pointerdown', 'keydown', 'touchstart']
const client = createApiClient()
const capabilityGeneration = createCapabilityGeneration()
const screen = ref('loading')
const busy = ref(false)
const actionBusy = ref('')
const error = ref('')
const context = ref(null)
const activeSlug = ref('overview')
const pageData = ref(null)
const pageLoading = ref(false)
const pageError = ref('')
const actionResult = ref(null)
const rebroadcastArmed = ref(false)
const query = ref('')
const lastUpdated = ref('')
const mobileNavigation = ref(false)
const credentials = ref({ username: 'admin', password: '', confirmation: '' })
const locale = ref(storedLocale())
applyDocumentLocale(locale.value)
const settingsDraft = ref({ enabled: true, port: 8888, language: 'en', read_rpc: '', effective_read_rpc: '', read_rpc_file: '' })
const settingsBusy = ref(false)
const settingsError = ref('')
const settingsMessage = ref('')
const restartRequired = ref(false)
const copyNotice = ref('')
const capabilities = ref([])
const capabilitiesLoading = ref(false)
const capabilitiesError = ref('')
const capabilitySearch = ref('')
const capabilityGroupFilter = ref('all')
const capabilityDrawerOpen = ref(false)
const capabilityDrawer = ref(null)
const capabilityDrawerMode = ref('form')
const selectedCapability = ref(null)
const capabilityValues = ref({})
const capabilityFieldErrors = ref({})
const capabilityReview = ref(false)
const capabilityBusy = ref(false)
const capabilityStartError = ref('')
const capabilityPollError = ref('')
const capabilityAnnouncement = ref('')
const currentTask = ref(null)
let refreshTimer
let copyNoticeTimer
let capabilityPollTimer
let capabilityOpener

const idleTimer = createIdleTimer({ timeoutMs: IDLE_TIMEOUT_MS, onIdle: expireIdleSession })
const activePage = computed(() => findPage(activeSlug.value))
const activeGroup = computed(() => NAV_GROUPS.find((group) => group.id === activePage.value.groupId) || NAV_GROUPS[0])
const activePageTitle = computed(() => t(locale.value, activePage.value.labelKey))
const activeGroupTitle = computed(() => t(locale.value, activeGroup.value.labelKey))
const connected = computed(() => context.value?.connected === true)
const statusText = computed(() => context.value?.connected == null ? t(locale.value, 'status.waiting') : connected.value ? t(locale.value, 'status.online') : t(locale.value, 'status.offline'))
const webAddress = computed(() => `http://${globalThis.location?.hostname || 'localhost'}:${settingsDraft.value.port}`)
const pageView = computed(() => normalizePage(pageData.value))
const frameLag = computed(() => {
  if (context.value?.last_received_frame == null || context.value?.last_global_head_frame == null) return null
  const received = Number(context.value?.last_received_frame)
  const head = Number(context.value?.last_global_head_frame)
  return Number.isFinite(received) && Number.isFinite(head) ? Math.max(0, head - received) : null
})
const overviewStats = computed(() => [
  { key: 'received', label: t(locale.value, 'overview.receivedFrame'), value: context.value?.last_received_frame, tone: 'violet' },
  { key: 'global', label: t(locale.value, 'overview.globalFrame'), value: context.value?.last_global_head_frame, tone: 'blue' },
  { key: 'lag', label: t(locale.value, 'overview.frameLag'), value: frameLag.value, tone: frameLag.value > 8 ? 'warning' : 'green' },
  { key: 'workers', label: t(locale.value, 'overview.workers'), value: context.value?.running_workers, tone: 'green' },
  { key: 'allocations', label: t(locale.value, 'overview.allocations'), value: context.value?.allocation_count, tone: 'amber' },
])
const rewardClaimable = computed(() => firstSemantic(['claimable', 'claimable_rewards', 'claimable_amount', 'claimable_base_units']) ?? pageView.value.stats[0]?.value ?? '—')
const rewardRecipient = computed(() => optionalSemantic(['recipient']))
const rewardProverAddress = computed(() => optionalSemantic(['prover_address']))
const rewardIdentity = computed(() => firstSemantic(['reward_identity', 'identity']))
const rewardLegacyAddress = computed(() => optionalSemantic(['legacy_account_address']))
const rewardProof = computed(() => firstSemantic(['proof_verified', 'proof']))
const rewardProofVerified = computed(() => semanticProofVerified(rewardProof.value))
const rewardFrame = computed(() => firstSemantic(['cited_frame', 'frame']))
const rewardPhaseCode = computed(() => firstSemantic(['phase_code']))
const rewardPhase = computed(() => firstSemantic(['mint_phase', 'phase']) || actionResult.value?.phase || '')
const rewardReceipt = computed(() => optionalSemantic(['mint_receipt', 'receipt']) || actionResult.value?.receipt)
const rewardWitnessError = computed(() => optionalSemantic(['witness_error']))
const rewardMintPollError = computed(() => optionalSemantic(['mint_poll_error']))
const rewardDetail = computed(() => {
  const phase = rewardPhaseCode.value || rewardPhase.value
  if (/claim_accepted|ClaimAccepted/.test(String(phase))) return t(locale.value, 'mint.acceptedWaiting')
  if (/claim_rejected|ClaimRejected/.test(String(phase))) return t(locale.value, 'mint.rejectedRecovery')
  return firstSemantic(['mint_detail', 'detail']) || actionResult.value?.detail
})
const rewardCanRebroadcast = computed(() => actionResult.value?.can_rebroadcast === true || firstSemantic(['can_rebroadcast']) === true)
const rewardSteps = computed(() => rewardProgress(rewardPhaseCode.value || rewardPhase.value))
const hasSemanticContent = computed(() => pageView.value.stats.length || pageView.value.sections.length || pageView.value.tables.length)
const capabilityGroups = computed(() => buildGroups(capabilities.value))
const filteredCapabilityGroups = computed(() => {
  const needle = capabilitySearch.value.trim().toLocaleLowerCase()
  return capabilityGroups.value
    .filter((group) => capabilityGroupFilter.value === 'all' || group.id === capabilityGroupFilter.value)
    .map((group) => ({
      ...group,
      capabilities: group.capabilities.filter((item) => {
        if (activePage.value.capabilityId && item.id !== activePage.value.capabilityId) return false
        if (!needle) return true
        const command = item.target?.command_path?.join(' ') || item.target?.route || ''
        return [item.id, item.title, item.description, command].some((value) => String(value || '').toLocaleLowerCase().includes(needle))
      }),
    }))
    .filter((group) => group.capabilities.length)
})
const filteredCapabilityCount = computed(() => filteredCapabilityGroups.value.reduce((total, group) => total + group.capabilities.length, 0))
const currentTaskCapability = computed(() => capabilities.value.find((item) => item.id === currentTask.value?.capability_id) || selectedCapability.value)
const capabilityCommand = computed(() => selectedCapability.value?.target?.command_path?.join(' ') || selectedCapability.value?.target?.route || '')
const currentTaskCommand = computed(() => currentTaskCapability.value?.target?.command_path?.join(' ') || currentTaskCapability.value?.target?.route || currentTask.value?.capability_id || '')
const capabilityNeedsConfirmation = computed(() => Boolean(selectedCapability.value && (!selectedCapability.value.read_only || selectedCapability.value.confirmation)))
const capabilitySummary = computed(() => summarizeCapabilityValues(selectedCapability.value, capabilityValues.value))

function firstSemantic(keys) {
  for (const key of keys) {
    const value = semanticField(pageView.value, key)
    if (value != null && value !== '—') return value
  }
  return null
}

function optionalSemantic(keys) {
  const value = firstSemantic(keys)
  return semanticValueIsAbsent(value) ? null : value
}

function safeLocaleTime() {
  return new Date().toLocaleTimeString(locale.value, { hour: '2-digit', minute: '2-digit', second: '2-digit' })
}

function pageDescription(page) {
  const translated = t(locale.value, page.descriptionKey)
  return translated === page.descriptionKey ? t(locale.value, 'description.default') : translated
}

function storedLocale() {
  try {
    const value = globalThis.localStorage?.getItem('qclient-console-language')
    return LOCALES.some((item) => item.id === value) ? value : 'en'
  } catch { return 'en' }
}

function rememberLocale(value) {
  locale.value = applyDocumentLocale(value)
  try { globalThis.localStorage?.setItem('qclient-console-language', locale.value) } catch { /* Storage may be disabled. */ }
}

async function bootstrap() {
  error.value = ''
  try {
    const state = await client.bootstrap()
    if (!state.initialized) { screen.value = 'setup'; return }
    await client.session()
    await enterConsole()
  } catch (failure) {
    screen.value = failure.status === 401 ? 'login' : 'error'
    if (failure.status !== 401) error.value = failure.message
  }
}

async function submitSetup() {
  if (credentials.value.password !== credentials.value.confirmation) { error.value = t(locale.value, 'login.mismatch'); return }
  await authenticate('setup')
}
async function submitLogin() { await authenticate('login') }
async function authenticate(mode) {
  busy.value = true
  error.value = ''
  try {
    if (mode === 'setup') await client.setup({ password: credentials.value.password, password_confirmation: credentials.value.confirmation })
    else await client.login({ username: credentials.value.username, password: credentials.value.password })
    credentials.value.password = ''
    credentials.value.confirmation = ''
    await enterConsole()
  } catch (failure) { error.value = failure.message } finally { busy.value = false }
}

async function enterConsole() {
  screen.value = 'dashboard'
  await loadSettings()
  if (screen.value !== 'dashboard') return
  await Promise.all([refreshContext(), loadPage(activeSlug.value), loadCapabilities()])
  if (screen.value !== 'dashboard') return
  startActivityTracking()
  clearInterval(refreshTimer)
  refreshTimer = setInterval(refreshContext, 5000)
}

async function loadCapabilities() {
  const generation = capabilityGeneration.capture()
  capabilitiesLoading.value = true
  capabilitiesError.value = ''
  try {
    const catalog = await client.getCapabilities()
    if (!capabilityRequestIsCurrent(generation)) return
    capabilities.value = Array.isArray(catalog) ? catalog : Array.isArray(catalog?.capabilities) ? catalog.capabilities : []
  } catch (failure) {
    if (!capabilityRequestIsCurrent(generation)) return
    if (failure.status === 401) expireFromServer()
    else capabilitiesError.value = failure.message || t(locale.value, 'capabilities.catalogError')
  } finally {
    if (capabilityRequestIsCurrent(generation)) capabilitiesLoading.value = false
  }
}

async function refreshContext() {
  try {
    context.value = await client.context()
    lastUpdated.value = safeLocaleTime()
    error.value = ''
    const phase = canonicalMintPhase(rewardPhaseCode.value || rewardPhase.value)
    if (activeSlug.value === 'rewards' && phase && !/^(not_started|claim_rejected|permanent_error)$/i.test(phase) && !rewardSteps.value.every((step) => step.state === 'done') && !pageLoading.value) void loadPage('rewards')
  } catch (failure) {
    if (failure.status === 401) expireFromServer()
    else error.value = failure.message
  }
}

async function loadPage(slug = activeSlug.value) {
  if (findPage(slug).layout === 'capabilities') { pageData.value = null; pageError.value = ''; return }
  if (slug === 'other') { pageData.value = null; pageError.value = ''; await loadSettings(); return }
  pageLoading.value = true
  pageError.value = ''
  if (slug !== 'rewards') actionResult.value = null
  try {
    const pageResult = unwrapPageResponse(await client.page(slug, query.value.trim(), locale.value))
    if (slug === activeSlug.value) { pageData.value = pageResult.data; pageError.value = pageResult.error }
  } catch (failure) {
    if (failure.status === 401) expireFromServer()
    else pageError.value = failure.message
  } finally { pageLoading.value = false }
}

async function selectPage(slug) {
  activeSlug.value = slug
  pageData.value = null
  query.value = ''
  pageError.value = ''
  actionResult.value = null
  mobileNavigation.value = false
  await loadPage(slug)
}
async function refreshAll() { await Promise.all([refreshContext(), activeSlug.value === 'other' ? loadSettings() : loadPage()]) }

async function refreshWorkspace() {
  if (activePage.value.layout === 'capabilities') await Promise.all([refreshContext(), loadCapabilities()])
  else await refreshAll()
}

function rememberCapabilityOpener(event) { capabilityOpener = event?.currentTarget || document.activeElement }

async function focusCapabilityDrawer() {
  await nextTick()
  if (capabilityDrawerOpen.value) capabilityDrawer.value?.focus()
}

function openCapability(capability, event) {
  if (capability.target?.kind === 'native') {
    void selectPage(capability.target.route)
    return
  }
  rememberCapabilityOpener(event)
  selectedCapability.value = capability
  capabilityValues.value = initialValues(capability)
  capabilityFieldErrors.value = {}
  capabilityReview.value = false
  capabilityStartError.value = ''
  capabilityAnnouncement.value = ''
  capabilityDrawerMode.value = 'form'
  capabilityDrawerOpen.value = true
  void focusCapabilityDrawer()
}

function openCurrentTask(event) {
  if (!currentTask.value) return
  rememberCapabilityOpener(event)
  selectedCapability.value = currentTaskCapability.value
  capabilityDrawerMode.value = 'task'
  capabilityDrawerOpen.value = true
  void focusCapabilityDrawer()
}

function resetCapabilityDraft(mode) {
  const state = abandonCapabilityDraft(mode, {
    selectedCapability: selectedCapability.value,
    capabilityValues: capabilityValues.value,
    capabilityFieldErrors: capabilityFieldErrors.value,
    capabilityReview: capabilityReview.value,
    capabilityStartError: capabilityStartError.value,
    capabilityAnnouncement: capabilityAnnouncement.value,
    currentTask: currentTask.value,
  })
  selectedCapability.value = state.selectedCapability
  capabilityValues.value = state.capabilityValues
  capabilityFieldErrors.value = state.capabilityFieldErrors
  capabilityReview.value = state.capabilityReview
  capabilityStartError.value = state.capabilityStartError
  capabilityAnnouncement.value = state.capabilityAnnouncement
  currentTask.value = state.currentTask
}

function closeCapabilityDrawer() {
  resetCapabilityDraft(capabilityDrawerMode.value)
  capabilityDrawerOpen.value = false
  void nextTick(() => {
    if (capabilityOpener?.isConnected) capabilityOpener.focus()
    capabilityOpener = null
  })
}

function trapCapabilityFocus(event) {
  const drawer = capabilityDrawer.value
  if (!drawer) return
  const focusable = [...drawer.querySelectorAll('button:not([disabled]), input:not([disabled]), select:not([disabled]), a[href], [tabindex]:not([tabindex="-1"])')]
  if (!focusable.length) {
    event.preventDefault()
    drawer.focus()
    return
  }
  const first = focusable[0]
  const last = focusable[focusable.length - 1]
  const active = document.activeElement
  if (event.shiftKey && (active === first || active === drawer || !drawer.contains(active))) {
    event.preventDefault()
    last.focus()
  } else if (!event.shiftKey && (active === last || !drawer.contains(active))) {
    event.preventDefault()
    first.focus()
  }
}

function setCapabilityValue(fieldId, index, value) {
  const next = [...(capabilityValues.value[fieldId] || [])]
  next[index] = String(value)
  capabilityValues.value = { ...capabilityValues.value, [fieldId]: next }
  if (capabilityFieldErrors.value[fieldId]) {
    const errors = { ...capabilityFieldErrors.value }
    delete errors[fieldId]
    capabilityFieldErrors.value = errors
  }
}

function setCapabilityBoolean(fieldId, index, checked) { setCapabilityValue(fieldId, index, checked ? 'true' : 'false') }

function addCapabilityValue(field) {
  const empty = field.kind === 'boolean' ? 'false' : ''
  capabilityValues.value = { ...capabilityValues.value, [field.id]: [...(capabilityValues.value[field.id] || []), empty] }
}

function removeCapabilityValue(fieldId, index) {
  capabilityValues.value = {
    ...capabilityValues.value,
    [fieldId]: (capabilityValues.value[fieldId] || []).filter((_, valueIndex) => valueIndex !== index),
  }
}

function prepareCapabilityStart() {
  if (!selectedCapability.value) return
  capabilityFieldErrors.value = validateCapabilityValues(selectedCapability.value, capabilityValues.value)
  capabilityStartError.value = ''
  if (Object.keys(capabilityFieldErrors.value).length) {
    void focusFirstCapabilityError(firstCapabilityErrorFieldId(selectedCapability.value, capabilityFieldErrors.value))
    return
  }
  if (capabilityNeedsConfirmation.value) { capabilityReview.value = true; return }
  void startSelectedCapability(false)
}

async function focusFirstCapabilityError(fieldId) {
  const field = selectedCapability.value?.fields?.find((item) => item.id === fieldId)
  if (!field) return
  await nextTick()
  const hasValueControl = (capabilityValues.value[field.id] || []).length > 0
  const targetId = hasValueControl ? capabilityFieldId(field) : capabilityFieldAddId(field)
  document.getElementById(targetId)?.focus()
}

function confirmCapabilityStart() { void startSelectedCapability(true) }

async function startSelectedCapability(confirmed) {
  if (!selectedCapability.value) return
  const generation = capabilityGeneration.capture()
  capabilityBusy.value = true
  capabilityStartError.value = ''
  capabilityPollError.value = ''
  clearTimeout(capabilityPollTimer)
  try {
    const submissionValues = normalizeCapabilityValues(selectedCapability.value, capabilityValues.value)
    const task = await client.startCapability(selectedCapability.value.id, submissionValues, confirmed)
    if (!capabilityRequestIsCurrent(generation)) return
    setCapabilityAnnouncement(taskAnnouncement('', task.state))
    currentTask.value = task
    capabilityValues.value = {}
    capabilityFieldErrors.value = {}
    capabilityDrawerMode.value = 'task'
    capabilityReview.value = false
    if (!isTerminalTask(task)) scheduleCapabilityPoll(generation)
  } catch (failure) {
    if (!capabilityRequestIsCurrent(generation)) return
    if (failure.status === 401) expireFromServer()
    else capabilityStartError.value = failure.message || t(locale.value, 'capabilities.startError')
  } finally {
    if (capabilityRequestIsCurrent(generation)) capabilityBusy.value = false
  }
}

function capabilityRequestIsCurrent(generation) {
  return capabilityGeneration.isCurrent(generation) && screen.value === 'dashboard'
}

function scheduleCapabilityPoll(generation) {
  if (!capabilityRequestIsCurrent(generation)) return
  clearTimeout(capabilityPollTimer)
  capabilityPollTimer = setTimeout(() => {
    if (capabilityRequestIsCurrent(generation)) void pollCapabilityTask()
  }, 1000)
}

async function pollCapabilityTask() {
  const generation = capabilityGeneration.capture()
  if (!capabilityRequestIsCurrent(generation) || !currentTask.value || isTerminalTask(currentTask.value)) return
  const taskId = currentTask.value.id
  const previousState = currentTask.value.state
  try {
    const task = await client.getCapabilityTask(taskId)
    if (!capabilityRequestIsCurrent(generation)) return
    if (currentTask.value?.id !== taskId) return
    currentTask.value = task
    setCapabilityAnnouncement(taskAnnouncement(previousState, task.state))
    capabilityPollError.value = ''
    if (!isTerminalTask(currentTask.value)) scheduleCapabilityPoll(generation)
  } catch (failure) {
    if (!capabilityRequestIsCurrent(generation)) return
    if (failure.status === 401) expireFromServer()
    else {
      capabilityPollError.value = failure.message || t(locale.value, 'capabilities.pollError')
      setCapabilityAnnouncement(taskAnnouncement(previousState, previousState, true))
      scheduleCapabilityPoll(generation)
    }
  }
}

function capabilityStateLabel(state) {
  const key = `capabilities.${state}`
  const translated = t(locale.value, key)
  return translated === key ? state || '—' : translated
}

function setCapabilityAnnouncement(code) {
  capabilityAnnouncement.value = code === 'poll_error'
    ? t(locale.value, 'capabilities.pollError')
    : code ? capabilityStateLabel(code) : ''
}

function capabilityFieldId(field, index = 0) { return `capability-${field.id.replace(/[^a-zA-Z0-9_-]/g, '-')}-${index}` }
function capabilityFieldAddId(field) { return `${capabilityFieldId(field)}-add` }
function capabilityFieldHelpId(field) { return `${capabilityFieldId(field)}-help` }
function capabilityFieldErrorId(field) { return `${capabilityFieldId(field)}-error` }
function capabilityFieldDescribedBy(field) {
  return [field.help ? capabilityFieldHelpId(field) : '', capabilityFieldErrors.value[field.id] ? capabilityFieldErrorId(field) : ''].filter(Boolean).join(' ') || undefined
}

function clearCapabilityState() {
  capabilityGeneration.invalidate()
  clearTimeout(capabilityPollTimer)
  capabilityPollTimer = undefined
  capabilities.value = []
  capabilitiesLoading.value = false
  capabilitiesError.value = ''
  capabilitySearch.value = ''
  capabilityGroupFilter.value = 'all'
  capabilityDrawerOpen.value = false
  capabilityOpener = null
  resetCapabilityDraft('form')
  capabilityDrawerMode.value = 'form'
  capabilityBusy.value = false
  capabilityPollError.value = ''
  currentTask.value = null
}

async function runMint() {
  rebroadcastArmed.value = false
  actionBusy.value = 'mint'
  pageError.value = ''
  try { actionResult.value = await client.mint(); await loadPage('rewards') }
  catch (failure) { if (failure.status === 401) expireFromServer(); else pageError.value = failure.message }
  finally { actionBusy.value = '' }
}

async function runMintRebroadcast() {
  if (!rebroadcastArmed.value) { rebroadcastArmed.value = true; return }
  actionBusy.value = 'rebroadcast'
  pageError.value = ''
  try {
    actionResult.value = await client.rebroadcastMint(true)
    rebroadcastArmed.value = false
    await loadPage('rewards')
  } catch (failure) {
    if (failure.status === 401) expireFromServer()
    else pageError.value = failure.message
  } finally { actionBusy.value = '' }
}

async function loadSettings() {
  settingsError.value = ''
  try {
    const data = await client.settings()
    const source = data?.settings || data || {}
    settingsDraft.value = {
      enabled: source.enabled ?? true,
      port: Number(source.port ?? 8888),
      language: source.language || 'en',
      read_rpc: source.read_rpc || '',
      effective_read_rpc: source.effective_read_rpc || '',
      read_rpc_file: source.read_rpc_file || '',
    }
    rememberLocale(settingsDraft.value.language)
    restartRequired.value = Boolean(source.restart_required)
  } catch (failure) {
    if (failure.status === 401) expireFromServer()
    else settingsError.value = failure.message
  }
}

async function saveSettings() {
  const port = Number(settingsDraft.value.port)
  if (!Number.isInteger(port) || port < 1 || port > 65535) { settingsError.value = t(locale.value, 'settings.invalidPort'); return }
  settingsBusy.value = true
  settingsError.value = ''
  settingsMessage.value = ''
  try {
    const data = await client.updateSettings({
      enabled: settingsDraft.value.enabled,
      port,
      language: settingsDraft.value.language,
      read_rpc: settingsDraft.value.read_rpc.trim() || null,
    })
    const source = data?.settings || data || {}
    settingsDraft.value.read_rpc = source.read_rpc || ''
    settingsDraft.value.effective_read_rpc = source.effective_read_rpc || ''
    settingsDraft.value.read_rpc_file = source.read_rpc_file || ''
    rememberLocale(settingsDraft.value.language)
    restartRequired.value = Boolean(data?.restart_required ?? data?.settings?.restart_required ?? true)
    settingsMessage.value = t(locale.value, 'settings.saved')
  } catch (failure) {
    if (failure.status === 401) expireFromServer()
    else settingsError.value = failure.message
  } finally { settingsBusy.value = false }
}

async function logout() {
  busy.value = true
  stopActivityTracking()
  clearCapabilityState()
  try { await client.logout() } finally { context.value = null; screen.value = 'login'; busy.value = false }
}
async function copyValue(value) {
  if (value == null || value === '—') return
  let copied = false
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(String(value))
      copied = true
    }
  } catch { copied = false }
  if (!copied) {
    const textarea = document.createElement('textarea')
    textarea.value = String(value)
    textarea.setAttribute('readonly', '')
    textarea.style.position = 'fixed'
    textarea.style.opacity = '0'
    document.body.appendChild(textarea)
    textarea.select()
    try { copied = document.execCommand('copy') } catch { copied = false }
    textarea.remove()
  }
  copyNotice.value = t(locale.value, copied ? 'common.copied' : 'common.copyFailed')
  clearTimeout(copyNoticeTimer)
  copyNoticeTimer = setTimeout(() => { copyNotice.value = '' }, 1800)
}
function startActivityTracking() {
  stopActivityTracking()
  USER_ACTIVITY_EVENTS.forEach((event) => window.addEventListener(event, recordActivity, { passive: true }))
  idleTimer.start()
}
function stopActivityTracking() {
  clearInterval(refreshTimer)
  refreshTimer = undefined
  idleTimer.stop()
  USER_ACTIVITY_EVENTS.forEach((event) => window.removeEventListener(event, recordActivity))
}
function recordActivity() { idleTimer.reset() }
function expireFromServer() { stopActivityTracking(); clearCapabilityState(); context.value = null; screen.value = 'login'; error.value = t(locale.value, 'error.expired') }
function expireIdleSession() {
  stopActivityTracking()
  clearCapabilityState()
  void client.logout().catch(() => {})
  context.value = null
  screen.value = 'login'
  error.value = t(locale.value, 'error.idle')
}

function teardown() {
  stopActivityTracking()
  clearCapabilityState()
  clearTimeout(copyNoticeTimer)
  window.removeEventListener('keydown', handleCapabilityEscape)
}

function handleCapabilityEscape(event) {
  if (event.key === 'Escape' && capabilityDrawerOpen.value) closeCapabilityDrawer()
}

onMounted(() => {
  window.addEventListener('keydown', handleCapabilityEscape)
  void bootstrap()
})
onBeforeUnmount(teardown)
</script>

<template>
  <main v-if="screen === 'loading'" class="center-shell" aria-live="polite"><div class="loader"></div><p>{{ t(locale, 'common.loading') }} qclient-console</p></main>

  <main v-else-if="screen === 'setup' || screen === 'login'" class="auth-shell">
    <section class="auth-story">
      <div class="brand-mark"><span></span><span></span><span></span></div><p class="eyebrow">QUILIBRIUM NODE CONSOLE</p>
      <h1>{{ t(locale, 'login.storyTitle') }}<br><em>{{ t(locale, 'login.storyAccent') }}</em></h1><p class="story-copy">{{ t(locale, 'login.storyCopy') }}</p>
      <div class="signal-card"><span class="signal-dot"></span><div><b>{{ t(locale, 'login.localReady') }}</b><small>{{ t(locale, 'login.localHint') }}</small></div></div>
    </section>
    <section class="auth-panel"><div class="auth-card">
      <div class="mobile-brand"><div class="brand-mark small"><span></span><span></span><span></span></div> qclient-console</div>
      <p class="step-label">{{ screen === 'setup' ? t(locale, 'login.firstRun') : t(locale, 'login.welcome') }}</p><h2>{{ screen === 'setup' ? t(locale, 'login.setupTitle') : t(locale, 'login.title') }}</h2><p class="form-hint">{{ t(locale, 'login.hint') }}</p>
      <form @submit.prevent="screen === 'setup' ? submitSetup() : submitLogin()">
        <label><span>{{ t(locale, 'login.account') }}</span><input v-model="credentials.username" autocomplete="username" readonly></label>
        <label><span>{{ t(locale, 'login.password') }}</span><input v-model="credentials.password" type="password" :autocomplete="screen === 'setup' ? 'new-password' : 'current-password'" minlength="10" placeholder="••••••••••" required autofocus></label>
        <label v-if="screen === 'setup'"><span>{{ t(locale, 'login.confirm') }}</span><input v-model="credentials.confirmation" type="password" autocomplete="new-password" minlength="10" placeholder="••••••••••" required></label>
        <p v-if="error" class="form-error" role="alert">{{ error }}</p><button class="primary-button" :disabled="busy"><span>{{ busy ? t(locale, 'common.processing') : screen === 'setup' ? t(locale, 'login.setupSubmit') : t(locale, 'login.submit') }}</span><span aria-hidden="true">→</span></button>
      </form><p class="local-note"><span>⌁</span> admin · localhost</p><div class="auth-locales"><button v-for="option in LOCALES" :key="option.id" :class="{ active: locale === option.id }" @click="rememberLocale(option.id)">{{ option.label }}</button></div>
    </div></section>
  </main>

  <main v-else-if="screen === 'error'" class="center-shell"><div class="error-card"><p class="eyebrow">CONNECTION ERROR</p><h2>{{ t(locale, 'error.console') }}</h2><p>{{ error }}</p><button class="primary-button" @click="bootstrap">{{ t(locale, 'error.retry') }}</button></div></main>

  <div v-else class="console-shell">
    <button class="mobile-menu" :aria-label="t(locale, 'common.openNavigation')" @click="mobileNavigation = !mobileNavigation"><span aria-hidden="true">☰</span></button>
    <aside class="sidebar" :class="{ open: mobileNavigation }">
      <div class="side-brand"><div class="brand-mark small"><span></span><span></span><span></span></div><div><b>qclient</b><small>console</small></div></div>
      <nav aria-label="qclient-console"><button v-for="group in NAV_GROUPS" :key="group.id" :class="{ active: activePage.groupId === group.id }" @click="selectPage(group.pages[0].slug)"><span class="nav-icon" aria-hidden="true">{{ group.icon }}</span><span class="nav-label">{{ t(locale, group.labelKey) }}</span><span v-if="group.pages.length > 1" class="nav-count">{{ group.pages.length }}</span></button></nav>
      <div class="sidebar-footer"><span class="avatar">A</span><div><b>admin</b><small>local operator</small></div><button :title="t(locale, 'common.logout')" :aria-label="t(locale, 'common.logout')" :disabled="busy" @click="logout"><span aria-hidden="true">↗</span></button></div>
    </aside>
    <button v-if="mobileNavigation" class="navigation-scrim" :aria-label="t(locale, 'common.closeNavigation')" @click="mobileNavigation = false"></button>

    <section class="workspace">
      <header class="topbar"><div><p class="breadcrumb">{{ activeGroupTitle }} / {{ activePageTitle }}</p><h1>{{ activePageTitle }}</h1></div><div class="top-actions"><button class="current-task-button" :class="currentTask ? `tone-${taskTone(currentTask.state)}` : ''" :disabled="!currentTask" @click="openCurrentTask($event)"><span aria-hidden="true">▶</span><span>{{ t(locale, 'capabilities.currentTask') }}</span><i v-if="currentTask">{{ capabilityStateLabel(currentTask.state) }}</i></button><span class="live-pill" :class="{ offline: !connected }"><i></i>{{ statusText }}</span><button class="refresh-button" :disabled="pageLoading || capabilitiesLoading" @click="refreshWorkspace">{{ t(locale, 'common.refresh') }} <span aria-hidden="true">↻</span></button></div></header>
      <nav v-if="activeGroup.pages.length > 1" class="subnav" :aria-label="activeGroupTitle"><button v-for="page in activeGroup.pages" :key="page.slug" :class="{ active: activeSlug === page.slug }" @click="selectPage(page.slug)">{{ t(locale, page.labelKey) }}</button></nav>

      <div class="content">
        <section class="page-intro"><div><p class="eyebrow">{{ activeGroupTitle }}</p><h2>{{ activePageTitle }}</h2><p>{{ pageDescription(activePage) }}</p></div><div v-if="activeSlug === 'rewards'" class="page-actions"><button class="action-button mint-button" :disabled="Boolean(actionBusy) || Boolean(rewardWitnessError)" @click="runMint">{{ actionBusy ? t(locale, 'common.processing') : t(locale, 'action.mint') }}</button></div></section>
        <p v-if="error" class="dashboard-error">{{ error }}</p><p v-if="pageError && activeSlug !== 'other'" class="dashboard-error" role="alert">{{ pageError }}</p><p v-if="copyNotice" class="copy-notice" role="status">{{ copyNotice }}</p>

        <section v-if="activePage.layout === 'capabilities'" class="capability-center">
          <div v-if="!activePage.capabilityId" class="capability-toolbar">
            <label><span>{{ t(locale, 'capabilities.search') }}</span><input v-model="capabilitySearch" type="search" name="capability-search" autocomplete="off" :placeholder="t(locale, 'capabilities.search')"></label>
            <label><span>{{ t(locale, 'capabilities.allGroups') }}</span><select v-model="capabilityGroupFilter" name="capability-group-filter"><option value="all">{{ t(locale, 'capabilities.allGroups') }}</option><option v-for="group in capabilityGroups" :key="group.id" :value="group.id">{{ t(locale, `capabilityGroup.${group.id}`) }}</option></select></label>
            <span class="capability-count">{{ filteredCapabilityCount }}</span>
          </div>
          <div v-if="capabilitiesLoading" class="capability-local-state"><div class="loader"></div><p>{{ t(locale, 'common.loading') }}</p></div>
          <div v-else-if="capabilitiesError" class="capability-local-state error" role="alert"><div><b>{{ t(locale, 'capabilities.catalogError') }}</b><p>{{ capabilitiesError }}</p></div><button class="refresh-button" @click="loadCapabilities">{{ t(locale, 'error.retry') }}</button></div>
          <div v-else-if="!filteredCapabilityGroups.length" class="capability-local-state"><span aria-hidden="true">◇</span><p>{{ t(locale, 'capabilities.noResults') }}</p></div>
          <template v-else>
            <section v-for="group in filteredCapabilityGroups" :key="group.id" class="capability-group">
              <header><h3>{{ t(locale, `capabilityGroup.${group.id}`) }}</h3><span>{{ group.capabilities.length }}</span></header>
              <div class="capability-list">
                <button v-for="item in group.capabilities" :key="item.id" class="capability-item" :disabled="currentTask?.state === 'running' && item.target.kind === 'embedded_cli'" @click="openCapability(item, $event)">
                  <span class="capability-item-main"><b>{{ item.title }}</b><small>{{ item.description || item.id }}</small></span>
                  <span class="capability-item-meta"><code>{{ item.target.command_path?.join(' ') || item.target.route }}</code><i>{{ item.target.kind === 'native' ? t(locale, 'capabilities.native') : item.read_only ? t(locale, 'capabilities.readOnly') : t(locale, 'capabilities.write') }}</i></span>
                  <span aria-hidden="true">→</span>
                </button>
              </div>
            </section>
          </template>
        </section>

        <LegacyMigration
          v-if="activeSlug === 'legacy-assets'"
          :locale="locale"
          :client="client"
          :active-config-name="context?.active_config_name || ''"
          @unauthorized="expireFromServer"
        />

        <template v-if="activePage.layout === 'overview'">
          <section class="node-status-strip">
            <div class="node-status-primary"><span class="status-indicator" :class="{ online: connected }"></span><div><b>{{ statusText }}</b><small>{{ context?.rpc_address || t(locale, 'common.unavailable') }}</small></div></div>
            <div class="config-chip"><span>{{ t(locale, 'overview.config') }}</span><b>{{ context?.active_config_name || '—' }}</b></div><p class="config-path" :title="context?.active_config_path">{{ compactValue(context?.active_config_path, 78) }}</p>
          </section>
          <section class="metric-grid overview-metrics"><article v-for="item in overviewStats" :key="item.key" class="metric-card" :class="item.tone"><div class="metric-head"><span></span><p>{{ item.label }}</p></div><strong>{{ displayValue(item.value) }}</strong></article></section>
          <section class="overview-grid">
            <article class="overview-card"><div class="card-heading"><div><p class="eyebrow">NODE</p><h3>{{ t(locale, 'overview.node') }}</h3></div><span class="card-index">01</span></div><dl class="compact-list"><div><dt>{{ t(locale, 'overview.rpc') }}</dt><dd>{{ compactValue(context?.rpc_address) }}</dd></div><div><dt>{{ t(locale, 'overview.peer') }}</dt><dd :title="context?.peer_id">{{ compactValue(context?.peer_id) }}</dd></div><div><dt>{{ t(locale, 'overview.config') }}</dt><dd>{{ context?.active_config_name || '—' }}</dd></div></dl><button class="text-link" @click="selectPage('node-status')">{{ t(locale, 'page.node-status') }} <span>→</span></button></article>
            <article class="overview-card"><div class="card-heading"><div><p class="eyebrow">EXECUTION</p><h3>{{ t(locale, 'overview.execution') }}</h3></div><span class="card-index">02</span></div><div class="execution-gauge"><div><strong>{{ displayValue(context?.running_workers) }}</strong><span>· {{ displayValue(context?.allocated_workers) }} {{ t(locale, 'overview.allocated') }}</span></div><p>{{ t(locale, 'overview.workers') }}</p></div><div class="progress-track"><span :style="{ width: `${Math.min(100, context?.running_workers ? (context.allocated_workers / context.running_workers) * 100 : 0)}%` }"></span></div><button class="text-link" @click="selectPage('prover')">{{ t(locale, 'page.prover') }} <span>→</span></button></article>
            <article class="overview-card quick-card"><div class="card-heading"><div><p class="eyebrow">ACTIONS</p><h3>{{ t(locale, 'overview.quick') }}</h3></div><span class="card-index">03</span></div><button @click="selectPage('rewards')"><span class="quick-icon">◈</span><span><b>{{ t(locale, 'page.rewards') }}</b><small>{{ t(locale, 'wallet.claimable') }}</small></span><i>→</i></button><button @click="selectPage('health')"><span class="quick-icon">⌁</span><span><b>{{ t(locale, 'page.health') }}</b><small>{{ statusText }}</small></span><i>→</i></button><button @click="selectPage('other')"><span class="quick-icon">⚙</span><span><b>{{ t(locale, 'page.other') }}</b><small>Web · {{ settingsDraft.port }}</small></span><i>→</i></button></article>
          </section>
        </template>

        <section v-if="activePage.layout === 'form'" class="query-card"><label><span>Query</span><input v-model="query" :placeholder="pageDescription(activePage)" @keyup.enter="loadPage()"></label><button class="action-button" :disabled="pageLoading" @click="loadPage()">{{ t(locale, 'common.query') }}</button></section>

        <p v-if="activeSlug === 'rewards' && rewardMintPollError" class="progress-detail dashboard-error" role="status" aria-live="polite" aria-atomic="true" :title="rewardMintPollError">{{ compactValue(rewardMintPollError, 96) }}</p>

        <template v-if="activeSlug === 'rewards' && !pageLoading">
          <section class="reward-grid">
            <article class="reward-balance-card">
              <p class="eyebrow">AVAILABLE REWARD</p><span>{{ t(locale, 'wallet.claimable') }}</span><strong>{{ displayValue(rewardClaimable) }}</strong>
              <div class="reward-wallet"><span>◈</span><div><small>{{ t(locale, 'wallet.recipient') }}</small><b :title="rewardRecipient">{{ compactValue(rewardRecipient, 48) }}</b></div><button v-if="rewardRecipient" :aria-label="t(locale, 'common.copy')" @click="copyValue(rewardRecipient)">⧉</button></div>
              <p v-if="!rewardRecipient" class="reward-destination-hint">{{ t(locale, 'wallet.recipientHint') }}</p>
              <p v-if="rewardWitnessError" class="reward-read-warning"><b>{{ t(locale, 'wallet.witnessUnavailable') }}</b><span :title="rewardWitnessError">{{ compactValue(rewardWitnessError, 96) }}</span></p>
              <p v-if="rewardIdentity" class="reward-identity">{{ t(locale, 'wallet.identity') }} · <b>{{ rewardIdentity }}</b></p>
              <button class="primary-button reward-mint" :disabled="Boolean(actionBusy) || Boolean(rewardWitnessError)" @click="runMint">{{ actionBusy ? t(locale, 'common.processing') : t(locale, 'wallet.mint') }} <span aria-hidden="true">→</span></button>
              <button v-if="rewardCanRebroadcast" class="recovery-button" :class="{ armed: rebroadcastArmed }" :disabled="Boolean(actionBusy)" @click="runMintRebroadcast">{{ rebroadcastArmed ? t(locale, 'mint.rebroadcastConfirm') : t(locale, 'mint.rebroadcast') }}</button>
              <button v-if="rewardCanRebroadcast && rebroadcastArmed" class="recovery-cancel" type="button" @click="rebroadcastArmed = false">{{ t(locale, 'common.cancel') }}</button>
            </article>
            <article class="mint-progress-card">
              <div class="card-heading"><div><p class="eyebrow">REWARD CLAIM</p><h3>{{ t(locale, 'wallet.progress') }}</h3></div><span class="phase-badge">{{ rewardPhase || 'Ready' }}</span></div>
              <ol class="mint-progress"><li v-for="(step, index) in rewardSteps" :key="step.key" :class="step.state"><span>{{ step.state === 'done' ? '✓' : index + 1 }}</span><div><b>{{ t(locale, `mint.${step.key}`) }}</b><small>{{ step.state }}</small></div></li></ol>
              <dl class="reward-meta">
                <div><dt>{{ t(locale, 'wallet.proof') }}</dt><dd :class="{ success: rewardProofVerified }">{{ rewardProofVerified ? t(locale, 'wallet.verified') : rewardProof || t(locale, 'wallet.notVerified') }}</dd></div>
                <div v-if="rewardProverAddress"><dt>{{ t(locale, 'wallet.prover') }}</dt><dd :title="rewardProverAddress">{{ compactValue(rewardProverAddress, 42) }}</dd></div>
                <div><dt>{{ t(locale, 'wallet.citedFrame') }}</dt><dd>{{ displayValue(rewardFrame) }}</dd></div>
                <div v-if="rewardLegacyAddress"><dt>{{ t(locale, 'wallet.legacy') }}</dt><dd :title="rewardLegacyAddress">{{ compactValue(rewardLegacyAddress, 42) }}</dd></div>
                <div v-if="rewardReceipt"><dt>{{ t(locale, 'wallet.receipt') }}</dt><dd :title="rewardReceipt">{{ compactValue(rewardReceipt, 42) }}</dd></div>
              </dl>
              <p v-if="rewardDetail" class="progress-detail">{{ rewardDetail }}</p>
            </article>
          </section>
        </template>

        <template v-if="activeSlug === 'other'">
          <section class="settings-grid">
            <form class="settings-card" @submit.prevent="saveSettings"><div class="card-heading"><div><p class="eyebrow">WEB CONSOLE</p><h3>{{ t(locale, 'page.other') }}</h3></div><span class="settings-state" :class="{ off: !settingsDraft.enabled }">{{ settingsDraft.enabled ? t(locale, 'settings.enabled') : t(locale, 'settings.disabled') }}</span></div><div class="setting-row"><div><b>{{ t(locale, 'settings.web') }}</b><p>{{ t(locale, 'settings.webHint') }}</p></div><label class="settings-switch"><input v-model="settingsDraft.enabled" type="checkbox"><span></span></label></div><label class="setting-field"><span>{{ t(locale, 'settings.port') }}</span><div class="port-input"><span>:</span><input v-model.number="settingsDraft.port" type="number" min="1" max="65535" inputmode="numeric"></div></label><label class="setting-field"><span>{{ t(locale, 'settings.readRpc') }}</span><div class="read-rpc-input"><input v-model.trim="settingsDraft.read_rpc" type="text" placeholder="/ip4/127.0.0.1/tcp/8337"><small>{{ t(locale, 'settings.readRpcHint') }}</small></div></label><fieldset class="language-field"><legend>{{ t(locale, 'settings.language') }}</legend><div class="language-grid"><button v-for="option in LOCALES" :key="option.id" type="button" :class="{ active: settingsDraft.language === option.id }" @click="settingsDraft.language = option.id">{{ option.label }}</button></div></fieldset><div class="restart-notice" :class="{ required: restartRequired }"><span>↻</span><p>{{ t(locale, 'settings.restartHint') }}</p></div><p v-if="settingsError" class="form-error" role="alert">{{ settingsError }}</p><p v-if="settingsMessage" class="success-message">{{ settingsMessage }}</p><button class="primary-button settings-save" :disabled="settingsBusy">{{ settingsBusy ? t(locale, 'common.processing') : t(locale, 'common.save') }}</button></form>
            <aside class="settings-card local-settings"><div class="card-heading"><div><p class="eyebrow">WEB ACCESS</p><h3>{{ t(locale, 'settings.local') }}</h3></div><span class="settings-state" :class="{ off: !settingsDraft.enabled }">{{ settingsDraft.enabled ? t(locale, 'settings.enabled') : t(locale, 'settings.disabled') }}</span></div><dl class="compact-list"><div><dt>{{ t(locale, 'settings.address') }}</dt><dd :title="webAddress">{{ webAddress }}</dd></div><div><dt>{{ t(locale, 'settings.protocol') }}</dt><dd>HTTP</dd></div><div><dt>{{ t(locale, 'settings.session') }}</dt><dd>{{ t(locale, 'settings.sessionValue') }}</dd></div><div><dt>{{ t(locale, 'settings.nodeConfig') }}</dt><dd :title="context?.active_config_name">{{ context?.active_config_name || '—' }}</dd></div><div><dt>{{ t(locale, 'settings.effectiveReadRpc') }}</dt><dd :title="settingsDraft.effective_read_rpc">{{ settingsDraft.effective_read_rpc || '—' }}</dd></div><div><dt>{{ t(locale, 'settings.readRpcFile') }}</dt><dd :title="settingsDraft.read_rpc_file">{{ settingsDraft.read_rpc_file || '—' }}</dd></div></dl><p class="local-security-note"><span>⌁</span>{{ t(locale, 'settings.accessHint') }}</p></aside>
          </section>
        </template>

        <section v-if="pageView.stats.length && activePage.layout !== 'overview' && activePage.layout !== 'capabilities' && activeSlug !== 'rewards' && activeSlug !== 'other'" class="metric-grid page-metrics"><article v-for="item in pageView.stats" :key="item.key" class="metric-card" :class="item.tone"><div class="metric-head"><span></span><p>{{ item.label }}</p></div><strong>{{ displayValue(item.value) }}</strong><small v-if="item.hint">{{ item.hint }}</small></article></section>
        <section v-if="pageLoading && activePage.layout !== 'overview' && activePage.layout !== 'capabilities'" class="data-card loading-card"><div class="loader"></div><p>{{ t(locale, 'common.loading') }} {{ activePageTitle }}</p></section>

        <template v-else-if="activePage.layout !== 'overview' && activePage.layout !== 'capabilities' && activeSlug !== 'rewards' && activeSlug !== 'other'">
          <section v-for="section in pageView.sections" :key="section.key" class="data-card semantic-section"><div class="section-heading"><div><h3>{{ section.title }}</h3><p v-if="section.description">{{ section.description }}</p></div><span>{{ String(pageView.sections.indexOf(section) + 1).padStart(2, '0') }}</span></div><dl class="semantic-fields"><div v-for="field in section.fields" :key="field.key"><dt>{{ field.label }}</dt><dd :class="field.tone" :title="displayValue(field.value)">{{ compactValue(field.value) }}<button v-if="field.copyable" :aria-label="t(locale, 'common.copy')" @click="copyValue(field.value)">⧉</button></dd></div></dl></section>
          <section v-for="table in pageView.tables" :key="table.key" class="data-card table-card"><div class="section-heading"><div><h3>{{ table.title }}</h3><p v-if="table.description">{{ table.description }}</p></div><span>{{ table.rows.length }}</span></div><div class="table-scroll"><table><thead><tr><th v-for="column in table.columns" :key="column.key" :class="`align-${column.align || 'left'}`">{{ column.label }}</th></tr></thead><tbody><tr v-for="(row, index) in table.rows" :key="row.id || row.name || index"><td v-for="column in table.columns" :key="column.key" :class="`align-${column.align || 'left'}`" :title="displayValue(row[column.key])">{{ compactValue(row[column.key]) }}</td></tr></tbody></table></div></section>
          <section v-if="pageView.emptyState || (!hasSemanticContent && !pageError)" class="data-card empty-state"><span aria-hidden="true">◇</span><div><h3>{{ pageView.emptyState?.title || t(locale, 'common.empty') }}</h3><p>{{ pageView.emptyState?.detail || t(locale, 'common.unavailable') }}</p></div></section>
        </template>

        <footer><span>{{ t(locale, 'footer.config') }} <b>{{ context?.active_config_name || '—' }}</b></span><span>{{ t(locale, 'footer.updated') }} <b>{{ lastUpdated || '—' }}</b></span><span>qclient-console Web</span></footer>
      </div>
    </section>

    <template v-if="capabilityDrawerOpen">
      <button class="capability-scrim" :aria-label="t(locale, 'capabilities.close')" @click="closeCapabilityDrawer"></button>
      <aside ref="capabilityDrawer" class="capability-drawer" role="dialog" aria-modal="true" aria-labelledby="capability-drawer-title" tabindex="-1" @keydown.tab="trapCapabilityFocus" @keydown.esc="closeCapabilityDrawer">
        <header class="capability-drawer-header"><div><p class="eyebrow">QCLIENT</p><h2 id="capability-drawer-title">{{ capabilityDrawerMode === 'task' ? t(locale, 'capabilities.currentTask') : selectedCapability?.title }}</h2></div><button :aria-label="t(locale, 'capabilities.close')" @click="closeCapabilityDrawer">×</button></header>

        <template v-if="capabilityDrawerMode === 'form' && selectedCapability">
          <div class="capability-command"><span>{{ t(locale, 'capabilities.command') }}</span><code>qclient {{ capabilityCommand }}</code><p>{{ selectedCapability.description }}</p></div>
          <form v-if="!capabilityReview" class="capability-form" @submit.prevent="prepareCapabilityStart">
            <fieldset v-for="field in selectedCapability.fields" :key="field.id" class="capability-field" :class="{ invalid: capabilityFieldErrors[field.id] }">
              <legend>{{ field.label }} <em>{{ t(locale, field.required ? 'capabilities.required' : 'capabilities.optional') }}</em></legend>
              <p v-if="field.help" :id="capabilityFieldHelpId(field)" class="capability-help">{{ field.help }}</p>
              <template v-if="field.repeated">
                <div v-for="(value, index) in capabilityValues[field.id]" :key="index" class="capability-repeat-row">
                  <label v-if="field.kind === 'boolean'" class="capability-boolean" :for="capabilityFieldId(field, index)"><input :id="capabilityFieldId(field, index)" :name="`${field.id}[${index}]`" type="checkbox" autocomplete="off" :checked="value === 'true'" :aria-required="field.required" :aria-invalid="Boolean(capabilityFieldErrors[field.id])" :aria-describedby="capabilityFieldDescribedBy(field)" @change="setCapabilityBoolean(field.id, index, $event.target.checked)"><span>{{ field.label }} · {{ index + 1 }} · {{ value === 'true' ? 'true' : 'false' }}</span></label>
                  <template v-else>
                    <label class="capability-input-label" :for="capabilityFieldId(field, index)">{{ field.label }} · {{ index + 1 }}</label>
                    <input :id="capabilityFieldId(field, index)" :name="`${field.id}[${index}]`" :value="value" :type="field.sensitive ? 'password' : 'text'" :autocomplete="field.sensitive ? 'new-password' : 'off'" :inputmode="field.kind === 'integer' ? 'numeric' : 'text'" :pattern="field.kind === 'integer' ? '[0-9]*' : undefined" :aria-required="field.required" :aria-invalid="Boolean(capabilityFieldErrors[field.id])" :aria-describedby="capabilityFieldDescribedBy(field)" @input="setCapabilityValue(field.id, index, $event.target.value)">
                  </template>
                  <button type="button" @click="removeCapabilityValue(field.id, index)">{{ t(locale, 'capabilities.remove') }}</button>
                </div>
                <button :id="capabilityFieldAddId(field)" class="capability-add" type="button" :aria-describedby="capabilityFieldDescribedBy(field)" @click="addCapabilityValue(field)">＋ {{ t(locale, 'capabilities.add') }}</button>
              </template>
              <label v-else-if="field.kind === 'boolean'" class="capability-boolean" :for="capabilityFieldId(field)"><input :id="capabilityFieldId(field)" :name="field.id" type="checkbox" autocomplete="off" :checked="capabilityValues[field.id]?.[0] === 'true'" :aria-required="field.required" :aria-invalid="Boolean(capabilityFieldErrors[field.id])" :aria-describedby="capabilityFieldDescribedBy(field)" @change="setCapabilityBoolean(field.id, 0, $event.target.checked)"><span>{{ field.label }} · {{ capabilityValues[field.id]?.[0] === 'true' ? 'true' : 'false' }}</span></label>
              <template v-else>
                <label class="capability-input-label" :for="capabilityFieldId(field)">{{ field.label }}</label>
                <input :id="capabilityFieldId(field)" :name="field.id" :value="capabilityValues[field.id]?.[0]" :type="field.sensitive ? 'password' : 'text'" :autocomplete="field.sensitive ? 'new-password' : 'off'" :inputmode="field.kind === 'integer' ? 'numeric' : 'text'" :pattern="field.kind === 'integer' ? '[0-9]*' : undefined" :aria-required="field.required" :aria-invalid="Boolean(capabilityFieldErrors[field.id])" :aria-describedby="capabilityFieldDescribedBy(field)" @input="setCapabilityValue(field.id, 0, $event.target.value)">
              </template>
              <p v-if="capabilityFieldErrors[field.id]" :id="capabilityFieldErrorId(field)" class="capability-field-error" role="alert">{{ t(locale, `capabilities.validation.${capabilityFieldErrors[field.id]}`) }}</p>
            </fieldset>
            <p v-if="capabilityStartError" class="form-error" role="alert">{{ capabilityStartError }}</p>
            <button class="primary-button capability-run" :disabled="capabilityBusy">{{ capabilityBusy ? t(locale, 'common.processing') : capabilityNeedsConfirmation ? t(locale, 'capabilities.review') : t(locale, 'capabilities.run') }} <span aria-hidden="true">→</span></button>
          </form>

          <section v-else class="capability-review">
            <div class="capability-review-alert"><span aria-hidden="true">!</span><div><h3>{{ t(locale, 'capabilities.review') }}</h3><p>{{ selectedCapability.confirmation }}</p></div></div>
            <dl><div><dt>{{ t(locale, 'capabilities.activeConfig') }}</dt><dd>{{ context?.active_config_name || '—' }}</dd></div><div><dt>{{ t(locale, 'capabilities.command') }}</dt><dd>{{ selectedCapability.title }}</dd></div><div v-for="item in capabilitySummary" :key="item.id"><dt>{{ item.label }}</dt><dd>{{ item.value }}</dd></div></dl>
            <p v-if="capabilityStartError" class="form-error" role="alert">{{ capabilityStartError }}</p>
            <div class="capability-review-actions"><button type="button" :disabled="capabilityBusy" @click="capabilityReview = false">{{ t(locale, 'common.cancel') }}</button><button class="primary-button" type="button" :disabled="capabilityBusy" @click="confirmCapabilityStart">{{ capabilityBusy ? t(locale, 'common.processing') : t(locale, 'capabilities.confirm') }} <span aria-hidden="true">→</span></button></div>
          </section>
        </template>

        <section v-else-if="currentTask" class="capability-task">
          <p class="capability-task-announcement" role="status" aria-live="polite" aria-atomic="true">{{ capabilityAnnouncement }}</p>
          <div class="capability-task-status"><span :class="`tone-${taskTone(currentTask.state)}`">{{ capabilityStateLabel(currentTask.state) }}</span><code>qclient {{ currentTaskCommand }}</code><dl><div><dt>ID</dt><dd>{{ currentTask.id }}</dd></div><div><dt>{{ t(locale, 'capabilities.exitCode') }}</dt><dd>{{ currentTask.exit_code ?? '—' }}</dd></div></dl></div>
          <p v-if="capabilityPollError" class="form-error">{{ capabilityPollError }}</p>
          <section class="capability-output"><h3>{{ t(locale, 'capabilities.stdout') }}</h3><pre v-if="currentTask.stdout">{{ currentTask.stdout }}</pre><p v-else>{{ t(locale, 'capabilities.emptyOutput') }}</p></section>
          <section class="capability-output stderr"><h3>{{ t(locale, 'capabilities.stderr') }}</h3><pre v-if="currentTask.stderr">{{ currentTask.stderr }}</pre><p v-else>{{ t(locale, 'capabilities.emptyOutput') }}</p></section>
        </section>
        <div v-else class="capability-local-state"><p>{{ t(locale, 'capabilities.noTask') }}</p></div>
      </aside>
    </template>
  </div>
</template>
