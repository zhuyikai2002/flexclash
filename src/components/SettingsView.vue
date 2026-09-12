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
 * every action is wired through the existing Pinia stores so
 * `safeInvoke` / `safeListen` continue to guard the IPC layer.
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
import { safeInvokeOr, safeListen, type UnlistenFn } from '@/utils/tauri-bridge'
import { resetApplication, onResetCompleted, clearClientState, type ResetReport } from '@/services/reset'
import ConfirmModal from '@/components/ConfirmModal.vue'

const { t, locale, setLocale, supportedLocales } = useI18n()
const kernel = useKernelStore()
const sysproxy = useProxyStore()
const desktop = useDesktopStore()
const tun = useTunStore()

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

/** Close-window behavior.  Persisted to localStorage so the choice
 *  survives a restart. */
const CLOSE_BEHAVIOR_KEY = 'flexclash.closeBehavior'
type CloseBehavior = 'minimize' | 'exit'
const closeBehavior = ref<CloseBehavior>(
  (typeof localStorage !== 'undefined' &&
    (localStorage.getItem(CLOSE_BEHAVIOR_KEY) as CloseBehavior | null)) || 'minimize',
)
function setCloseBehavior(v: CloseBehavior) {
  closeBehavior.value = v
  if (typeof localStorage !== 'undefined') localStorage.setItem(CLOSE_BEHAVIOR_KEY, v)
}

// ---------------------------------------------------------------------------
// Updater (Phase updater): real tauri-plugin-updater via typed bindings.
// ---------------------------------------------------------------------------
import { commands, type UpdateInfo } from '@/bindings'

type UpdatePhase =
  | 'idle'
  | 'checking'
  | 'uptodate'
  | 'available'
  | 'downloading'
  | 'installing'
  | 'error'

const updateState = ref<UpdatePhase>('idle')
const updateMessage = ref<string | null>(null)
const updateInfo = ref<UpdateInfo | null>(null)
const dlPercent = ref(0)
const appVersion = '0.1.2' // mirrors package.json (bumped at release time)
let updateUnlisten: UnlistenFn | null = null

/** Attach a live progress listener for `updater://progress`. */
function attachProgress(): Promise<void> {
  return safeListen<{ downloaded?: number; total?: number | null }>(
    'updater://progress',
    (e) => {
      const { downloaded = 0, total } = e.payload
      dlPercent.value =
        total && total > 0 ? Math.min(100, Math.round((downloaded / total) * 100)) : 0
    },
  ).then((u) => { updateUnlisten = u })
}

async function checkForUpdate() {
  updateState.value = 'checking'
  updateMessage.value = null
  updateInfo.value = null
  try {
    const found = await commands.checkUpdate()
    if (found.status === 'error') {
      throw new Error(typeof found.error === 'string' ? found.error : JSON.stringify(found.error))
    }
    const info = found.data
    if (info) {
      updateInfo.value = info
      updateState.value = 'available'
    } else {
      updateState.value = 'uptodate'
      updateMessage.value = `v${appVersion} (latest)`
    }
  } catch (e) {
    updateState.value = 'error'
    updateMessage.value = e instanceof Error ? e.message : String(e)
  }
}

async function doUpdate() {
  if (!updateInfo.value) return
  updateState.value = 'downloading'
  dlPercent.value = 0
  updateMessage.value = null
  await attachProgress()
  try {
    const r = await commands.installUpdate()
    if (r.status === 'error') {
      throw new Error(typeof r.error === 'string' ? r.error : JSON.stringify(r.error))
    }
    // Installer has run; the app relaunches itself. If it returns here,
    // the platform deferred the swap — surface as ready.
    updateState.value = 'installing'
    updateMessage.value = `v${updateInfo.value.version} installed — restarting…`
  } catch (e) {
    updateState.value = 'error'
    updateMessage.value = e instanceof Error ? e.message : String(e)
  }
  if (updateUnlisten) { updateUnlisten(); updateUnlisten = null }
}

const GITHUB_URL = 'https://github.com/zhuyikai2002/flexclash'

// ---------------------------------------------------------------------------
// Derived state
// ---------------------------------------------------------------------------

const statusLabel = computed(() => {
  switch (kernel.state) {
    case 'running':  return t('dashboard.state.running')
    case 'starting': return t('dashboard.state.starting')
    case 'stopping': return t('dashboard.state.stopping')
    case 'stopped':  return t('dashboard.state.stopped')
    case 'crashed':  return t('dashboard.state.crashed')
    default:         return t('dashboard.state.unknown')
  }
})

const statusColorClass = computed(() => {
  switch (kernel.state) {
    case 'running':  return 'text-emerald-400'
    case 'starting':
    case 'stopping': return 'text-amber-400'
    case 'crashed':  return 'text-rose-400'
    case 'stopped':  return 'text-zinc-500'
    default:         return 'text-zinc-500'
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

/** Read the current mixed-port + controller port from the
 *  bundled yaml and surface them as read-only badges.  The real
 *  "edit + persist" flow lands when the Rust config-write Tauri
 *  command ships (Phase 8). */
const mixedPort = ref(7897)
const controllerPort = ref(9091)
const socksPort = ref(7892)

void safeInvokeOr<{
  mixedPort: number
  socksPort: number
  controller: string
} | null>('get_kernel_ports', null).then((res) => {
  if (res) {
    if (typeof res.mixedPort === 'number') mixedPort.value = res.mixedPort
    if (typeof res.socksPort === 'number') socksPort.value = res.socksPort
    if (res.controller) {
      const m = res.controller.match(/:(\d+)$/)
      if (m) controllerPort.value = Number(m[1])
    }
  }
}).catch(() => { /* noop — fall back to bundled defaults */ })

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
      class="rounded-2xl border border-white/5 bg-white/[0.04] backdrop-blur-md p-5 space-y-4"
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
                kernel.state === 'starting' || kernel.state === 'stopping' ? 'bg-amber-400 animate-pulse' :
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
      class="rounded-2xl border border-white/5 bg-white/[0.04] backdrop-blur-md p-5 space-y-4"
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

      <!-- Strict route + DNS (read-only preview) -->
      <div class="grid grid-cols-1 sm:grid-cols-2 gap-3">
        <div class="rounded-xl border border-white/5 bg-white/[0.03] p-4">
          <div class="flex items-center justify-between">
            <div class="text-xs text-zinc-300 font-medium">
              {{ t('settings.tun.strict_route') }}
            </div>
            <span class="text-[10px] font-mono px-1.5 py-0.5 rounded bg-zinc-800 text-zinc-400">
              {{ t('settings.tun.coming_soon') }}
            </span>
          </div>
          <p class="text-[11px] text-zinc-500 mt-1.5 leading-relaxed">
            {{ t('settings.tun.strict_route_desc') }}
          </p>
        </div>
        <div class="rounded-xl border border-white/5 bg-white/[0.03] p-4">
          <div class="flex items-center justify-between">
            <div class="text-xs text-zinc-300 font-medium">
              {{ t('settings.tun.dns_hijack') }}
            </div>
            <span class="text-[10px] font-mono px-1.5 py-0.5 rounded bg-zinc-800 text-zinc-400">
              {{ t('settings.tun.coming_soon') }}
            </span>
          </div>
          <p class="text-[11px] text-zinc-500 mt-1.5 leading-relaxed">
            {{ t('settings.tun.dns_hijack_desc') }}
          </p>
        </div>
      </div>
    </section>

    <!-- ==================== General ==================== -->
    <section
      class="rounded-2xl border border-white/5 bg-white/[0.04] backdrop-blur-md p-5 space-y-4"
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
      class="rounded-2xl border border-white/5 bg-white/[0.04] backdrop-blur-md p-5 space-y-4"
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
          <div class="mt-1.5 text-xl font-mono text-zinc-100">v{{ appVersion }}</div>
          <div class="mt-1 text-[10px] text-zinc-500 font-mono">
            {{ t('app.tagline') }}
          </div>
        </div>

        <div class="rounded-xl border border-white/5 bg-white/[0.03] p-4">
          <div class="text-[10px] uppercase tracking-wider text-zinc-500 font-semibold">
            {{ t('settings.about.update_check') }}
          </div>
          <div class="mt-2 flex items-center gap-2">
            <button
              type="button"
              :disabled="updateState === 'checking'"
              @click="checkForUpdate"
              class="inline-flex items-center gap-2 rounded-lg border border-white/10 bg-white/[0.04] hover:bg-white/[0.08] disabled:opacity-60 text-zinc-100 px-3 py-1.5 text-xs font-medium transition-colors"
            >
              <Loader2 v-if="updateState === 'checking'" class="w-3.5 h-3.5 animate-spin" />
              <CheckCircle2 v-else-if="updateState === 'uptodate'" class="w-3.5 h-3.5 text-emerald-400" />
              <AlertCircle v-else-if="updateState === 'available'" class="w-3.5 h-3.5 text-amber-400" />
              <RefreshCw v-else class="w-3.5 h-3.5" />
              {{ t('settings.about.update_action') }}
            </button>
            <span v-if="updateMessage" class="text-[11px] text-zinc-400 font-mono">
              {{ updateMessage }}
            </span>
          </div>

          <!-- Available release card: version + body + update button -->
          <div
            v-if="updateInfo"
            class="mt-3 rounded-xl border border-white/5 bg-zinc-950/30 p-3 space-y-2"
          >
            <div class="flex items-center gap-2">
              <ArrowUpCircle class="w-4 h-4 text-sky-400 shrink-0" />
              <span class="text-xs font-semibold text-zinc-100">
                {{ t('settings.about.new_version', { v: updateInfo.version }) }}
              </span>
              <button
                type="button"
                :disabled="updateState === 'downloading' || updateState === 'installing'"
                class="ml-auto inline-flex items-center gap-1.5 rounded-lg bg-sky-500 px-3 py-1.5 text-[11px] font-semibold text-white hover:bg-sky-400 disabled:opacity-50 transition-colors"
                @click="doUpdate"
              >
                <Loader2
                  v-if="updateState === 'downloading' || updateState === 'installing'"
                  class="w-3 h-3 animate-spin"
                />
                <DownloadCloud v-else class="w-3 h-3" />
                {{ t('settings.about.update_now') }}
              </button>
            </div>
            <!-- Progress bar while downloading -->
            <div
              v-if="updateState === 'downloading'"
              class="flex items-center gap-2"
            >
              <div class="relative h-1.5 flex-1 overflow-hidden rounded-full bg-white/5">
                <div
                  class="absolute inset-y-0 left-0 bg-sky-400 transition-all"
                  :style="{ width: `${dlPercent}%` }"
                ></div>
              </div>
              <span class="font-mono text-[10px] text-zinc-400">{{ dlPercent }}%</span>
            </div>
            <p
              v-if="updateInfo.body"
              class="max-h-24 overflow-y-auto whitespace-pre-wrap text-[11px] leading-relaxed text-zinc-400 font-mono"
            >
              {{ updateInfo.body }}
            </p>
            <!-- Error detail (check/install failures, incl. network) -->
            <p
              v-if="updateState === 'error'"
              class="text-[11px] leading-relaxed text-rose-300 font-mono"
            >
              {{ updateMessage }}
            </p>
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
          <div class="absolute inset-0 bg-zinc-950/80 backdrop-blur-sm" />
          <div class="relative w-full max-w-sm rounded-2xl border border-emerald-400/20 bg-zinc-900/95 backdrop-blur-xl shadow-2xl shadow-black/50 p-6 text-center space-y-3">
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
