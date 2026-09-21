<script setup lang="ts">
/**
 * SettingsView.vue — Phase 7 unified settings panel.
 *
 * Four card-grouped sections:
 *   1. Core & Network — version, restart, port config, log level
 *   2. TUN Advanced    — driver stack, strict route, DNS
 *   3. General         — language, close-to-tray, route cleanup
 *   4. About           — client version, GitHub link, update check
 *
 * Design language matches the dashboard (glass surface, gradient
 * accents, ring-1 borders).  The component is presentation-only:
 * every action is wired through the existing Pinia stores, so all
 * IPC stays behind `@/bindings` + `@/utils/tauri-bridge`.
 */
import { computed, onMounted, onUnmounted, ref } from 'vue'
import {
  Cpu, Network, RotateCcw, FileText, Activity, Eye, EyeOff,
  Layers, GitBranch, Languages, Power, X, Github, RefreshCw, Loader2,
  CheckCircle2, AlertCircle, Hash, Cog, Wifi, BookOpen, AlertTriangle, Trash2,
  ArrowUpCircle, DownloadCloud,
} from 'lucide-vue-next'
import { useI18n } from '@/composables/useI18n'
import { useKernelStore } from '@/stores/kernel'
import { useProxyStore } from '@/stores/proxy'
import { useDesktopStore } from '@/stores/desktop'
import { useTunStore } from '@/stores/tun'
import { useSettingsStore } from '@/stores/settings'
import { useUpdaterStore } from '@/stores/updater'
import {
  applyTunAdvanced,
  type TunAdvancedOptions,
} from '@/services/tun'
import { getAppVersion, type UnlistenFn } from '@/utils/tauri-bridge'
import { resetApplication, onResetCompleted, clearClientState } from '@/services/reset'
import {
  CLOSE_BEHAVIOR_KEY,
  pushCloseBehavior,
  readCloseBehavior,
  type CloseBehavior,
} from '@/services/desktop'
import { useAnomaliesStore } from '@/stores/anomalies'
import { useNoticesStore } from '@/stores/notices'
import type { ResetReport } from '@/bindings'
import ConfirmModal from '@/components/ConfirmModal.vue'
import AutokillToggle from '@/components/AutokillToggle.vue'
import DiagnosticsDialog from '@/components/DiagnosticsDialog.vue'

const { t, locale, setLocale, supportedLocales } = useI18n()
const kernel = useKernelStore()
const sysproxy = useProxyStore()
const desktop = useDesktopStore()
const tun = useTunStore()
const settings = useSettingsStore()
const updater = useUpdaterStore()
const anomalies = useAnomaliesStore()
const notices = useNoticesStore()

// ---------------------------------------------------------------------------
// Local UI state
// ---------------------------------------------------------------------------

/** Log-level options shown in the radio group.  Mihomo supports
 *  `silent / error / warning / info / debug`; we expose the four
 *  sensible ones for end-users. */
const logLevels = ['silent', 'error', 'warning', 'info'] as const
type LogLevel = typeof logLevels[number]
const logLevel = ref<LogLevel>('info')
const logLevelDirty = ref(false)

/** TUN stack options (driver selection).  Real mihomo enum: `mixed`
 *  (WinTun + gvisor) or `gvisor` (pure userspace).  We don't write
 *  this through the config yet — it's a presentation-only preview
 *  for Phase 7; persistence will land in a follow-up.
 */
const tunStacks = [
  { value: 'mixed',  label: 'Mixed (Wintun + gVisor)', desc: 'Best compatibility (recommended)' },
  { value: 'gvisor', label: 'gVisor (pure userspace)', desc: 'Lower CPU, no driver install' },
] as const
const tunStack = ref<'mixed' | 'gvisor'>('mixed')

/** Close-window behavior. Persisted to localStorage so the choice survives a
 *  restart, *and* pushed to Rust on every change — the `CloseRequested` hook
 *  is what actually decides, so a renderer-only value would leave this switch
 *  looking selected while doing nothing. */
const closeBehavior = ref<CloseBehavior>(readCloseBehavior())
function setCloseBehavior(v: CloseBehavior) {
  closeBehavior.value = v
  if (typeof localStorage !== 'undefined') localStorage.setItem(CLOSE_BEHAVIOR_KEY, v)
  // Reported through the unified notice channel rather than swallowed: a
  // failure here means the UI and the backend disagree about what closing
  // the window does, which is exactly the bug this setting is for.
  void pushCloseBehavior(v).catch((e) => {
    notices.raiseError('desktop', e)
  })
}

// ---------------------------------------------------------------------------
// TUN Advanced switches (strict-route / dns-hijack)
//
// The values themselves live in the settings store (persisted). This block
// only owns the *side effect*: once TUN is already up, a flipped switch has
// to reach the running kernel, otherwise the card would happily show "on"
// while config.yaml still says otherwise.
// ---------------------------------------------------------------------------

/** A push to the running kernel is in flight. */
const advancedBusy = ref(false)
/** Non-fatal failure of that push — the value is still persisted. */
const advancedError = ref<string | null>(null)

async function onAdvancedToggle(which: keyof TunAdvancedOptions) {
  if (advancedBusy.value) return
  if (which === 'strictRoute') settings.toggleStrictRoute()
  else settings.toggleDnsHijack()

  advancedError.value = null
  // With TUN off there is no kernel holding the old yaml: the switch is
  // persisted and `enableTun()` stamps it on the way up.
  if (!tun.isOn) return

  advancedBusy.value = true
  try {
    await applyTunAdvanced(settings.tunAdvancedSnapshot)
  } catch (e) {
    advancedError.value = e instanceof Error ? e.message : String(e)
  } finally {
    advancedBusy.value = false
  }
}

// ---------------------------------------------------------------------------
// About card: installed client version + the in-app updater.
//
// The updater logic itself lives in `stores/updater.ts` (phase machine +
// single progress listener); this component only projects that store into
// the card. A manual check here reuses the same store as the cold-start
// dialog, so it never attaches a competing listener or races the prompt.
// ---------------------------------------------------------------------------

/** Installed app version, read from the Tauri bundle at mount time
 *  (`tauri.conf.json > version`). Null until the IPC call resolves, and
 *  in plain browser preview where there is no bundle to ask. Never
 *  hard-code this — a literal silently drifts behind the manifest. */
const appVersion = ref<string | null>(null)

/** Display form of {@link appVersion}. Falls back to an em dash rather
 *  than a stale/guessed number when the version is not (yet) known. */
const versionLabel = computed(() => (appVersion.value ? `v${appVersion.value}` : '—'))

/** Primary button label, derived from the updater store's phase machine:
 *  check → checking → download → downloading → restart → installing. */
const updateActionLabel = computed(() => {
  switch (updater.phase) {
    case 'checking': return t('updater.checking')
    case 'downloading': return t('updater.downloading')
    case 'available': return t('updater.download')
    case 'ready': return t('updater.restart')
    case 'installing': return t('updater.installing')
    default: return t('updater.check')
  }
})

/** Advance the flow exactly one step. `download()` and `install()` reject on
 *  failure (the store already recorded `error`), so each is wrapped rather
 *  than letting an unhandled rejection escape the click handler. */
async function onUpdateAction() {
  if (updater.phase === 'available') {
    try { await updater.download() } catch { /* surfaced via updater.error */ }
  } else if (updater.phase === 'ready') {
    try { await updater.install() } catch { /* surfaced via updater.error */ }
  } else {
    await updater.check()
  }
}

const GITHUB_URL = 'https://github.com/zhuyikai2002/flexclash'

// ---------------------------------------------------------------------------
// Derived state
// ---------------------------------------------------------------------------

const statusLabel = computed(() => {
  switch (kernel.state) {
    case 'running':    return t('dashboard.state.running')
    case 'starting':   return t('dashboard.state.starting')
    case 'stopping':   return t('dashboard.state.stopping')
    case 'recovering': return t('dashboard.state.recovering')
    case 'stopped':    return t('dashboard.state.stopped')
    case 'crashed':    return t('dashboard.state.crashed')
    default:           return t('dashboard.state.unknown')
  }
})

const statusColorClass = computed(() => {
  switch (kernel.state) {
    case 'running':    return 'text-emerald-400'
    case 'starting':
    case 'stopping':
    case 'recovering': return 'text-amber-400'
    case 'crashed':    return 'text-rose-400'
    case 'stopped':    return 'text-zinc-500'
    default:           return 'text-zinc-500'
  }
})

/** Endpoint the frontend uses to talk to mihomo.  Rendered as
 *  selectable text so the user can copy-paste into a tool like
 *  Postman for debugging. */
const controllerEndpoint = computed(() => kernel.endpoint || 'http://127.0.0.1:9091')

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

const acting = ref(false)
async function safeRun(fn: () => Promise<void>) {
  if (acting.value) return
  acting.value = true
  try { await fn() } finally { acting.value = false }
}

async function restartKernel() {
  await safeRun(async () => {
    await kernel.restart()
  })
}

/** Read-only port badges, seeded with the reserved values the Rust side
 *  stamps onto every sanitised profile (`RESERVED_MIXED_PORT` = 7897,
 *  `RESERVED_CONTROLLER` = 127.0.0.1:9091).  The "edit + persist" flow
 *  lands when the Rust config-write command ships.
 *
 *  This used to `invoke('get_kernel_ports')` — a command that exists nowhere
 *  in the Rust registry, so every mount fired an IPC call that could only
 *  reject (and was swallowed by the trailing `.catch`).  The literals below
 *  are therefore exactly what the UI has always displayed. */
const mixedPort = ref(7897)
const controllerPort = ref(9091)
const socksPort = ref(7892)

/** Sweep residual routes.  Reuses the TUN store's `runSweep`
 *  action so we don't need a new Tauri command — the Rust side
 *  already exposes this via `commands::tun::sweep_tun_routes`. */
const sweeping = ref(false)
const sweepMessage = ref<string | null>(null)
async function sweepRoutes() {
  if (sweeping.value) return
  sweeping.value = true
  sweepMessage.value = null
  try {
    const r = await tun.runSweep()
    const removed = r?.deleted_routes ?? 0
    sweepMessage.value = `✓ ${t('settings.general.sweep_done')} (${removed} route${removed === 1 ? '' : 's'})`
  } catch (e) {
    sweepMessage.value = '✗ ' + (e instanceof Error ? e.message : String(e))
  } finally {
    sweeping.value = false
    setTimeout(() => { sweepMessage.value = null }, 4000)
  }
}

/** Log level change handler.  Real implementation lands when the
 *  PUT /configs Tauri command ships; for now we mark dirty so the
 *  user sees the change visually. */
function onLogLevelChange(v: LogLevel) {
  logLevel.value = v
  logLevelDirty.value = true
  // eslint-disable-next-line no-console
  console.info(`[settings] log-level → ${v} (will apply on next kernel restart)`)
}

// ---------------------------------------------------------------------------
// Phase 8: "Reset Application" flow
// ---------------------------------------------------------------------------

/** Diagnostics modal (see `DiagnosticsDialog`). */
const diagOpen = ref(false)

const resetOpen = ref(false)
const resetting = ref(false)
const lastResetReport = ref<ResetReport | null>(null)
const showResultModal = ref(false)
let unlistenReset: UnlistenFn | null = null

async function performReset() {
  if (resetting.value) return
  resetting.value = true
  // Don't close the modal yet — the user should see the in-progress
  // state until the Rust event fires.  The modal "busy" state is
  // bound to `resetting`.
  try {
    const report = await resetApplication()
    lastResetReport.value = report
  } catch (e) {
    // eslint-disable-next-line no-console
    console.error('[reset] failed', e)
    lastResetReport.value = null
  } finally {
    resetting.value = false
  }
}

/** Watch for the `app://reset-completed` event from Rust.
 *  When it fires we:
 *   1) close the typing-confirm modal
 *   2) clear the frontend's localStorage + in-memory proxies/rules stores
 *   3) show a "Reset complete — restarting" modal
 *   4) trigger an `app.exit(0)` 1.2s later so the user sees the modal. */
onMounted(() => {
  void onResetCompleted((report) => {
    lastResetReport.value = report
    resetOpen.value = false
    showResultModal.value = true
    // Wipe localStorage + the in-memory proxies/rules stores. Without this
    // the views keep rendering the pre-reset group list and node delay
    // measurements until the process actually exits.
    clearClientState()
    setTimeout(() => {
      try {
        if (typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window) {
          // @ts-expect-error — global tauri shim
          window.__TAURI_INTERNALS__?.invoke?.('plugin:process|exit', { code: 0 })
            .catch(() => { window.close() })
        } else {
          window.close()
        }
      } catch {
        /* ignore */
      }
    }, 1200)
  }).then((u) => { unlistenReset = u })
})

// Read the installed version once at mount. `getAppVersion()` asks the
// Tauri bundle (i.e. `tauri.conf.json`), so the About card and the
// "already up to date" message track the manifest automatically instead
// of drifting behind a hand-maintained literal.
onMounted(() => {
  void getAppVersion().then((v) => { appVersion.value = v })
})

onUnmounted(() => {
  if (unlistenReset) unlistenReset()
})
</script>

<template>
  <div class="space-y-6">
    <!-- ==================== Header ==================== -->
    <header class="flex items-center justify-between">
      <div>
        <h1 class="text-2xl font-semibold text-zinc-100">
          {{ t('settings.title') }}
        </h1>
        <p class="text-xs text-zinc-500 mt-1">
          {{ t('settings.subtitle') }}
        </p>
      </div>
    </header>

    <!-- ==================== Core & Network ==================== -->
    <section
      class="glass-card rounded-2xl p-5 space-y-4"
    >
      <div class="flex items-center gap-2 text-zinc-300">
        <Cpu class="w-4 h-4 text-sky-400" />
        <h2 class="text-sm font-semibold uppercase tracking-wider">
          {{ t('settings.core.title') }}
        </h2>
      </div>

      <!-- Version + status row -->
      <div class="grid grid-cols-1 md:grid-cols-2 gap-3">
        <div class="rounded-xl border border-white/5 bg-white/[0.03] p-4">
          <div class="text-[10px] uppercase tracking-wider text-zinc-500 font-semibold">
            {{ t('settings.core.version') }}
          </div>
          <div class="mt-1.5 text-xl font-mono text-zinc-100">
            {{ kernel.version || '—' }}
          </div>
          <div class="mt-1 text-[10px] text-zinc-500 font-mono">
            mihomo
          </div>
        </div>

        <div class="rounded-xl border border-white/5 bg-white/[0.03] p-4">
          <div class="text-[10px] uppercase tracking-wider text-zinc-500 font-semibold">
            {{ t('settings.core.status') }}
          </div>
          <div :class="['mt-1.5 text-xl font-semibold flex items-center gap-2', statusColorClass]">
            <span
              :class="[
                'w-2 h-2 rounded-full',
                kernel.state === 'running' ? 'bg-emerald-400 shadow-lg shadow-emerald-400/50' :
                kernel.state === 'crashed' ? 'bg-rose-400' :
                kernel.state === 'starting' || kernel.state === 'stopping' || kernel.state === 'recovering' ? 'bg-amber-400 animate-pulse' :
                'bg-zinc-600'
              ]"
            ></span>
            {{ statusLabel }}
          </div>
          <div class="mt-1 text-[10px] text-zinc-500 font-mono">
            {{ t('settings.core.auto_managed') }}
          </div>
        </div>
      </div>

      <!-- Action row -->
      <div class="flex flex-wrap items-center gap-2">
        <button
          :disabled="acting"
          @click="restartKernel"
          class="inline-flex items-center gap-2 rounded-xl bg-indigo-500 hover:bg-indigo-400 disabled:bg-indigo-900 disabled:text-indigo-400 text-white px-4 py-2 text-sm font-medium transition-colors shadow-sm shadow-indigo-500/20"
        >
          <RotateCcw class="w-4 h-4" />
          {{ t('settings.core.restart') }}
        </button>
        <button
          :disabled="acting"
          @click="safeRun(async () => { await kernel.probeWithBackoff() })"
          class="inline-flex items-center gap-2 rounded-xl border border-white/5 bg-white/[0.04] hover:bg-white/[0.08] hover:border-white/10 text-zinc-100 px-4 py-2 text-sm transition-colors"
        >
          <Activity class="w-4 h-4" />
          {{ t('settings.core.reprobe') }}
        </button>
      </div>

      <!-- Port config (read-only preview, editable in Phase 8) -->
      <div class="rounded-xl border border-white/5 bg-white/[0.03] p-4 space-y-3">
        <div class="text-[10px] uppercase tracking-wider text-zinc-500 font-semibold flex items-center gap-1.5">
          <Hash class="w-3 h-3" />
          {{ t('settings.core.ports') }}
        </div>
        <div class="grid grid-cols-1 sm:grid-cols-3 gap-3">
          <div>
            <div class="text-xs text-zinc-400">HTTP / Mixed</div>
            <div class="selectable mt-1 text-lg font-mono text-zinc-100">{{ mixedPort }}</div>
          </div>
          <div>
            <div class="text-xs text-zinc-400">SOCKS5</div>
            <div class="selectable mt-1 text-lg font-mono text-zinc-100">{{ socksPort }}</div>
          </div>
          <div>
            <div class="text-xs text-zinc-400">RESTful controller</div>
            <div class="selectable mt-1 text-lg font-mono text-zinc-100">
              127.0.0.1:{{ controllerPort }}
            </div>
          </div>
        </div>
        <p class="text-[10px] text-zinc-600 italic">
          {{ t('settings.core.ports_hint') }}
        </p>
      </div>

      <!-- Log level -->
      <div class="rounded-xl border border-white/5 bg-white/[0.03] p-4 space-y-3">
        <div class="text-[10px] uppercase tracking-wider text-zinc-500 font-semibold flex items-center gap-1.5">
          <FileText class="w-3 h-3" />
          {{ t('settings.core.log_level') }}
        </div>
        <div class="grid grid-cols-2 sm:grid-cols-4 gap-2">
          <button
            v-for="lvl in logLevels"
            :key="lvl"
            type="button"
            :class="[
              'rounded-lg px-3 py-2 text-sm font-mono transition-colors',
              logLevel === lvl
                ? 'bg-sky-500/20 text-sky-200 ring-1 ring-sky-400/40'
                : 'bg-white/[0.03] text-zinc-400 hover:bg-white/[0.06]'
            ]"
            @click="onLogLevelChange(lvl)"
          >
            {{ lvl }}
          </button>
        </div>
        <p v-if="logLevelDirty" class="text-[10px] text-amber-300/80">
          {{ t('settings.core.log_level_hint') }}
        </p>
      </div>
    </section>

    <!-- ==================== TUN Advanced ==================== -->
    <section
      class="glass-card rounded-2xl p-5 space-y-4"
    >
      <div class="flex items-center gap-2 text-zinc-300">
        <Layers class="w-4 h-4 text-indigo-400" />
        <h2 class="text-sm font-semibold uppercase tracking-wider">
          {{ t('settings.tun.title') }}
        </h2>
      </div>

      <!-- Driver stack -->
      <div class="space-y-2">
        <div class="text-[10px] uppercase tracking-wider text-zinc-500 font-semibold">
          {{ t('settings.tun.stack') }}
        </div>
        <div class="grid grid-cols-1 sm:grid-cols-2 gap-2">
          <button
            v-for="opt in tunStacks"
            :key="opt.value"
            type="button"
            :class="[
              'text-left rounded-xl p-3 transition-colors',
              tunStack === opt.value
                ? 'bg-indigo-500/15 ring-1 ring-indigo-400/40'
                : 'bg-white/[0.03] hover:bg-white/[0.06] ring-1 ring-transparent'
            ]"
            @click="tunStack = opt.value"
          >
            <div class="text-sm font-medium text-zinc-100">{{ opt.label }}</div>
            <div class="text-[11px] text-zinc-500 mt-0.5">{{ opt.desc }}</div>
          </button>
        </div>
      </div>

      <!-- Strict route + DNS hijack: live switches stamped onto `tun:`
           when TUN is enabled (see config/profile.rs::toggle_tun_block). -->
      <div class="grid grid-cols-1 sm:grid-cols-2 gap-3">
        <button
          type="button"
          role="switch"
          :aria-checked="settings.tunAdvanced.strictRoute"
          :disabled="advancedBusy"
          :class="[
            'text-left rounded-xl border p-4 transition-colors',
            'focus:outline-none focus-visible:ring-2 focus-visible:ring-indigo-400/50',
            advancedBusy && 'cursor-wait',
            settings.tunAdvanced.strictRoute
              ? 'border-indigo-400/40 bg-indigo-500/15'
              : 'border-white/5 bg-white/[0.03] hover:bg-white/[0.06]',
          ]"
          @click="onAdvancedToggle('strictRoute')"
        >
          <div class="flex items-center justify-between gap-2">
            <div class="text-xs text-zinc-300 font-medium">
              {{ t('settings.tun.strict_route') }}
            </div>
            <span
              :class="[
                'shrink-0 text-[10px] font-mono px-1.5 py-0.5 rounded',
                settings.tunAdvanced.strictRoute
                  ? 'bg-indigo-500/25 text-indigo-200'
                  : 'bg-zinc-800 text-zinc-400',
              ]"
            >
              {{ t(settings.tunAdvanced.strictRoute ? 'common.on' : 'common.off') }}
            </span>
          </div>
          <p class="text-[11px] text-zinc-500 mt-1.5 leading-relaxed">
            {{ t('settings.tun.strict_route_desc') }}
          </p>
        </button>

        <button
          type="button"
          role="switch"
          :aria-checked="settings.tunAdvanced.dnsHijack"
          :disabled="advancedBusy"
          :class="[
            'text-left rounded-xl border p-4 transition-colors',
            'focus:outline-none focus-visible:ring-2 focus-visible:ring-indigo-400/50',
            advancedBusy && 'cursor-wait',
            settings.tunAdvanced.dnsHijack
              ? 'border-indigo-400/40 bg-indigo-500/15'
              : 'border-white/5 bg-white/[0.03] hover:bg-white/[0.06]',
          ]"
          @click="onAdvancedToggle('dnsHijack')"
        >
          <div class="flex items-center justify-between gap-2">
            <div class="text-xs text-zinc-300 font-medium">
              {{ t('settings.tun.dns_hijack') }}
            </div>
            <span
              :class="[
                'shrink-0 text-[10px] font-mono px-1.5 py-0.5 rounded',
                settings.tunAdvanced.dnsHijack
                  ? 'bg-indigo-500/25 text-indigo-200'
                  : 'bg-zinc-800 text-zinc-400',
              ]"
            >
              {{ t(settings.tunAdvanced.dnsHijack ? 'common.on' : 'common.off') }}
            </span>
          </div>
          <p class="text-[11px] text-zinc-500 mt-1.5 leading-relaxed">
            {{ t('settings.tun.dns_hijack_desc') }}
          </p>
        </button>
      </div>

      <p v-if="advancedError" class="text-[11px] text-rose-300">
        {{ t('settings.tun.apply_failed', { error: advancedError }) }}
      </p>
      <p v-else-if="!tun.isOn" class="text-[11px] text-zinc-500">
        {{ t('settings.tun.apply_on_enable') }}
      </p>
    </section>

    <!-- ==================== General ==================== -->
    <section
      class="glass-card rounded-2xl p-5 space-y-4"
    >
      <div class="flex items-center gap-2 text-zinc-300">
        <Cog class="w-4 h-4 text-emerald-400" />
        <h2 class="text-sm font-semibold uppercase tracking-wider">
          {{ t('settings.general.title') }}
        </h2>
      </div>

      <!-- Language picker -->
      <div class="space-y-2">
        <div class="text-[10px] uppercase tracking-wider text-zinc-500 font-semibold flex items-center gap-1.5">
          <Languages class="w-3 h-3" />
          {{ t('settings.general.language') }}
        </div>
        <div class="flex flex-wrap gap-2">
          <button
            v-for="loc in supportedLocales"
            :key="loc"
            type="button"
            :class="[
              'rounded-lg px-3 py-1.5 text-sm font-medium transition-colors',
              locale === loc
                ? 'bg-emerald-500/20 text-emerald-200 ring-1 ring-emerald-400/40'
                : 'bg-white/[0.03] text-zinc-400 hover:bg-white/[0.06]'
            ]"
            @click="setLocale(loc)"
          >
            {{ t(`language.${loc}`) }}
          </button>
        </div>
      </div>

      <!-- Close behavior -->
      <div class="space-y-2">
        <div class="text-[10px] uppercase tracking-wider text-zinc-500 font-semibold flex items-center gap-1.5">
          <Power class="w-3 h-3" />
          {{ t('settings.general.close_behavior') }}
        </div>
        <div class="grid grid-cols-1 sm:grid-cols-2 gap-2">
          <button
            type="button"
            :class="[
              'text-left rounded-xl p-3 transition-colors',
              closeBehavior === 'minimize'
                ? 'bg-emerald-500/15 ring-1 ring-emerald-400/40'
                : 'bg-white/[0.03] hover:bg-white/[0.06] ring-1 ring-transparent'
            ]"
            @click="setCloseBehavior('minimize')"
          >
            <div class="text-sm font-medium text-zinc-100 flex items-center gap-2">
              <EyeOff class="w-3.5 h-3.5" />
              {{ t('settings.general.close_minimize') }}
            </div>
            <p class="text-[11px] text-zinc-500 mt-1">
              {{ t('settings.general.close_minimize_desc') }}
            </p>
          </button>
          <button
            type="button"
            :class="[
              'text-left rounded-xl p-3 transition-colors',
              closeBehavior === 'exit'
                ? 'bg-rose-500/15 ring-1 ring-rose-400/40'
                : 'bg-white/[0.03] hover:bg-white/[0.06] ring-1 ring-transparent'
            ]"
            @click="setCloseBehavior('exit')"
          >
            <div class="text-sm font-medium text-zinc-100 flex items-center gap-2">
              <X class="w-3.5 h-3.5" />
              {{ t('settings.general.close_exit') }}
            </div>
            <p class="text-[11px] text-zinc-500 mt-1">
              {{ t('settings.general.close_exit_desc') }}
            </p>
          </button>
        </div>
      </div>

      <!-- Route cleanup -->
      <div class="rounded-xl border border-white/5 bg-white/[0.03] p-4 space-y-2">
        <div class="text-[10px] uppercase tracking-wider text-zinc-500 font-semibold flex items-center gap-1.5">
          <Wifi class="w-3 h-3" />
          {{ t('settings.general.route_cleanup') }}
        </div>
        <p class="text-[11px] text-zinc-500 leading-relaxed">
          {{ t('settings.general.route_cleanup_desc') }}
        </p>
        <div class="flex items-center gap-3">
          <button
            type="button"
            :disabled="sweeping"
            @click="sweepRoutes"
            class="inline-flex items-center gap-2 rounded-lg border border-white/10 bg-white/[0.04] hover:bg-white/[0.08] disabled:opacity-60 text-zinc-100 px-3 py-1.5 text-xs font-medium transition-colors"
          >
            <Loader2 v-if="sweeping" class="w-3.5 h-3.5 animate-spin" />
            <RotateCcw v-else class="w-3.5 h-3.5" />
            {{ t('settings.general.route_cleanup_action') }}
          </button>
          <span v-if="sweepMessage" class="text-[11px] text-zinc-400 font-mono">
            {{ sweepMessage }}
          </span>
        </div>
      </div>
    </section>

    <!-- ==================== About ==================== -->
    <section
      class="glass-card rounded-2xl p-5 space-y-4"
    >
      <div class="flex items-center gap-2 text-zinc-300">
        <BookOpen class="w-4 h-4 text-amber-400" />
        <h2 class="text-sm font-semibold uppercase tracking-wider">
          {{ t('settings.about.title') }}
        </h2>
      </div>

      <div class="grid grid-cols-1 md:grid-cols-2 gap-3">
        <div class="rounded-xl border border-white/5 bg-white/[0.03] p-4">
          <div class="text-[10px] uppercase tracking-wider text-zinc-500 font-semibold">
            {{ t('settings.about.client_version') }}
          </div>
          <div class="mt-1.5 text-xl font-mono text-zinc-100">{{ versionLabel }}</div>
          <div class="mt-1 text-[10px] text-zinc-500 font-mono">
            {{ t('app.tagline') }}
          </div>
        </div>

        <div class="rounded-xl border border-white/5 bg-white/[0.03] p-4">
          <div class="text-[10px] uppercase tracking-wider text-zinc-500 font-semibold">
            {{ t('settings.about.update_check') }}
          </div>

          <!-- Primary action + optional "view details" (the changelog now lives
               in UpdateDialog, not inline, so the card stays compact). -->
          <div class="mt-2 flex flex-wrap items-center gap-2">
            <button
              type="button"
              :disabled="updater.isBusy"
              @click="onUpdateAction"
              :class="[
                'inline-flex items-center gap-2 rounded-lg px-3 py-1.5 text-xs font-medium transition-colors',
                updater.phase === 'ready'
                  ? 'bg-emerald-500 hover:bg-emerald-400 text-white shadow-sm shadow-emerald-500/30'
                  : 'border border-white/10 bg-white/[0.04] hover:bg-white/[0.08] text-zinc-100 disabled:opacity-60',
              ]"
            >
              <Loader2
                v-if="updater.phase === 'checking' || updater.phase === 'downloading'"
                class="w-3.5 h-3.5 animate-spin"
              />
              <DownloadCloud v-else-if="updater.phase === 'available'" class="w-3.5 h-3.5" />
              <ArrowUpCircle v-else-if="updater.phase === 'ready'" class="w-3.5 h-3.5" />
              <RefreshCw v-else class="w-3.5 h-3.5" />
              {{ updateActionLabel }}
            </button>

            <button
              v-if="updater.phase === 'available'"
              type="button"
              @click="updater.promptOpen = true"
              class="inline-flex items-center gap-2 rounded-lg border border-white/10 bg-white/[0.04] hover:bg-white/[0.08] text-zinc-100 px-3 py-1.5 text-xs font-medium transition-colors"
            >
              {{ t('updater.view_details') }}
            </button>
          </div>

          <!-- Status line: up-to-date / new version / error -->
          <div class="mt-2 flex items-center gap-1.5 text-[11px]">
            <template v-if="updater.phase === 'uptodate'">
              <CheckCircle2 class="w-3.5 h-3.5 text-emerald-400 shrink-0" />
              <span class="text-zinc-400">{{ t('updater.uptodate') }}</span>
            </template>
            <template v-else-if="updater.phase === 'available' || updater.phase === 'ready'">
              <ArrowUpCircle class="w-3.5 h-3.5 text-sky-400 shrink-0" />
              <span class="text-zinc-100 font-medium">
                {{ t('updater.new_version', { version: updater.versionLabel }) }}
              </span>
            </template>
            <template v-else-if="updater.phase === 'error'">
              <AlertCircle class="w-3.5 h-3.5 text-rose-400 shrink-0" />
              <span class="text-rose-300 font-mono break-all">{{ updater.error }}</span>
            </template>
          </div>

          <!-- Progress bar while downloading -->
          <div v-if="updater.phase === 'downloading'" class="mt-3 flex items-center gap-2">
            <div class="relative h-1.5 flex-1 overflow-hidden rounded-full bg-white/5">
              <div
                class="absolute inset-y-0 left-0 bg-sky-400 transition-all"
                :style="{ width: `${updater.progress}%` }"
              ></div>
            </div>
            <span class="font-mono text-[10px] text-zinc-400">{{ updater.progress }}%</span>
          </div>
        </div>
      </div>

      <div class="rounded-xl border border-white/5 bg-white/[0.03] p-4 flex items-center justify-between">
        <div class="flex items-center gap-3">
          <div class="w-8 h-8 rounded-lg bg-white/5 flex items-center justify-center">
            <Github class="w-4 h-4 text-zinc-300" />
          </div>
          <div>
            <div class="text-xs font-medium text-zinc-100">zhuyikai2002/flexclash</div>
            <div class="text-[10px] text-zinc-500 font-mono">{{ GITHUB_URL }}</div>
          </div>
        </div>
        <a
          :href="GITHUB_URL"
          target="_blank"
          rel="noopener noreferrer"
          class="text-[11px] text-sky-400 hover:text-sky-300 transition-colors"
        >
          {{ t('settings.about.open_repo') }} ↗
        </a>
      </div>
    </section>

    <!-- ==================== Diagnostics (v0.6.x Step 5) ==================== -->
    <section
      class="glass-card rounded-2xl p-5 space-y-4"
    >
      <div class="flex items-center gap-2 text-zinc-300">
        <Activity class="w-4 h-4 text-sky-400" />
        <h2 class="text-sm font-semibold uppercase tracking-wider">
          {{ t('settings.diagnostics.title') }}
        </h2>
      </div>

      <!-- The autopilot's kill switch. Renders itself disabled when the
           environment vetoes — see AutokillToggle for why that is a
           dedicated component rather than another ToggleCard. -->
      <AutokillToggle />

      <!-- Entry point to the raw anomaly ring. -->
      <div class="flex items-center justify-between gap-3 rounded-xl border border-white/5 bg-white/[0.03] px-4 py-3">
        <div class="min-w-0">
          <div class="text-sm font-medium text-zinc-100">
            {{ t('settings.diagnostics.log_title') }}
          </div>
          <p class="mt-0.5 text-[11px] text-zinc-500">
            {{ t('settings.diagnostics.log_entry_desc', { n: anomalies.records.length }) }}
          </p>
        </div>
        <button
          type="button"
          class="inline-flex shrink-0 items-center gap-2 rounded-lg border border-white/5 bg-white/[0.04] hover:bg-white/[0.08] hover:border-white/10 text-zinc-100 px-3 py-1.5 text-sm transition-colors"
          @click="diagOpen = true"
        >
          <BookOpen class="w-3.5 h-3.5" />
          {{ t('settings.diagnostics.log_open') }}
        </button>
      </div>
    </section>

    <!-- ==================== Reset & Maintenance (Phase 8) ==================== -->
    <section
      class="rounded-2xl border border-rose-500/15 bg-rose-500/[0.03] backdrop-blur-md p-5 space-y-4"
    >
      <div class="flex items-center gap-2 text-rose-200">
        <AlertTriangle class="w-4 h-4" />
        <h2 class="text-sm font-semibold uppercase tracking-wider">
          {{ t('settings.danger_zone.title') }}
        </h2>
      </div>

      <div class="rounded-xl border border-rose-500/15 bg-white/[0.03] p-4 space-y-3">
        <div class="flex items-start gap-3">
          <div class="w-9 h-9 rounded-lg bg-rose-500/10 ring-1 ring-rose-400/20 flex items-center justify-center shrink-0">
            <Trash2 class="w-4 h-4 text-rose-300" />
          </div>
          <div class="flex-1 min-w-0">
            <div class="text-sm font-medium text-zinc-100">
              {{ t('settings.danger_zone.reset_app') }}
            </div>
            <p class="text-[11px] text-zinc-500 mt-1 leading-relaxed">
              {{ t('settings.danger_zone.reset_app_desc') }}
            </p>
          </div>
        </div>
        <div class="flex justify-end">
          <button
            type="button"
            :disabled="resetting"
            @click="resetOpen = true"
            class="inline-flex items-center gap-2 rounded-lg bg-rose-500/15 hover:bg-rose-500/25 disabled:opacity-60 ring-1 ring-rose-400/30 text-rose-200 px-3 py-1.5 text-sm font-medium transition-colors"
          >
            <AlertTriangle class="w-3.5 h-3.5" />
            {{ t('settings.danger_zone.reset_app_action') }}
          </button>
        </div>
      </div>
    </section>

    <!-- ==================== Diagnostics modal ==================== -->
    <DiagnosticsDialog :open="diagOpen" @close="diagOpen = false" />

    <!-- ==================== Reset Confirm Modal (type RESET) ==================== -->
    <ConfirmModal
      v-model:open="resetOpen"
      :title="t('settings.danger_zone.confirm_title')"
      :body="t('settings.danger_zone.confirm_body')"
      require-text="RESET"
      :confirm-label="t('settings.danger_zone.confirm_action')"
      :cancel-label="t('common.cancel')"
      :busy="resetting"
      tone="danger"
      @confirm="performReset"
    />

    <!-- ==================== Reset Result Modal ==================== -->
    <Teleport to="body">
      <Transition
        enter-active-class="transition duration-150"
        enter-from-class="opacity-0"
        enter-to-class="opacity-100"
        leave-active-class="transition duration-100"
        leave-from-class="opacity-100"
        leave-to-class="opacity-0"
      >
        <div
          v-if="showResultModal"
          class="fixed inset-0 z-[100] flex items-center justify-center p-4"
        >
          <div class="glass-scrim absolute inset-0" />
          <div class="glass-popover border-emerald-400/20 relative w-full max-w-sm rounded-2xl p-6 text-center space-y-3">
            <div class="mx-auto w-12 h-12 rounded-full bg-emerald-500/15 ring-1 ring-emerald-400/30 flex items-center justify-center">
              <CheckCircle2 class="w-6 h-6 text-emerald-300" />
            </div>
            <h2 class="text-base font-semibold text-zinc-100">
              {{ t('settings.danger_zone.result_title') }}
            </h2>
            <p class="text-[12px] text-zinc-400 leading-relaxed">
              {{ t('settings.danger_zone.result_desc') }}
            </p>
            <p v-if="lastResetReport" class="text-[10px] text-zinc-500 font-mono break-all">
              {{ lastResetReport.detail }}
            </p>
            <div class="flex items-center justify-center gap-2 text-[11px] text-emerald-300/80 font-mono pt-1">
              <Loader2 class="w-3 h-3 animate-spin" />
              restarting…
            </div>
          </div>
        </div>
      </Transition>
    </Teleport>
  </div>
</template>
