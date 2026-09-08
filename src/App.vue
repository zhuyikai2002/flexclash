<script setup lang="ts">
/**
 * App.vue — FlexClash root component (Phase 8: dashboard IA refactor).
 *
 *   ┌─────┬────────────────────────────────────────────┐
 *   │     │  <main class="app-main flex-1 min-w-0 …">  │
 *   │  S  │     <p-6> scroll-area                      │
 *   │  i  │       tab content                          │
 *   │  d  │                                            │
 *   │  e  │   dashboard =                              │
 *   │  b  │     Row 1: SystemProxy | Tun | AutoStart   │
 *   │  a  │     Row 2: OutboundMode (left)             │
 *   │  r  │             + ActiveProfile (right)        │
 *   │     │     Row 3: NetworkStats                    │
 *   │     │     Row 4: Traffic (only if running)       │
 *   │     │                                            │
 *   └─────┴────────────────────────────────────────────┘
 *
 * The .app-root / .app-aside / .app-main shell is defined in
 * `style.css`.  The sidebar's <aside> wrapper now lives in THIS
 * file (so the layout chrome and the chrome-state live in one
 * place); Sidebar.vue provides just the inner content.
 */
import { onMounted, onUnmounted, ref } from 'vue'
import { safeListen, type UnlistenFn } from '@/utils/tauri-bridge'

import { useKernelStore } from '@/stores/kernel'
import { useProxiesStore } from '@/stores/proxies'
import { useProfilesStore } from '@/stores/profiles'
import { useProxyStore } from '@/stores/proxy'
import { useDesktopStore } from '@/stores/desktop'
import { useConnectionsStore } from '@/stores/connections'
import { useTunStore } from '@/stores/tun'
import { useHistoryStore } from '@/stores/history'
import { useAppStateStore } from '@/stores/appstate'
import type { AppStateSnapshot } from '@/bindings'

import Sidebar from '@/components/Sidebar.vue'
import TrafficCard from '@/components/TrafficCard.vue'
import ProxyGroups from '@/components/ProxyGroups.vue'
import ProfileManager from '@/components/ProfileManager.vue'
import SubscribeDialog from '@/components/SubscribeDialog.vue'
import SystemProxyToggle from '@/components/SystemProxyToggle.vue'
import AutoStartToggle from '@/components/AutoStartToggle.vue'
import TunModeToggle from '@/components/TunModeToggle.vue'
import OutboundModeSwitcher from '@/components/OutboundModeSwitcher.vue'
import ActiveProfileCard from '@/components/ActiveProfileCard.vue'
import NetworkStatsCard from '@/components/NetworkStatsCard.vue'
import ConnectionsView from '@/components/ConnectionsView.vue'
import StatsView from '@/components/StatsView.vue'
import SettingsView from '@/components/SettingsView.vue'

const kernel = useKernelStore()
const proxies = useProxiesStore()
const profiles = useProfilesStore()
const sysproxy = useProxyStore()
const desktop = useDesktopStore()
const conns = useConnectionsStore()
const tun = useTunStore()
const history = useHistoryStore()
const appstate = useAppStateStore()

type TabId = 'dashboard' | 'proxies' | 'connections' | 'profiles' | 'stats' | 'settings'
const tab = ref<TabId>('dashboard')
const dialogOpen = ref(false)

let unlistenTun: UnlistenFn | null = null

function openConnections() {
  tab.value = 'connections'
}

onMounted(async () => {
  await kernel.init()
  void kernel.ensureRunning()

  // Phase R3: single point of truth — Rust pushes a 1s state snapshot;
  // we project it straight into the appstate store.
  const unlistenState = await safeListen<AppStateSnapshot>(
    'app-state://sync',
    (e) => appstate.apply(e.payload),
  )
  appstate.addListener(unlistenState)

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
  appstate.release()
  history.dispose()
  kernel.dispose()
})
</script>

<template>
  <div class="app-root h-screen w-screen">
    <!-- ============== Sidebar (slim left rail, 68px) ============== -->
    <aside
      class="app-aside w-[68px] min-w-[68px] max-w-[68px] h-full flex-shrink-0 z-30"
    >
      <Sidebar
        v-model="tab"
        :kernel-state="kernel.state"
        :conn-count="kernel.isRunning ? conns.totalConnections : 0"
        :profile-count="profiles.profiles.length"
      />
    </aside>

    <!-- ============== Main content (flex-1, scrollable) ============== -->
    <main class="app-main flex-1 min-w-0 h-full overflow-y-auto z-10 p-6 scroll-smooth">
      <div class="mx-auto max-w-7xl space-y-6">
        <!-- ============== Dashboard tab (Phase 8 IA) ============== -->
        <div v-if="tab === 'dashboard'" class="space-y-6">
          <!-- Row 1: three system-control hero cards -->
          <section class="grid grid-cols-1 md:grid-cols-3 gap-3">
            <SystemProxyToggle />
            <TunModeToggle />
            <AutoStartToggle />
          </section>

          <!-- Row 2: outbound mode switcher + active profile card -->
          <section class="grid grid-cols-1 lg:grid-cols-2 gap-3">
            <OutboundModeSwitcher />
            <ActiveProfileCard />
          </section>

          <!-- Row 3: network stats (only meaningful once the kernel is up) -->
          <NetworkStatsCard @open-connections="openConnections" />

          <!-- Row 4: live traffic — only when the kernel is up -->
          <TrafficCard v-if="kernel.isRunning" />
        </div><!-- /Dashboard tab -->

        <!-- ============== Proxies tab (full-width node grid) ============== -->
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

        <div v-else-if="tab === 'settings'" class="space-y-6">
          <SettingsView />
        </div>

        <SubscribeDialog :open="dialogOpen" @close="dialogOpen = false" />

        <footer class="text-center text-xs text-zinc-600 pt-4">
          Frontend ↔ Mihomo direct (no Rust in the data path). Rust only owns sidecar lifecycle.
        </footer>
      </div>
    </main>
  </div>
</template>
