<script setup lang="ts">
/**
 * ConnectionsView.vue — M8 main panel: toolbar + virtual list.
 * (Style refactor only; virtualization math untouched.)
 */
import { computed, ref, watch, type Component } from 'vue'
import { useVirtualizer } from '@tanstack/vue-virtual'
import {
  AlertTriangle, Filter, Globe, Loader2, Pause, Play, Plug, RefreshCw, Search, Shield, Trash2, X, XCircle, Zap,
} from 'lucide-vue-next'

import { useConnectionsStore, type PollIntervalMs } from '@/stores/connections'
import { useProxiesStore } from '@/stores/proxies'
import { useKernelStore } from '@/stores/kernel'
import { useToastStore } from '@/stores/toast'
import { useKernelDataPump } from '@/composables/useKernelDataPump'
import ConnectionRow from '@/components/ConnectionRow.vue'
import { formatRate } from '@/utils/format'
import { useI18n } from '@/composables/useI18n'
import type { ConnectionRow as ConnectionRowType } from '@/types/clash'
import type { KillReport } from '@/bindings'

const props = defineProps<{ active: boolean }>()

const store = useConnectionsStore()
const proxies = useProxiesStore()
const kernel = useKernelStore()
const toast = useToastStore()
const { t } = useI18n()

const keyword = ref('')
watch(keyword, (v) => store.setSearchKeyword(v))

const policyFilter = ref('')
watch(policyFilter, (v) => store.setPolicyFilter(v))

const intervalChoice = ref<PollIntervalMs>(store.pollIntervalMs)
watch(intervalChoice, (v) => store.setPollInterval(v))

const intervalOptions: { label: string; value: PollIntervalMs }[] = [
  { label: '1s', value: 1000 },
  { label: '2s', value: 2000 },
  { label: '5s', value: 5000 },
]

const confirmOpen = ref(false)
const closing = ref(false)
function askCloseAll() { confirmOpen.value = true }
async function doCloseAll() {
  if (closing.value) return
  closing.value = true
  try { await store.closeAll() } catch (e) { console.error('[connections] closeAll failed', e) } finally {
    closing.value = false
    confirmOpen.value = false
  }
}

async function manualRefresh() { await store.forceRefresh() }
function togglePause() { store.setPaused(!store.isPaused) }

useKernelDataPump({
  tasks: [
    {
      name: 'connections',
      intervalMs: () => store.pollIntervalMs,
      enabled: () => props.active && !store.isPaused,
      fetch: () => store.refresh(),
    },
  ],
  // Kernel vanished: what is on screen is old, not empty. Mark both stores
  // and let the first tick after recovery replace it.
  onKernelDown: () => {
    store.markStale()
    proxies.markStale()
  },
  onKernelUp: () => store.forceRefresh(),
})

const scrollEl = ref<HTMLElement | null>(null)
const filteredRows = computed(() => store.filteredRows)
const total = computed(() => filteredRows.value.length)

const virtualizer = useVirtualizer({
  get count() { return total.value },
  getScrollElement: () => scrollEl.value,
  estimateSize: () => 40,
  overscan: 8,
})

const virtualRows = computed(() => virtualizer.value.getVirtualItems())
const totalSize = computed(() => virtualizer.value.getTotalSize())

const totalUpText = computed(() => formatRate(
  filteredRows.value.reduce((s, r) => s + r.uploadSpeed, 0),
))
const totalDownText = computed(() => formatRate(
  filteredRows.value.reduce((s, r) => s + r.downloadSpeed, 0),
))

function onCloseRow(id: string) {
  store.closeOne(id).catch((e) => { console.error('[connections] closeOne failed', e) })
}

// ---------------------------------------------------------------------------
// Right-click context menu (kill current / same-host / same-rule). A single
// Teleported menu (position: fixed) avoids clipping inside the virtual list's
// scroll box. It also supports full keyboard navigation (↑/↓/Enter/Esc) and an
// Alt "batch-kill" mode (⚡ 断开该节点下所有死链).
// ---------------------------------------------------------------------------
interface RowMenu { x: number; y: number; row: ConnectionRowType }
const rowMenu = ref<RowMenu | null>(null)
const menuBusy = ref(false)

/** Alt held when the menu opened (or live-held while open) → batch mode. */
const altMode = ref(false)
/** Keyboard-highlighted item index (↑/↓). */
const activeIndex = ref(0)

/** Resolve the egress node name from a connection's proxy chain. */
function nodeOf(row: ConnectionRowType): string | null {
  const c = row.chains
  return c && c.length ? c[c.length - 1] : null
}

interface MenuItem {
  key: string
  label: string
  icon: 'x' | 'globe' | 'shield' | 'zap'
  disabled?: boolean
  action: () => void
}
const ICONS: Record<string, Component> = { x: X, globe: Globe, shield: Shield, zap: Zap }

function onRowContextMenu(ev: MouseEvent, row: ConnectionRowType) {
  altMode.value = ev.altKey
  rowMenu.value = { x: ev.clientX, y: ev.clientY, row }
}

function closeRowMenu() { rowMenu.value = null }

/** The menu's visible items, reshaped when Alt is held. */
const menuItems = computed<MenuItem[]>(() => {
  const rm = rowMenu.value
  if (!rm) return []
  const r = rm.row
  if (altMode.value) {
    const node = nodeOf(r)
    if (node) {
      // Batch-kill panel: ⚡ 断开该节点下所有死链 (force-kill entry point).
      return [{
        key: 'batch-node',
        label: t('connections.kill_by_proxy'),
        icon: 'zap',
        action: () => runBatchKill(node),
      }]
    }
    // No resolvable node → fall through to the normal list so it's never empty.
  }
  return [
    { key: 'current', label: t('connections.kill_current'), icon: 'x', action: () => void killCurrent(r) },
    { key: 'host', label: t('connections.kill_same_host'), icon: 'globe', disabled: !r.host, action: () => void killByHost(r) },
    { key: 'rule', label: t('connections.kill_same_rule'), icon: 'shield', disabled: !r.rule, action: () => void killByRule(r) },
  ]
})

function notifyKill(r: KillReport) {
  if (r.killed > 0) toast.push('success', t('connections.killed_n', { n: r.killed }))
  else toast.push('info', t('connections.kill_none'))
}

async function killCurrent(row: ConnectionRowType) {
  closeRowMenu()
  try {
    await store.closeOne(row.id)
    toast.push('success', t('connections.killed_n', { n: 1 }))
  } catch (e) {
    toast.push('error', t('connections.kill_failed', { msg: e instanceof Error ? e.message : String(e) }))
  }
}

async function killByHost(row: ConnectionRowType) {
  closeRowMenu()
  if (!row.host) return
  await runKill(() => store.closeByHost(row.host))
}

async function killByRule(row: ConnectionRowType) {
  closeRowMenu()
  if (!row.rule) return
  await runKill(() => store.closeByRule(row.rule))
}

/** Alt-mode entry: force-kill every connection egressing through `node`. */
function runBatchKill(node: string) {
  closeRowMenu()
  void runKill(() => store.closeByProxy(node))
}

async function runKill(fn: () => Promise<KillReport>) {
  if (menuBusy.value) return
  menuBusy.value = true
  try {
    notifyKill(await fn())
  } catch (e) {
    toast.push('error', t('connections.kill_failed', { msg: e instanceof Error ? e.message : String(e) }))
  } finally {
    menuBusy.value = false
  }
}

// Keyboard navigation: ↑/↓ move, Enter confirms, Esc closes. Alt is tracked
// live, so holding/releasing it reshapes the menu on the fly. Listeners are
// bound only while the menu is open, so it never steals keys from the app.
function onMenuKeydown(e: KeyboardEvent) {
  if (!rowMenu.value) return
  altMode.value = e.altKey
  const items = menuItems.value
  if (e.key === 'ArrowDown') {
    e.preventDefault()
    activeIndex.value = items.length ? (activeIndex.value + 1) % items.length : 0
  } else if (e.key === 'ArrowUp') {
    e.preventDefault()
    activeIndex.value = items.length ? (activeIndex.value - 1 + items.length) % items.length : 0
  } else if (e.key === 'Enter') {
    e.preventDefault()
    const item = items[activeIndex.value]
    if (item && !item.disabled) item.action()
  } else if (e.key === 'Escape') {
    e.preventDefault()
    closeRowMenu()
  }
}
function onMenuKeyup(e: KeyboardEvent) {
  if (e.key === 'Alt') altMode.value = false
}

watch(rowMenu, (v) => {
  if (v) {
    activeIndex.value = 0
    window.addEventListener('keydown', onMenuKeydown)
    window.addEventListener('keyup', onMenuKeyup)
  } else {
    window.removeEventListener('keydown', onMenuKeydown)
    window.removeEventListener('keyup', onMenuKeyup)
    altMode.value = false
  }
})

// Keep the highlight in range when the item set changes (Alt toggle).
watch(menuItems, (items) => {
  if (activeIndex.value >= items.length) activeIndex.value = 0
})
</script>

<template>
  <section class="overflow-hidden rounded-2xl border border-white/5 bg-white/[0.04] backdrop-blur-md">
    <header class="flex items-center justify-between px-5 py-3 border-b border-white/5">
      <div class="flex items-center gap-2">
        <h2 class="text-sm font-semibold text-zinc-100">{{ t('connections.title') }}</h2>
        <span class="rounded-md bg-indigo-500/20 px-1.5 py-0.5 text-[10px] font-mono tabular-nums text-indigo-200">
          {{ store.totalConnections }} total · {{ total }} shown
        </span>
        <span
          v-if="store.lastError"
          class="inline-flex items-center gap-1 rounded-md bg-rose-500/10 border border-rose-500/20 px-2 py-0.5 text-[10px] text-rose-300"
          :title="store.lastError"
        >
          <XCircle class="h-3 w-3" />
          {{ store.lastError }}
        </span>
      </div>
      <div class="flex items-center gap-3 text-[11px] font-mono tabular-nums">
        <span class="text-emerald-400">↑ {{ totalUpText }}</span>
        <span class="text-indigo-400">↓ {{ totalDownText }}</span>
      </div>
    </header>

    <div class="flex flex-wrap items-center gap-2 px-5 py-3 border-b border-white/5 bg-white/[0.02]">
      <div class="relative flex-1 min-w-[200px] max-w-md">
        <Search class="absolute left-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-zinc-500" />
        <input
          v-model="keyword"
          type="text"
          :placeholder="t('stats.rules.search_placeholder')"
          class="w-full pl-7 pr-2 py-1.5 rounded-lg bg-zinc-950/40 border border-white/5 text-xs text-zinc-100 placeholder-zinc-500 focus:outline-none focus:ring-1 focus:ring-indigo-500/50 focus:border-indigo-400/30 transition-colors"
        />
      </div>

      <div class="flex items-center gap-1.5">
        <Filter class="h-3.5 w-3.5 text-zinc-500" />
        <select
          v-model="policyFilter"
          class="rounded-lg bg-zinc-950/40 border border-white/5 text-xs text-zinc-100 px-2 py-1.5 focus:outline-none focus:ring-1 focus:ring-indigo-500/50"
        >
          <option value="">{{ t('stats.rules.all_types') }}</option>
          <option v-for="p in store.distinctPolicies" :key="p" :value="p">{{ p }}</option>
        </select>
      </div>

      <div class="flex items-center rounded-lg border border-white/5 overflow-hidden">
        <button
          v-for="opt in intervalOptions"
          :key="opt.value"
          type="button"
          class="px-2.5 py-1.5 text-xs font-mono transition-colors"
          :class="intervalChoice === opt.value
            ? 'bg-indigo-500/20 text-indigo-200'
            : 'text-zinc-400 hover:bg-white/5'"
          :disabled="store.isPaused"
          @click="intervalChoice = opt.value"
        >
          {{ opt.label }}
        </button>
        <button
          type="button"
          class="px-2 py-1.5 text-xs transition-colors border-l border-white/5"
          :class="store.isPaused
            ? 'bg-amber-500/20 text-amber-200'
            : 'text-zinc-400 hover:bg-white/5'"
          :title="store.isPaused ? t('common.start') : t('common.stop')"
          @click="togglePause"
        >
          <Pause v-if="!store.isPaused" class="h-3.5 w-3.5" />
          <Play v-else class="h-3.5 w-3.5" />
        </button>
      </div>

      <button
        type="button"
        :disabled="!kernel.isUp"
        class="inline-flex items-center gap-1.5 rounded-lg border border-white/5 bg-white/[0.04] hover:bg-white/[0.08] disabled:opacity-40 text-zinc-100 px-2.5 py-1.5 text-xs transition-colors"
        @click="manualRefresh"
      >
        <RefreshCw class="h-3.5 w-3.5" :class="store.busy ? 'animate-spin' : ''" />
        {{ t('common.refresh') }}
      </button>

      <button
        type="button"
        :disabled="!store.rows.length || closing"
        class="inline-flex items-center gap-1.5 rounded-lg bg-rose-500/80 hover:bg-rose-500 disabled:opacity-40 text-white px-2.5 py-1.5 text-xs transition-colors"
        @click="askCloseAll"
      >
        <Trash2 class="h-3.5 w-3.5" />
        {{ t('connections.disconnect_all') }}
      </button>
    </div>

    <Transition name="zls">
    <div
      v-if="total > 0"
      class="flex items-center gap-3 px-3 py-1.5 bg-white/[0.02] border-b border-white/5 text-[10px] text-zinc-500 uppercase tracking-wider font-semibold overflow-hidden"
    >
      <div class="flex-1 min-w-0">{{ t('connections.columns.host') }}</div>
      <div class="w-28 shrink-0">{{ t('connections.columns.process') }}</div>
      <div class="w-24 shrink-0">{{ t('connections.columns.network') }} · {{ t('connections.columns.type') }}</div>
      <div class="w-56 shrink-0">{{ t('connections.columns.rule') }}</div>
      <div class="w-40 shrink-0">Chain</div>
      <div class="w-44 shrink-0">{{ t('connections.columns.time') }} / {{ t('dashboard.traffic.upload') }} / {{ t('dashboard.traffic.download') }}</div>
      <div class="w-20 shrink-0 text-right">Sum</div>
      <div class="w-24 shrink-0">{{ t('connections.columns.rule') }}</div>
      <div class="w-8 shrink-0"></div>
    </div>
    </Transition>

    <Transition name="zls">
    <div
      v-if="store.stale && total > 0"
      class="flex items-center gap-2 px-3 py-2 border-b border-amber-500/20 bg-amber-500/[0.07] text-[11px] text-amber-300/90 overflow-hidden"
    >
      <AlertTriangle class="h-3.5 w-3.5 shrink-0" />
      <span>{{ t('connections.stale_hint') }}</span>
      <span class="ml-auto font-mono text-[10px] text-amber-300/60">{{ kernel.availability }}</span>
    </div>
    </Transition>

    <div ref="scrollEl" class="relative overflow-auto" style="height: 480px">
      <!-- State branches: skeleton / empty / loading / error / list.
           `mode="out-in"` makes each swap fade out fully before the next fades
           in, so the list never overlaps the skeleton or the empty state.
           The list branch is first and *persists* even when the kernel is down
           + stale — the mask below frosts it instead of blanking it. -->
      <Transition name="fade" mode="out-in">
        <div
          v-if="total > 0"
          key="list"
          :style="{ height: `${totalSize}px`, position: 'relative', width: '100%' }"
        >
          <div
            v-for="vrow in virtualRows"
            :key="vrow.index"
            :style="{
              position: 'absolute',
              top: 0, left: 0, width: '100%',
              transform: `translateY(${vrow.start}px)`,
            }"
          >
            <ConnectionRow
              :row="filteredRows[vrow.index]"
              @close="onCloseRow"
              @contextmenu="onRowContextMenu($event, filteredRows[vrow.index])"
            />
          </div>
        </div>

        <!-- Kernel down and nothing to preserve: invite the user to start it. -->
        <div
          v-else-if="!kernel.isUp"
          key="start"
          class="flex items-center justify-center h-full text-zinc-500 text-sm px-6 text-center"
        >
          Start the kernel to see live connections.
        </div>

        <!-- Initialised, no rows, no error: standard empty state. -->
        <div
          v-else-if="store.initialised && !store.lastError"
          key="empty"
          class="flex flex-col items-center justify-center h-full gap-2 text-zinc-500 text-sm px-6 text-center"
        >
          <Plug class="h-6 w-6 text-zinc-600" />
          <span>{{ t('connections.empty') }}</span>
        </div>

        <!-- First load. -->
        <div
          v-else-if="!store.initialised && !store.lastError"
          key="loading"
          class="flex items-center justify-center h-full text-zinc-500 text-sm"
        >
          <Loader2 class="h-4 w-4 animate-spin mr-2" />
          {{ t('common.loading') }}
        </div>

        <!-- Error. -->
        <div
          v-else
          key="error"
          class="flex items-center justify-center h-full text-rose-300 text-sm px-6 text-center"
        >
          {{ store.lastError }}
        </div>
      </Transition>

      <!-- Stale backdrop: frosts the *preserved* old list when the kernel is
           down, so the panel reads "data may be stale" instead of going blank.
           Sibling of the Transition (not inside it) so it stays put while the
           list branch remains mounted underneath. -->
      <Transition name="fade">
        <div
          v-if="kernel.availability === 'down' && store.stale && total > 0"
          class="absolute inset-0 z-10 flex items-center justify-center bg-zinc-950/40 backdrop-blur-sm"
        >
          <div class="flex flex-col items-center gap-2 text-center px-6">
            <AlertTriangle class="h-7 w-7 text-amber-400" />
            <span class="text-sm font-medium text-amber-200">{{ t('kernel.staleOverlay') }}</span>
          </div>
        </div>
      </Transition>
    </div>

    <Teleport to="body">
      <div
        v-if="confirmOpen"
        class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
        @click.self="confirmOpen = false"
      >
        <div class="rounded-2xl border border-white/10 bg-zinc-900/95 backdrop-blur-xl p-5 max-w-sm w-full mx-4 shadow-2xl">
          <h3 class="text-base font-semibold text-zinc-100">
            {{ t('connections.disconnect_confirm_title') }}
          </h3>
          <p class="mt-2 text-sm text-zinc-400">
            {{ t('connections.disconnect_confirm_body', { n: store.totalConnections }) }}
          </p>
          <div class="mt-4 flex justify-end gap-2">
            <button
              type="button"
              class="rounded-lg border border-white/5 bg-white/[0.04] hover:bg-white/[0.08] text-zinc-200 px-3 py-1.5 text-sm transition-colors"
              @click="confirmOpen = false"
            >
              {{ t('common.cancel') }}
            </button>
            <button
              type="button"
              :disabled="closing"
              class="inline-flex items-center gap-1.5 rounded-lg bg-rose-500 hover:bg-rose-400 disabled:opacity-60 text-white px-3 py-1.5 text-sm transition-colors"
              @click="doCloseAll"
            >
              <Loader2 v-if="closing" class="h-3.5 w-3.5 animate-spin" />
              <Trash2 v-else class="h-3.5 w-3.5" />
              {{ t('connections.disconnect_confirm_ok') }}
            </button>
          </div>
        </div>
      </div>
    </Teleport>

    <!-- Right-click row context menu. Teleported to <body> so it never clips
         inside the virtual list's scroll box. A full-screen catcher closes it;
         the panel itself gets a Scale (0.95→1) + Fade-in transition, supports
         ↑/↓/Enter/Esc, and reshapes into a ⚡ batch-kill panel while Alt is held. -->
    <Teleport to="body">
      <div
        v-if="rowMenu"
        class="fixed inset-0 z-[90]"
        @click="closeRowMenu"
        @contextmenu.prevent="closeRowMenu"
      ></div>
      <Transition name="menu">
        <div
          v-if="rowMenu"
          class="absolute z-[91] flex min-w-[200px] flex-col rounded-xl border border-white/10 bg-zinc-900/95 p-1 shadow-2xl backdrop-blur-xl"
          :style="{ left: `${rowMenu.x}px`, top: `${rowMenu.y}px` }"
          @click.stop
        >
          <button
            v-for="(item, i) in menuItems"
            :key="item.key"
            type="button"
            :disabled="item.disabled || menuBusy"
            class="flex items-center gap-2 rounded-lg px-3 py-2 text-xs transition-colors"
            :class="[
              item.disabled || menuBusy ? 'opacity-40 cursor-not-allowed' : 'hover:bg-white/10',
              i === activeIndex ? 'bg-white/10 text-zinc-100' : 'text-zinc-200',
            ]"
            @click="item.action()"
            @mouseenter="activeIndex = i"
          >
            <component
              :is="ICONS[item.icon]"
              class="h-3.5 w-3.5"
              :class="item.icon === 'zap' ? 'text-amber-400' : 'text-zinc-400'"
            />
            {{ item.label }}
          </button>
        </div>
      </Transition>
    </Teleport>
  </section>
</template>

<style scoped>
/* ZLS (Zero Layout Shift): the column header and the stale banner used to be
   hard v-if inserts that shoved the virtual list below them on every toggle.
   Now they expand/collapse via a smooth max-height + opacity transition, so
   the scroll box glides instead of jumping. overflow-hidden clips content
   mid-transition; max-height caps the resting height (both bars are < 64px). */
.zls-enter-active,
.zls-leave-active {
  transition: max-height 0.25s ease, opacity 0.2s ease;
  overflow: hidden;
}
.zls-enter-from,
.zls-leave-to {
  max-height: 0;
  opacity: 0;
}
.zls-enter-to,
.zls-leave-from {
  max-height: 64px;
  opacity: 1;
}

/* fade — used by the out-in state swap and the stale backdrop mask. */
.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.25s ease;
}
.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}

/* menu — the right-click context panel grows from the cursor corner
   (Scale 0.95→1) while fading in, so it feels like it springs from the
   pointer rather than popping into existence. */
.menu-enter-active,
.menu-leave-active {
  transition: opacity 0.15s ease, transform 0.15s ease;
  transform-origin: top left;
}
.menu-enter-from,
.menu-leave-to {
  opacity: 0;
  transform: scale(0.95);
}
</style>
