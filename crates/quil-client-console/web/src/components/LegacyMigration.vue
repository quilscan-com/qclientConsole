<script setup>
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'

import { t } from '../locale.js'

const POLL_INTERVAL_MS = 12_000
const QUIL_BASE_UNITS = 8_000_000_000n
const QUIL_DISPLAY_SCALE = 1_000_000_000_000n

const props = defineProps({
  locale: { type: String, required: true },
  client: { type: Object, required: true },
  activeConfigName: { type: String, default: '' },
})
const emit = defineEmits(['unauthorized'])

const secret = ref('')
const reveal = ref(false)
const destinationMode = ref('current_node')
const existingPath = ref('')
const allowInitialize = ref(false)
const preflight = ref(null)
const status = ref(null)
const busy = ref('')
const errorKey = ref('')
const recoveryArmed = ref(false)
const observationChecked = ref(false)
const stageHeading = ref(null)
const alertRef = ref(null)
let pollTimer

const phaseCode = computed(() => migrationPhase(status.value?.phase))
const fingerprint = computed(() => status.value?.source?.fingerprint || preflight.value?.source?.fingerprint || '')
const maskedKey = computed(() => status.value?.source?.masked_key || preflight.value?.source?.masked_key || '')
const complete = computed(() => phaseCode.value === 'complete')
const generatedPackage = computed(() => {
  const taskDir = `${status.value?.task_dir || ''}/`
  return Boolean(status.value?.source_key_persisted === true
    && status.value?.recipient_config_dir === status.value?.task_dir
    && taskDir !== '/'
    && status.value?.config_path?.startsWith(taskDir)
    && status.value?.keys_path?.startsWith(taskDir))
})
const currentStage = computed(() => {
  if (complete.value) return 4
  if (status.value) return 3
  if (preflight.value) return 2
  return secret.value.trim() ? 1 : 0
})
const steps = computed(() => ['identity', 'destination', 'preflight', 'progress', 'files'].map((key, index) => ({
  key,
  state: index < currentStage.value ? 'done' : index === currentStage.value ? 'current' : 'pending',
})))
const feeTotal = computed(() => sumValues(preflight.value?.fees || []))
const sourceAmount = computed(() => preflight.value?.source_amount ?? status.value?.source_amount ?? null)
const progress = computed(() => {
  if (complete.value) return 100
  const details = phaseDetails(status.value?.phase)
  const total = Number(details.total ?? status.value?.batches?.length ?? 0)
  if (!total) return 0
  const batch = Number(details.batch ?? Math.min(total, Number(status.value?.current_batch ?? 0) + 1))
  return Math.max(0, Math.min(100, Math.round((batch / total) * 100)))
})
const batchLabel = computed(() => {
  const details = phaseDetails(status.value?.phase)
  const total = details.total ?? status.value?.batches?.length ?? 0
  const batch = details.batch ?? (total ? Math.min(Number(total), Number(status.value?.current_batch ?? 0) + 1) : 0)
  return `${batch}/${total}`
})
const canPreflight = computed(() => secret.value.trim().length > 0 && (destinationMode.value !== 'existing_config' || existingPath.value.trim().length > 0))
const canPoll = computed(() => ['prepared', 'migrating', 'awaiting_chain_confirmation', 'submission_unknown'].includes(phaseCode.value))
const needsSourceKey = computed(() => phaseCode.value === 'awaiting_source_key')
const needsRecovery = computed(() => ['submission_unknown', 'recoverable_error'].includes(phaseCode.value))
const phaseLabel = computed(() => t(props.locale, `migration.phase.${phaseCode.value || 'prepared'}`))
const error = computed(() => errorKey.value ? t(props.locale, errorKey.value) : '')
const localizedWarnings = computed(() => (preflight.value?.warnings || []).map((warning) => {
  const match = String(warning).match(/^(\d+) legacy outputs are outside current live shards$/)
  if (match) return t(props.locale, 'migration.warning.outsideLiveShards').replace('{count}', match[1])
  return t(props.locale, 'migration.warning.node')
}))
const nextActionLabel = computed(() => t(props.locale, ({
  prepared: 'migration.next.wait',
  migrating: 'migration.next.wait',
  awaiting_chain_confirmation: 'migration.next.observe',
  submission_unknown: observationChecked.value ? 'migration.next.reviewReplay' : 'migration.next.observe',
  recoverable_error: 'migration.next.fixAndReplay',
  awaiting_source_key: 'migration.next.enterKey',
  complete: 'migration.next.secureFiles',
})[phaseCode.value] || 'migration.next.wait'))
const observationLabel = computed(() => phaseCode.value === 'submission_unknown'
  ? t(props.locale, observationChecked.value ? 'migration.observationChecked' : 'migration.observationRequired')
  : '')
const recoveryActionLabel = computed(() => {
  if (phaseCode.value === 'submission_unknown' && !observationChecked.value) return t(props.locale, 'migration.observeNow')
  return t(props.locale, recoveryArmed.value ? 'migration.rebroadcastConfirm' : 'migration.rebroadcast')
})

function destinationPayload() {
  if (destinationMode.value === 'existing_config') return { mode: 'existing_config', path: existingPath.value.trim() }
  return { mode: destinationMode.value }
}

function migrationPhase(phase) {
  if (!phase) return ''
  if (typeof phase === 'string') return phase.replace(/[A-Z]/g, (letter) => `_${letter.toLowerCase()}`).replace(/^_/, '')
  return Object.keys(phase)[0] || ''
}

function phaseDetails(phase) {
  if (!phase || typeof phase !== 'object') return {}
  return phase[migrationPhase(phase)] || {}
}

function clearSecret() {
  secret.value = ''
  reveal.value = false
}

function sumValues(values) {
  try { return values.reduce((sum, value) => sum + BigInt(String(value)), 0n).toString() }
  catch { return '—' }
}

function formatQuil(value) {
  if (value == null) return '—'
  try {
    const raw = BigInt(String(value))
    let whole = raw / QUIL_BASE_UNITS
    const remainder = raw % QUIL_BASE_UNITS
    let fractional = (remainder * QUIL_DISPLAY_SCALE + QUIL_BASE_UNITS / 2n) / QUIL_BASE_UNITS
    if (fractional === QUIL_DISPLAY_SCALE) { whole += 1n; fractional = 0n }
    return `${whole}.${fractional.toString().padStart(12, '0')} QUIL`
  } catch { return String(value) }
}

function handleFailure(failure) {
  if (failure?.status === 401) emit('unauthorized')
  else if (failure?.status === 400) errorKey.value = 'migration.error.invalid'
  else if (failure?.status === 404) errorKey.value = 'migration.error.notFound'
  else if (failure?.status === 409) errorKey.value = 'migration.error.conflict'
  else if (failure?.status >= 500) errorKey.value = 'migration.error.node'
  else errorKey.value = 'migration.requestFailed'
}

function stopPolling() {
  clearTimeout(pollTimer)
  pollTimer = undefined
}

function schedulePolling() {
  stopPolling()
  if (!canPoll.value) return
  pollTimer = setTimeout(pollMigration, POLL_INTERVAL_MS)
}

function applyStatus(next, { observed = false } = {}) {
  const nextPhase = migrationPhase(next?.phase)
  status.value = next
  recoveryArmed.value = false
  observationChecked.value = nextPhase === 'submission_unknown' && observed
  errorKey.value = next?.error ? 'migration.error.savedOperation' : ''
  if (nextPhase === 'complete') stopPolling()
  else schedulePolling()
}

async function pollMigration() {
  if (!fingerprint.value || busy.value) { schedulePolling(); return }
  busy.value = 'poll'
  try {
    const latest = await props.client.legacyMigrationStatus(fingerprint.value)
    const latestPhase = migrationPhase(latest?.phase)
    if (['prepared', 'migrating', 'awaiting_chain_confirmation', 'submission_unknown'].includes(latestPhase)) {
      applyStatus(await props.client.advanceLegacyMigration(fingerprint.value, null), { observed: latestPhase === 'submission_unknown' })
    } else applyStatus(latest)
  } catch (failure) { handleFailure(failure); schedulePolling() }
  finally { busy.value = '' }
}

async function runPreflight() {
  if (!canPreflight.value) return
  busy.value = 'preflight'
  errorKey.value = ''
  const suppliedSecret = secret.value.trim()
  try {
    let existing = null
    try {
      existing = await props.client.lookupLegacyMigration(suppliedSecret)
    } catch (failure) {
      if (failure?.status !== 404) throw failure
    }
    if (existing) {
      try {
        if (migrationPhase(existing?.phase) === 'awaiting_source_key') {
          applyStatus(await props.client.advanceLegacyMigration(existing.source.fingerprint, suppliedSecret))
        } else applyStatus(existing)
      } finally { clearSecret() }
      return
    }
    const result = await props.client.preflightLegacyMigration({
      old_peer_private_key: suppliedSecret,
      destination: destinationPayload(),
      allow_initialize_recipient: destinationMode.value === 'create_new' ? false : allowInitialize.value,
    })
    preflight.value = result
    clearSecret()
  } catch (failure) { handleFailure(failure) }
  finally { busy.value = '' }
}

async function startMigration() {
  if (!preflight.value) return
  busy.value = 'start'
  errorKey.value = ''
  try { applyStatus(await props.client.startLegacyMigration(preflight.value)) }
  catch (failure) { handleFailure(failure) }
  finally { busy.value = '' }
}

async function resumeWithSourceKey() {
  if (!secret.value.trim() || !fingerprint.value) return
  busy.value = 'resume'
  errorKey.value = ''
  const suppliedSecret = secret.value.trim()
  try { applyStatus(await props.client.advanceLegacyMigration(fingerprint.value, suppliedSecret)) }
  catch (failure) { handleFailure(failure) }
  finally { clearSecret(); busy.value = '' }
}

async function recoverSavedOperation() {
  if (phaseCode.value === 'submission_unknown' && !observationChecked.value) {
    busy.value = 'observe'
    errorKey.value = ''
    try {
      const observed = await props.client.advanceLegacyMigration(fingerprint.value, null)
      applyStatus(observed, { observed: true })
      if (migrationPhase(observed?.phase) === 'submission_unknown') recoveryArmed.value = true
    } catch (failure) { handleFailure(failure) }
    finally { busy.value = '' }
    return
  }
  if (!recoveryArmed.value) { recoveryArmed.value = true; return }
  busy.value = 'recover'
  errorKey.value = ''
  try { applyStatus(await props.client.rebroadcastLegacyMigration(fingerprint.value, true)) }
  catch (failure) { handleFailure(failure) }
  finally { busy.value = '' }
}

function editPreflight() {
  stopPolling()
  preflight.value = null
  status.value = null
  recoveryArmed.value = false
  observationChecked.value = false
  errorKey.value = ''
  clearSecret()
}

function teardown() {
  stopPolling()
  clearSecret()
  preflight.value = null
}

onBeforeUnmount(teardown)

watch(currentStage, async (next, previous) => {
  if (next < 2 || next === previous) return
  await nextTick()
  stageHeading.value?.focus()
})

watch(errorKey, async (next) => {
  if (!next) return
  await nextTick()
  alertRef.value?.focus()
})
</script>

<template>
  <section class="migration-workspace" aria-labelledby="migration-title">
    <header class="migration-header">
      <div>
        <p class="eyebrow">LEGACY → CONFIDENTIAL</p>
        <h3 id="migration-title">{{ t(locale, 'migration.title') }}</h3>
        <p>{{ t(locale, 'migration.subtitle') }}</p>
      </div>
      <span class="migration-state" :class="{ complete }">{{ complete ? t(locale, 'migration.complete') : phaseLabel }}</span>
    </header>

    <ol class="migration-steps" :aria-label="t(locale, 'migration.steps')">
      <li v-for="(item, index) in steps" :key="item.key" :class="item.state" :aria-current="item.state === 'current' ? 'step' : undefined">
        <span>{{ item.state === 'done' ? '✓' : index + 1 }}</span>
        <b>{{ t(locale, `migration.${item.key}`) }}</b>
      </li>
    </ol>

    <div class="migration-live" aria-live="polite">
      <p v-if="error" ref="alertRef" class="migration-alert" role="alert" tabindex="-1">{{ error }}</p>
      <div v-if="busy === 'poll'" class="migration-polling"><span></span>{{ t(locale, 'migration.checkingChain') }}</div>
      <p v-if="status" class="visually-hidden">{{ phaseLabel }} · {{ progress }}% · {{ nextActionLabel }}<template v-if="observationLabel"> · {{ observationLabel }}</template></p>
    </div>

    <form v-if="!preflight && !status" class="migration-entry" @submit.prevent="runPreflight">
      <div class="migration-section-heading">
        <span>01</span>
        <div><h4>{{ t(locale, 'migration.identity') }}</h4><p>{{ t(locale, 'migration.identityHelp') }}</p></div>
      </div>
      <label class="migration-secret-field" for="legacy-peer-key">
        <span>{{ t(locale, 'migration.oldKey') }}</span>
        <div>
          <input v-if="reveal" id="legacy-peer-key" v-model="secret" type="text" autocomplete="off" autocapitalize="off" spellcheck="false" required>
          <input v-else id="legacy-peer-key" v-model="secret" type="password" autocomplete="off" autocapitalize="off" spellcheck="false" required>
          <button type="button" :aria-label="t(locale, reveal ? 'migration.hideKey' : 'migration.showKey')" :aria-pressed="reveal" @click="reveal = !reveal">{{ reveal ? '○' : '◉' }}</button>
        </div>
      </label>

      <fieldset class="migration-destination">
        <legend><span>02</span><b>{{ t(locale, 'migration.destination') }}</b></legend>
        <div class="migration-destination-options">
          <label :class="{ selected: destinationMode === 'current_node' }">
            <input v-model="destinationMode" type="radio" value="current_node">
            <span><b>{{ t(locale, 'migration.currentNode') }}</b><small>{{ activeConfigName || t(locale, 'migration.activeConfig') }}</small></span>
          </label>
          <label :class="{ selected: destinationMode === 'existing_config' }">
            <input v-model="destinationMode" type="radio" value="existing_config">
            <span><b>{{ t(locale, 'migration.existingConfig') }}</b><small>{{ t(locale, 'migration.existingConfigHint') }}</small></span>
          </label>
          <label :class="{ selected: destinationMode === 'create_new' }">
            <input v-model="destinationMode" type="radio" value="create_new">
            <span><b>{{ t(locale, 'migration.createNew') }}</b><small>{{ t(locale, 'migration.createNewHint') }}</small></span>
          </label>
        </div>
      </fieldset>

      <label v-if="destinationMode === 'existing_config'" class="migration-path-field" for="legacy-existing-path">
        <span>{{ t(locale, 'migration.configPath') }}</span>
        <input id="legacy-existing-path" v-model.trim="existingPath" type="text" autocomplete="off" placeholder="/home/user/.config/quil" required>
      </label>

      <label v-if="destinationMode !== 'create_new'" class="migration-initialize">
        <input v-model="allowInitialize" type="checkbox">
        <span><b>{{ t(locale, 'migration.allowInitialize') }}</b><small>{{ t(locale, 'migration.allowInitializeHint') }}</small></span>
      </label>

      <div class="migration-entry-actions">
        <p><span>⌁</span>{{ t(locale, 'migration.secretNote') }}</p>
        <button class="primary-button" :disabled="!canPreflight || Boolean(busy)">{{ busy === 'preflight' ? t(locale, 'common.processing') : t(locale, 'migration.runPreflight') }} <span aria-hidden="true">→</span></button>
      </div>
    </form>

    <div v-else-if="preflight && !status" class="migration-review">
      <div class="migration-operation-grid">
        <section>
          <div class="migration-section-heading"><span>03</span><div><h4 ref="stageHeading" tabindex="-1">{{ t(locale, 'migration.preflight') }}</h4><p>{{ t(locale, 'migration.reviewHelp') }}</p></div></div>
          <dl class="migration-metrics">
            <div><dt>{{ t(locale, 'migration.amount') }}</dt><dd>{{ formatQuil(preflight.source_amount) }}</dd></div>
            <div><dt>{{ t(locale, 'migration.outputs') }}</dt><dd>{{ preflight.source_count }}</dd></div>
            <div><dt>{{ t(locale, 'migration.batches') }}</dt><dd>{{ preflight.batch_count }}</dd></div>
            <div><dt>{{ t(locale, 'migration.fees') }}</dt><dd>{{ formatQuil(feeTotal) }}</dd></div>
            <div class="net"><dt>{{ t(locale, 'migration.net') }}</dt><dd>{{ formatQuil(preflight.estimated_net_amount) }}</dd></div>
          </dl>
        </section>
        <section class="migration-binding-summary">
          <dl>
            <div><dt>{{ t(locale, 'migration.oldKey') }}</dt><dd>{{ maskedKey }}</dd></div>
            <div><dt>{{ t(locale, 'migration.sourcePeer') }}</dt><dd :title="preflight.source.peer_id">{{ preflight.source.peer_id }}</dd></div>
            <div><dt>{{ t(locale, 'migration.destinationPath') }}</dt><dd :title="preflight.recipient_config_dir">{{ preflight.recipient_config_dir }}</dd></div>
            <div><dt>{{ t(locale, 'migration.configFilePath') }}</dt><dd :title="preflight.config_path">{{ preflight.config_path }}</dd></div>
            <div><dt>{{ t(locale, 'migration.keysFilePath') }}</dt><dd :title="preflight.keys_path">{{ preflight.keys_path }}</dd></div>
            <div><dt>{{ t(locale, 'migration.destinationAddress') }}</dt><dd :title="preflight.recipient_address">{{ preflight.recipient_address }}</dd></div>
            <div><dt>{{ t(locale, 'migration.readRpc') }}</dt><dd :title="preflight.read_endpoint">{{ preflight.read_endpoint }}</dd></div>
            <div><dt>{{ t(locale, 'migration.submitRpc') }}</dt><dd :title="preflight.submit_endpoint">{{ preflight.submit_endpoint }}</dd></div>
            <div><dt>{{ t(locale, 'migration.runningPeer') }}</dt><dd :title="preflight.running_peer_id">{{ preflight.running_peer_id }}</dd></div>
          </dl>
        </section>
      </div>
      <div class="migration-warnings" :class="{ empty: !preflight.warnings?.length }">
        <b>{{ t(locale, 'migration.warnings') }}</b>
        <ul v-if="localizedWarnings.length"><li v-for="(message, index) in localizedWarnings" :key="index">{{ message }}</li></ul>
        <p v-else>{{ t(locale, 'migration.noWarnings') }}</p>
      </div>
      <div class="migration-confirm-bar">
        <button type="button" class="migration-secondary" @click="editPreflight">{{ t(locale, 'migration.edit') }}</button>
        <button type="button" class="primary-button" :disabled="Boolean(busy)" @click="startMigration">{{ busy === 'start' ? t(locale, 'common.processing') : t(locale, 'migration.confirmStart') }} <span aria-hidden="true">→</span></button>
      </div>
    </div>

    <div v-else class="migration-progress-view">
      <div class="migration-operation-grid">
        <section>
          <div class="migration-section-heading"><span>{{ complete ? '05' : '04' }}</span><div><h4 ref="stageHeading" tabindex="-1">{{ t(locale, complete ? 'migration.files' : 'migration.progress') }}</h4><p>{{ phaseLabel }}</p></div></div>
          <div class="migration-progress-meter" :class="{ complete }">
            <div><span>{{ t(locale, 'migration.batch') }} {{ batchLabel }}</span><b>{{ progress }}%</b></div>
            <div class="migration-track"><span :style="{ width: `${progress}%` }"></span></div>
          </div>
          <dl class="migration-progress-meta">
            <div><dt>{{ t(locale, 'migration.oldKey') }}</dt><dd>{{ maskedKey }}</dd></div>
            <div><dt>{{ t(locale, 'migration.sourceAmount') }}</dt><dd>{{ formatQuil(sourceAmount) }}</dd></div>
            <div><dt>{{ t(locale, 'migration.phase') }}</dt><dd>{{ phaseLabel }}</dd></div>
            <div><dt>{{ t(locale, 'migration.nextAction') }}</dt><dd>{{ nextActionLabel }}</dd></div>
            <div v-if="observationLabel"><dt>{{ t(locale, 'migration.chainObservation') }}</dt><dd>{{ observationLabel }}</dd></div>
            <div><dt>{{ t(locale, 'migration.updated') }}</dt><dd>{{ status.updated_unix_ms || '—' }}</dd></div>
          </dl>
        </section>
        <section class="migration-binding-summary">
          <dl>
            <div v-if="complete"><dt>{{ t(locale, 'migration.taskPath') }}</dt><dd :title="status.task_dir">{{ status.task_dir }}</dd></div>
            <div v-if="complete"><dt>{{ t(locale, 'migration.activeConfigPath') }}</dt><dd :title="status.active_config_dir">{{ status.active_config_dir }}</dd></div>
            <div><dt>{{ t(locale, 'migration.sourcePeer') }}</dt><dd :title="status.source.peer_id">{{ status.source.peer_id }}</dd></div>
            <div><dt>{{ t(locale, 'migration.destinationPath') }}</dt><dd :title="status.recipient_config_dir">{{ status.recipient_config_dir }}</dd></div>
            <div><dt>{{ t(locale, 'migration.configFilePath') }}</dt><dd :title="status.config_path">{{ status.config_path }}</dd></div>
            <div><dt>{{ t(locale, 'migration.keysFilePath') }}</dt><dd :title="status.keys_path">{{ status.keys_path }}</dd></div>
            <div><dt>{{ t(locale, 'migration.destinationAddress') }}</dt><dd :title="status.recipient_address">{{ status.recipient_address }}</dd></div>
            <div><dt>{{ t(locale, 'migration.readRpc') }}</dt><dd :title="status.read_endpoint">{{ status.read_endpoint }}</dd></div>
            <div><dt>{{ t(locale, 'migration.submitRpc') }}</dt><dd :title="status.submit_endpoint">{{ status.submit_endpoint }}</dd></div>
            <div><dt>{{ t(locale, 'migration.runningPeer') }}</dt><dd :title="status.running_peer_id">{{ status.running_peer_id }}</dd></div>
          </dl>
        </section>
      </div>

      <form v-if="needsSourceKey" class="migration-recovery-panel" @submit.prevent="resumeWithSourceKey">
        <div><b>{{ t(locale, 'migration.awaitingSourceKey') }}</b><p>{{ t(locale, 'migration.awaitingSourceKeyHint') }}</p></div>
        <label for="legacy-resume-key"><span>{{ t(locale, 'migration.oldKey') }}</span><input id="legacy-resume-key" v-model="secret" type="password" autocomplete="off" autocapitalize="off" spellcheck="false" required></label>
        <button class="primary-button" :disabled="Boolean(busy) || !secret.trim()">{{ busy === 'resume' ? t(locale, 'common.processing') : t(locale, 'migration.resume') }}</button>
      </form>

      <div v-if="needsRecovery" class="migration-recovery-panel warning">
        <div><b>{{ phaseCode === 'submission_unknown' ? t(locale, 'migration.unknownTitle') : t(locale, 'migration.recoverableTitle') }}</b><p>{{ t(locale, phaseCode === 'submission_unknown' ? 'migration.unknownHint' : 'migration.recoverableHint') }} <strong v-if="observationLabel">{{ observationLabel }}</strong></p></div>
        <button class="recovery-button" :class="{ armed: recoveryArmed }" :disabled="Boolean(busy)" @click="recoverSavedOperation">{{ recoveryActionLabel }}</button>
        <button v-if="recoveryArmed" class="recovery-cancel" type="button" @click="recoveryArmed = false">{{ t(locale, 'common.cancel') }}</button>
      </div>

      <div v-if="complete" class="migration-complete-panel">
        <span aria-hidden="true">✓</span>
        <div><b>{{ t(locale, 'migration.complete') }}</b><p>{{ t(locale, generatedPackage ? 'migration.generatedFilesReady' : 'migration.existingFilesReady') }}</p></div>
        <div class="migration-complete-actions">
          <div v-if="generatedPackage" class="migration-downloads">
            <a :href="client.legacyMigrationFileUrl(fingerprint, 'config')" download="config.yml">{{ t(locale, 'migration.downloadConfig') }}</a>
            <a :href="client.legacyMigrationFileUrl(fingerprint, 'keys')" download="keys.yml">{{ t(locale, 'migration.downloadKeys') }}</a>
          </div>
          <button type="button" class="migration-secondary" @click="editPreflight">{{ t(locale, 'migration.startAnother') }}</button>
        </div>
      </div>
    </div>
  </section>
</template>
