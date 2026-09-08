<script setup lang="ts">
/**
 * App.vue — FlexClash root component.
 *
 *   ┌─────┬────────────────────────────────────────────┐
 *   │     │                                            │
 *   │  S  │  <main>  <p-6>  scroll-area                │
 *   │  i  │     tab content                            │
 *   │  d  │                                            │
 *   │  e  │                                            │
 *   │  b  │                                            │
 *   │  a  │                                            │
 *   │  r  │                                            │
 *   │     │                                            │
 *   └─────┴────────────────────────────────────────────┘
 *
 * Phase 4 layout:
 *   - Left rail (Sidebar.vue) at fixed 68px
 *   - Right main content area, flex-1, scrollable, p-6
 *   - Tabs lifted to root so Sidebar can show live counts
 *   - Brand header sits at the TOP of each tab (e.g. dashboard hero)
 *     so context is local to the page, not a global Navbar
 */
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { safeListen, type UnlistenFn } from '@/utils/tauri-bridge'
import { Power, RefreshCw, Terminal, AlertCircle, CheckCircle2, Loader2 } from 'lucide-vue-next'

import { useKernelStore } from '@/stores/kernel'
import { useProxiesStore } from '@/stores/proxies'
import { useProfilesStore } from '@/stores/profiles'
import { useProxyStore } from '@/stores/proxy'
import { useDesktopStore } from '@/stores/desktop'
import { useConnectionsStore } from '@/stores/connections'
import { useTunStore } from '@/stores/tun'
import { useHistoryStore } from '@/stores/history'
import { useI18n } from '@/composables/useI18n'

import Sidebar from '@/components/Sidebar.vue'
import TrafficCard from '@/components/TrafficCard.vue'
import ProxyGroups from '@/components/ProxyGroups.vue'
import ProfileManager from '@/components/ProfileManager.vue'
import SubscribeDialog from '@/components/SubscribeDialog.vue'
import SystemProxyToggle from '@/components/SystemProxyToggle.vue'
import AutoStartToggle from '@/components/AutoStartToggle.vue'
import TunModeToggle from '@/components/TunModeToggle.vue'
import ConnectionsView from '@/components/ConnectionsView.vue'
import StatsView from '@/components/StatsView.vue'

const kernel = useKernelStore()
const proxies = useProxiesStore()
const profiles = useProfilesStore()
const sysproxy = useProxyStore()
const desktop = useDesktopStore()
const conns = useConnectionsStore()
const tun = useTunStore()
const history = useHistoryStore()
const { t } = useI18n()

type TabId = 'dashboard' | 'proxies' | 'connections' | 'profiles' | 'stats'
const tab = ref<TabId>('dashboard')
const dialogOpen = ref(false)
const showLogs = ref(false)
const acting = ref(false)

let unlistenTun: UnlistenFn | null = null

onMounted(async () => {
  // Subsystems init in parallel.  `kernel.init()` is awaited because the
  // backoff probe can affect whether the dashboard shows "unreachable"
  // or "ready" on cold start.
  await kernel.init()
  if (kernel.isRunning && !proxies.lastFetchAt) {
    void proxies.fetchProxies()
  }
  profiles.attach()
  void profiles.refresh()
  void sysproxy.init()
  void desktop.init()
  await tun.init()
  unlistenTun = await safeListen('tun://state-changed', (e) => {
    tun.onStateChanged(e.payload as Parameters<typeof tun.onStateChanged>[0])
  })
  void history.init()
})

onUnmounted(() => {
  if (unlistenTun) unlistenTun()
  history.dispose()
})

async function safeRun(fn: () => Promise<void>) {
  if (acting.value) return
  acting.value = true
  try { await fn() } finally { acting.value = false }
}

const probeLabel = computed(() => {
  switch (kernel.probeStatus) {
    case 'healthy':    return t('dashboard.probe.alive', { ms: kernel.probeLatencyMs ?? '?' })
    case 'error':      return t('dashboard.probe.unreachable')
    case 'probing':    return t('dashboard.probe.probing')
    default:           return t('dashboard.probe.idle')
  }
})
const probeColorClass = computed(() => {
  switch (kernel.probeStatus) {
    case 'healthy':    return 'text-emerald-400'
    case 'error':      return 'text-rose-400'
    case 'probing':    return 'text-amber-400'
    default:           return 'text-zinc-500'
  }
})
</script>

<template>
  <div class="flex h-full w-full">
    <!-- ============== Sidebar (fixed left rail) ============== -->
    <Sidebar
      v-model="tab"
      :kernel-state="kernel.state"
      :conn-count="kernel.isRunning ? conns.totalConnections : 0"
      :profile-count="profiles.profiles.length"
    />

    <!-- ============== Main content area (flex-1, scrollable) ============== -->
    <main class="scroll-area flex-1 overflow-y-auto">
      <div class="mx-auto max-w-7xl p-6 space-y-6">
        <!-- ============== Config refresh notice ============== -->
        <div
          v-if="kernel.configRefresh"
          class="rounded-2xl border border-amber-500/20 bg-amber-500/10 p-3 text-sm text-amber-200 flex items-start gap-2.5"
        >
          <AlertCircle class="w-4 h-4 mt-0.5 shrink-0" />
          <span v-html="t('dashboard.notice.config_refreshed', {
            from: `<code class='font-mono px-1.5 py-0.5 rounded bg-black/30'>${kernel.configRefresh.fromPort ?? '?'}</code>`,
            to: `<code class='font-mono px-1.5 py-0.5 rounded bg-black/30'>${kernel.configRefresh.toPort}</code>`,
          })" />
        </div>

        <!-- ============== Dashboard tab ============== -->
        <div v-if="tab === 'dashboard'" class="space-y-6">
          <!-- Hero: kernel status + start/stop + probe -->
          <section
            class="rounded-2xl border border-white/5 bg-white/[0.04] p-6 backdrop-blur-md"
          >
            <div class="grid grid-cols-1 lg:grid-cols-3 gap-5">
              <!-- Version -->
              <div>
                <div class="text-[10px] uppercase tracking-wider text-zinc-400 font-semibold">
                  {{ t('dashboard.core.title') }}
                </div>
                <div class="mt-2 text-3xl font-semibold font-mono text-zinc-100 leading-none">
                  <span v-if="kernel.version">{{ kernel.version }}</span>
                  <span v-else class="text-zinc-600">—</span>
                </div>
                <div class="mt-2 text-[10px] text-zinc-500 font-mono">
                  {{ t('dashboard.core.version_label') }}: {{ kernel.version || t('common.unknown') }}
                </div>
              </div>

              <!-- Endpoint -->
              <div>
                <div class="text-[10px] uppercase tracking-wider text-zinc-400 font-semibold">
                  {{ t('dashboard.endpoint.title') }}
                </div>
                <div class="mt-2 text-sm font-mono break-all text-zinc-200 leading-tight">
                  {{ kernel.endpoint }}
                </div>
                <div class="mt-2 text-[10px] text-zinc-500 font-mono">
                  {{ t('dashboard.endpoint.label') }}
                </div>
              </div>

              <!-- Probe -->
              <div>
                <div class="text-[10px] uppercase tracking-wider text-zinc-400 font-semibold">
                  {{ t('dashboard.probe.title') }}
                </div>
                <div :class="['mt-2 text-3xl font-semibold font-mono flex items-center gap-2 leading-none', probeColorClass]">
                  <CheckCircle2 v-if="kernel.probeStatus === 'healthy'" class="w-6 h-6" />
                  <Loader2 v-else-if="kernel.probeStatus === 'probing'" class="w-6 h-6 animate-spin" />
                  <AlertCircle v-else-if="kernel.probeStatus === 'error'" class="w-6 h-6" />
                  <span>{{ probeLabel }}</span>
                </div>
              </div>
            </div>

            <!-- Actions -->
            <div class="mt-6 flex flex-wrap items-center gap-2">
              <button
                v-if="!kernel.isRunning"
                :disabled="acting || kernel.isTransitioning"
                @click="safeRun(() => kernel.start())"
                class="inline-flex items-center gap-2 rounded-xl bg-emerald-500 hover:bg-emerald-400 disabled:bg-emerald-900 disabled:text-emerald-400 text-white px-4 py-2 text-sm font-medium transition-colors shadow-sm shadow-emerald-500/20"
              >
                <Power class="w-4 h-4" />
                {{ t('dashboard.actions.start') }}
              </button>
              <template v-else>
                <button
                  :disabled="acting || kernel.isTransitioning"
                  @click="safeRun(() => kernel.restart())"
                  class="inline-flex items-center gap-2 rounded-xl bg-indigo-500 hover:bg-indigo-400 disabled:bg-indigo-900 disabled:text-indigo-400 text-white px-4 py-2 text-sm font-medium transition-colors shadow-sm shadow-indigo-500/20"
                >
                  <RefreshCw class="w-4 h-4" />
                  {{ t('dashboard.actions.restart') }}
                </button>
                <button
                  :disabled="acting || kernel.isTransitioning"
                  @click="safeRun(() => kernel.stop())"
                  class="inline-flex items-center gap-2 rounded-xl bg-rose-500 hover:bg-rose-400 disabled:bg-rose-900 disabled:text-rose-400 text-white px-4 py-2 text-sm font-medium transition-colors shadow-sm shadow-rose-500/20"
                >
                  <Power class="w-4 h-4" />
                  {{ t('dashboard.actions.stop') }}
                </button>
              </template>
              <button
                :disabled="acting"
                @click="safeRun(() => kernel.probeWithBackoff())"
                class="inline-flex items-center gap-2 rounded-xl border border-white/5 bg-white/[0.04] hover:bg-white/[0.08] hover:border-white/10 text-zinc-100 px-4 py-2 text-sm transition-colors"
              >
                <RefreshCw class="w-4 h-4" />
                {{ t('common.refresh') }}
              </button>
            </div>
          </section>

          <!-- Toggle grid: system proxy / TUN / autostart -->
          <section class="grid grid-cols-1 md:grid-cols-3 gap-3">
            <SystemProxyToggle />
            <TunModeToggle />
            <AutoStartToggle />
          </section>

          <!-- Live traffic hero -->
          <TrafficCard v-if="kernel.isRunning" />

          <!-- Proxy groups (top of dashboard) -->
          <ProxyGroups v-if="kernel.isRunning" />

          <!-- Logs (collapsed by default) -->
          <section class="overflow-hidden rounded-2xl border border-white/5 bg-white/[0.04] backdrop-blur-md">
            <button
              @click="showLogs = !showLogs"
              class="w-full flex items-center justify-between px-5 py-3 text-sm text-zinc-300 hover:bg-white/[0.04] transition-colors"
            >
              <div class="flex items-center gap-2">
                <Terminal class="w-4 h-4" />
                <span class="font-medium">Kernel logs</span>
                <span class="text-xs text-zinc-500 font-mono">({{ kernel.recentLogs.length }})</span>
              </div>
              <span class="text-zinc-500">{{ showLogs ? '−' : '+' }}</span>
            </button>
            <div v-if="showLogs" class="border-t border-white/5 bg-zinc-950/40">
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
          </section>

          <div
            v-if="kernel.lastError"
            class="rounded-lg border border-rose-500/20 bg-rose-500/10 p-3 text-sm text-rose-300 font-mono"
          >
            {{ kernel.lastError }}
          </div>
        </div><!-- /Dashboard tab -->

        <!-- ============== Proxies tab ============== -->
        <div v-else-if="tab === 'proxies'" class="space-y-6">
          <ProxyGroups v-if="kernel.isRunning" />
          <div
            v-else
            class="rounded-2xl border border-dashed border-white/10 bg-white/[0.02] p-12 text-center"
          >
            <div class="text-zinc-400 text-sm">Start the kernel to see proxy groups.</div>
          </div>
        </div>

        <div v-else-if="tab === 'profiles'" class="space-y-6">
          <ProfileManager @open-subscribe="dialogOpen = true" />
        </div>

        <div v-else-if="tab === 'connections'" class="space-y-6">
          <ConnectionsView :active="tab === 'connections'" />
        </div>

        <div v-else-if="tab === 'stats'" class="space-y-6">
          <StatsView />
        </div>

        <SubscribeDialog :open="dialogOpen" @close="dialogOpen = false" />

        <footer class="text-center text-xs text-zinc-600 pt-4">
          Frontend ↔ Mihomo direct (no Rust in the data path). Rust only owns sidecar lifecycle.
        </footer>
      </div>
    </main>
  </div>
</template>
