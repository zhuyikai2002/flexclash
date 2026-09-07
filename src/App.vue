<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { useKernelStore } from '@/stores/kernel'
import { useProxiesStore } from '@/stores/proxies'
import { useProfilesStore } from '@/stores/profiles'
import { useProxyStore } from '@/stores/proxy'
import { useDesktopStore } from '@/stores/desktop'
import { useConnectionsStore } from '@/stores/connections'
import { useTunStore } from '@/stores/tun'
import { useHistoryStore } from '@/stores/history'
import TrafficCard from '@/components/TrafficCard.vue'
import ProxyGroups from '@/components/ProxyGroups.vue'
import ProfileManager from '@/components/ProfileManager.vue'
import SubscribeDialog from '@/components/SubscribeDialog.vue'
import SystemProxyToggle from '@/components/SystemProxyToggle.vue'
import AutoStartToggle from '@/components/AutoStartToggle.vue'
import TunModeToggle from '@/components/TunModeToggle.vue'
import ConnectionsView from '@/components/ConnectionsView.vue'
import StatsView from '@/components/StatsView.vue'
import {
  AlertCircle,
  CheckCircle2,
  Loader2,
  Power,
  RefreshCw,
  Terminal,
  LayoutDashboard,
  Layers,
  Activity,
  BarChart3,
} from 'lucide-vue-next'

const kernel = useKernelStore()
const proxies = useProxiesStore()
const profiles = useProfilesStore()
const sysproxy = useProxyStore()
const desktop = useDesktopStore()
const conns = useConnectionsStore()
const tun = useTunStore()
const history = useHistoryStore()
const showLogs = ref(true)
const acting = ref(false)
let unlistenTun: UnlistenFn | null = null

type TabId = 'dashboard' | 'connections' | 'profiles' | 'stats'
const tab = ref<TabId>('dashboard')
const dialogOpen = ref(false)

onMounted(async () => {
  await kernel.init()
  // Proxies are fetched on demand by ProxyGroups onMounted, but if the
  // kernel was already running before the UI mounted, kick a refresh so
  // groups appear instantly (M3: avoid a flash of "No selector groups").
  if (kernel.isRunning && !proxies.lastFetchAt) {
    await proxies.fetchProxies()
  }
  // Profiles are passive — subscribe to the change event in case the Rust
  // side already wrote a profile before the UI mounted.
  profiles.attach()
  void profiles.refresh()
  // System-proxy switch (M5): read the current registry value once so the
  // dashboard shows the real state at boot, not a stale default.
  void sysproxy.init()
  // Autostart toggle (M7): same idea — read HKCU at boot, no stale UI.
  void desktop.init()
  // TUN toggle (M9): read the authoritative backend state and
  // subscribe to the tun://state-changed event so the UI reflects
  // elevation success / UAC cancel / sweep results in real time.
  await tun.init()
  unlistenTun = await listen('tun://state-changed', (e) => {
    tun.onStateChanged(e.payload as Parameters<typeof tun.onStateChanged>[0])
  })
  // M10: start the traffic history polling. The Rust side is already
  // writing samples; this just keeps the chart in sync.
  void history.init()
})

onUnmounted(() => {
  if (unlistenTun) unlistenTun()
  history.dispose()
})

async function safeRun(fn: () => Promise<void>) {
  if (acting.value) return
  acting.value = true
  try {
    await fn()
  } finally {
    acting.value = false
  }
}

const stateDotClass = computed(() => {
  switch (kernel.state) {
    case 'running': return 'bg-emerald-500'
    case 'stopped': return 'bg-zinc-500'
    case 'starting':
    case 'stopping': return 'bg-amber-500 animate-pulse'
    case 'crashed': return 'bg-rose-500'
    default: return 'bg-zinc-700'
  }
})

const probeLabel = computed(() => {
  switch (kernel.probeStatus) {
    case 'alive': return `${kernel.probeLatencyMs ?? '?'} ms`
    case 'unreachable': return 'unreachable'
    case 'probing': return 'probing…'
    default: return '—'
  }
})

const probeColorClass = computed(() => {
  switch (kernel.probeStatus) {
    case 'alive': return 'text-emerald-400'
    case 'unreachable': return 'text-rose-400'
    case 'probing': return 'text-amber-400'
    default: return 'text-zinc-500'
  }
})
</script>

<template>
  <main class="min-h-screen bg-zinc-950 text-zinc-100 p-6 md:p-10 font-sans">
    <div class="max-w-3xl mx-auto space-y-6">
      <!-- ============== Header ============== -->
      <header class="flex items-center justify-between">
        <div class="flex items-center gap-3">
          <div class="w-10 h-10 rounded-lg bg-gradient-to-br from-indigo-500 to-purple-600 flex items-center justify-center font-bold text-white shadow">
            F
          </div>
          <div>
            <h1 class="text-xl font-semibold tracking-tight">FlexClash</h1>
            <p class="text-xs text-zinc-500">Mihomo (Clash Meta) frontend · v0.1.0</p>
          </div>
        </div>
        <div class="flex items-center gap-2">
          <span :class="['w-2 h-2 rounded-full', stateDotClass]"></span>
          <span class="font-mono uppercase tracking-wider text-xs text-zinc-300">
            {{ kernel.state }}
          </span>
        </div>
      </header>

      <!-- ============== Config refresh notice ============== -->
      <div
        v-if="kernel.configRefresh"
        class="bg-amber-950/40 border border-amber-800/60 text-amber-200 rounded-lg p-3 text-sm flex items-start gap-2.5"
      >
        <AlertCircle class="w-4 h-4 mt-0.5 shrink-0" />
        <div>
          Default config rewritten —
          <code class="font-mono bg-black/30 px-1.5 py-0.5 rounded">
            external-controller
          </code>
          port auto-refreshed from
          <code class="font-mono bg-black/30 px-1.5 py-0.5 rounded">
            {{ kernel.configRefresh.fromPort ?? '?' }}
          </code>
          to
          <code class="font-mono bg-black/30 px-1.5 py-0.5 rounded">
            {{ kernel.configRefresh.toPort }}
          </code>
          .
        </div>
      </div>

      <!-- ============== Tab nav (M4) ============== -->
      <nav class="flex items-center gap-1 border-b border-slate-800">
        <button
          class="inline-flex items-center gap-1.5 border-b-2 px-3 py-2 text-sm transition"
          :class="tab === 'dashboard'
            ? 'border-sky-500 text-sky-300'
            : 'border-transparent text-slate-400 hover:text-slate-200'"
          @click="tab = 'dashboard'"
        >
          <LayoutDashboard class="h-4 w-4" />
          Dashboard
        </button>
        <button
          class="inline-flex items-center gap-1.5 border-b-2 px-3 py-2 text-sm transition"
          :class="tab === 'connections'
            ? 'border-sky-500 text-sky-300'
            : 'border-transparent text-slate-400 hover:text-slate-200'"
          @click="tab = 'connections'"
        >
          <Activity class="h-4 w-4" />
          Connections
          <span
            v-if="kernel.isRunning && conns.totalConnections"
            class="ml-1 rounded bg-slate-700/60 px-1.5 py-0.5 text-[10px] text-slate-200"
          >{{ conns.totalConnections }}</span>
        </button>
        <button
          class="inline-flex items-center gap-1.5 border-b-2 px-3 py-2 text-sm transition"
          :class="tab === 'profiles'
            ? 'border-sky-500 text-sky-300'
            : 'border-transparent text-slate-400 hover:text-slate-200'"
          @click="tab = 'profiles'"
        >
          <Layers class="h-4 w-4" />
          Profiles
          <span
            v-if="profiles.profiles.length"
            class="ml-1 rounded bg-slate-700/60 px-1.5 py-0.5 text-[10px] text-slate-200"
          >{{ profiles.profiles.length }}</span>
        </button>
        <button
          class="inline-flex items-center gap-1.5 border-b-2 px-3 py-2 text-sm transition"
          :class="tab === 'stats'
            ? 'border-sky-500 text-sky-300'
            : 'border-transparent text-slate-400 hover:text-slate-200'"
          @click="tab = 'stats'"
        >
          <BarChart3 class="h-4 w-4" />
          Rules &amp; stats
        </button>
      </nav>

      <!-- ============== Dashboard tab ============== -->
      <div v-if="tab === 'dashboard'" class="space-y-6">
      <div class="grid grid-cols-1 md:grid-cols-3 gap-4">
        <div class="bg-zinc-900 border border-zinc-800 rounded-xl p-5">
          <div class="text-[10px] text-zinc-500 uppercase tracking-widest font-semibold">
            Mihomo core
          </div>
          <div class="mt-2 text-2xl font-semibold font-mono">
            <span v-if="kernel.version">{{ kernel.version }}</span>
            <span v-else class="text-zinc-600">—</span>
          </div>
        </div>

        <div class="bg-zinc-900 border border-zinc-800 rounded-xl p-5">
          <div class="text-[10px] text-zinc-500 uppercase tracking-widest font-semibold">
            Control endpoint
          </div>
          <div class="mt-2 text-sm font-mono break-all text-zinc-200">
            {{ kernel.endpoint }}
          </div>
        </div>

        <div class="bg-zinc-900 border border-zinc-800 rounded-xl p-5">
          <div class="text-[10px] text-zinc-500 uppercase tracking-widest font-semibold">
            API probe
          </div>
          <div :class="['mt-2 text-2xl font-semibold font-mono flex items-center gap-2', probeColorClass]">
            <CheckCircle2 v-if="kernel.probeStatus === 'alive'" class="w-6 h-6" />
            <Loader2 v-else-if="kernel.probeStatus === 'probing'" class="w-6 h-6 animate-spin" />
            <AlertCircle v-else-if="kernel.probeStatus === 'unreachable'" class="w-6 h-6" />
            <span>{{ probeLabel }}</span>
          </div>
        </div>
      </div>

      <!-- ============== Actions ============== -->
      <div class="flex flex-wrap items-center gap-2">
        <button
          v-if="!kernel.isRunning"
          :disabled="acting || kernel.isTransitioning"
          @click="safeRun(() => kernel.start())"
          class="inline-flex items-center gap-2 bg-emerald-600 hover:bg-emerald-500 disabled:bg-emerald-900 disabled:text-emerald-400 text-white px-4 py-2 rounded-lg text-sm font-medium transition-colors"
        >
          <Power class="w-4 h-4" />
          Start kernel
        </button>
        <button
          v-else
          :disabled="acting || kernel.isTransitioning"
          @click="safeRun(() => kernel.restart())"
          class="inline-flex items-center gap-2 bg-indigo-600 hover:bg-indigo-500 disabled:bg-indigo-900 disabled:text-indigo-400 text-white px-4 py-2 rounded-lg text-sm font-medium transition-colors"
        >
          <RefreshCw class="w-4 h-4" />
          Restart kernel
        </button>
        <button
          :disabled="acting"
          @click="safeRun(() => kernel.probe())"
          class="inline-flex items-center gap-2 bg-zinc-800 hover:bg-zinc-700 disabled:opacity-50 text-zinc-100 px-4 py-2 rounded-lg text-sm transition-colors"
        >
          <RefreshCw class="w-4 h-4" />
          Re-probe
        </button>
      </div>

      <!-- ============== M5: System proxy takeover ============== -->
      <SystemProxyToggle />

      <!-- ============== M7: Autostart toggle ============== -->
      <AutoStartToggle />

      <!-- ============== M9: TUN mode toggle ============== -->
      <TunModeToggle />

      <!-- ============== M3: Real-time traffic ============== -->
      <TrafficCard v-if="kernel.isRunning" />

      <!-- ============== M3: Proxy groups ============== -->
      <ProxyGroups v-if="kernel.isRunning" />

      <!-- ============== Logs panel (M2) ============== -->
      <div class="bg-zinc-900 border border-zinc-800 rounded-xl overflow-hidden">
        <button
          @click="showLogs = !showLogs"
          class="w-full flex items-center justify-between px-5 py-3 text-sm text-zinc-300 hover:bg-zinc-800/50 transition-colors"
        >
          <div class="flex items-center gap-2">
            <Terminal class="w-4 h-4" />
            <span class="font-medium">Kernel logs</span>
            <span class="text-xs text-zinc-500">({{ kernel.recentLogs.length }})</span>
          </div>
          <span class="text-zinc-500">{{ showLogs ? '−' : '+' }}</span>
        </button>
        <div v-if="showLogs" class="border-t border-zinc-800 bg-black/30">
          <div class="font-mono text-xs p-4 max-h-72 overflow-auto space-y-0.5">
            <div
              v-for="(line, i) in kernel.recentLogs"
              :key="i"
              class="whitespace-pre-wrap break-all text-zinc-400"
            >{{ line }}</div>
            <div v-if="!kernel.recentLogs.length" class="text-zinc-600 italic">
              (no logs yet — start the kernel)
            </div>
          </div>
        </div>
      </div>

      <!-- ============== Error footer ============== -->
      <div
        v-if="kernel.lastError"
        class="bg-rose-950/40 border border-rose-800/60 text-rose-200 rounded-lg p-3 text-sm font-mono"
      >
        {{ kernel.lastError }}
      </div>
      </div><!-- /Dashboard tab -->

      <!-- ============== Profiles tab (M4) ============== -->
      <div v-else-if="tab === 'profiles'" class="space-y-6">
        <ProfileManager @open-subscribe="dialogOpen = true" />
      </div>

      <!-- ============== Connections tab (M8) ============== -->
      <div v-else-if="tab === 'connections'" class="space-y-6">
        <ConnectionsView :active="tab === 'connections'" />
      </div>

      <!-- ============== Rules & stats tab (M10) ============== -->
      <div v-else-if="tab === 'stats'" class="space-y-6">
        <StatsView />
      </div>

      <SubscribeDialog :open="dialogOpen" @close="dialogOpen = false" />

      <footer class="text-center text-xs text-zinc-600 pt-4">
        Frontend ↔ Mihomo direct (no Rust in the data path). Rust only owns sidecar lifecycle.
      </footer>
    </div>
  </main>
</template>
