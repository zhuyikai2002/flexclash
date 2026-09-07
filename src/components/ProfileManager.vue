<script setup lang="ts">
// ============================================================================
// ProfileManager — list, activate, delete, and (M6) update + quota progress.
//
// M6 changes:
//   - Per-card "Update" button (visible only when `url` is non-empty) that
//     re-fetches the subscription. While in flight the row shows a spinner
//     and disables the other actions on the same card.
//   - Quota bar + percentage: shown when both `used_bytes` and `total_bytes`
//     are present. Overflow (>100%) is clamped and rendered in a warning
//     colour.
//   - Expiry date: rendered in human-friendly absolute form via
//     `toLocaleDateString`, with a "in N days" hint next to it.
// ============================================================================

import { computed, onMounted, ref } from 'vue'
import { Trash2, RefreshCw, Star, Check, Download, RotateCw, CalendarClock } from 'lucide-vue-next'
import { useProfilesStore } from '@/stores/profiles'
import { formatBytes, formatRelative } from '@/utils/format'
import type { ProfileMeta } from '@/types/clash'

const store = useProfilesStore()

const emit = defineEmits<{
  (e: 'open-subscribe'): void
}>()

onMounted(() => {
  void store.refresh()
})

// ---------------------------------------------------------------------------
// Quota bar
// ---------------------------------------------------------------------------

interface QuotaInfo {
  /** Used bytes. */
  used: number
  /** Total bytes. */
  total: number
  /** Used / total as fraction in [0, 1]. Clamped at 1.0. */
  frac: number
  /** Used / total in percent, rounded. */
  pct: number
  /** True when the subscription has exceeded its quota. */
  over: boolean
}

function quota(p: ProfileMeta): QuotaInfo | null {
  if (p.used_bytes == null || p.total_bytes == null) return null
  const used = Math.max(0, p.used_bytes)
  const total = Math.max(1, p.total_bytes)
  const real = used / total
  return {
    used,
    total,
    frac: Math.min(1, real),
    pct: Math.round(real * 100),
    over: real > 1,
  }
}

function quotaBarClass(q: QuotaInfo): string {
  if (q.over) return 'bg-rose-500'
  if (q.pct >= 80) return 'bg-amber-500'
  if (q.pct >= 50) return 'bg-amber-400'
  return 'bg-emerald-500'
}

// ---------------------------------------------------------------------------
// Expiry
// ---------------------------------------------------------------------------

interface ExpiryInfo {
  date: string
  hint: string
  color: string
}

function expiry(p: ProfileMeta): ExpiryInfo | null {
  if (!p.expire_at) return null
  const d = new Date(p.expire_at)
  if (Number.isNaN(d.getTime())) return null
  const now = Date.now()
  const ms = d.getTime() - now
  const days = Math.round(ms / 86_400_000)
  let hint: string
  let color: string
  if (ms < 0) {
    hint = `expired ${Math.abs(days)}d ago`
    color = 'text-rose-400'
  } else if (days <= 3) {
    hint = `in ${days}d`
    color = 'text-rose-300'
  } else if (days <= 14) {
    hint = `in ${days}d`
    color = 'text-amber-300'
  } else {
    hint = `in ${days}d`
    color = 'text-slate-300'
  }
  return {
    date: d.toLocaleDateString(),
    hint,
    color,
  }
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

const busyId = ref<string | null>(null)
const updatingId = computed(() => {
  for (const [id, v] of Object.entries(store.updating)) {
    if (v) return id
  }
  return null
})

async function activate(id: string) {
  const r = await store.activateProfile(id)
  if (r.status === 'failed') {
    console.error('[profile] activate failed:', r.detail)
  }
}

async function activateWithBusy(id: string) {
  busyId.value = id
  try { await activate(id) } finally { busyId.value = null }
}

async function updateOne(id: string) {
  try {
    await store.updateProfile(id)
  } catch (e) {
    console.error('[profile] update failed:', e)
  }
}

async function remove(id: string, name: string) {
  if (!confirm(`Delete profile "${name}"?`)) return
  await store.remove(id)
}
</script>

<template>
  <section class="space-y-3">
    <header class="flex items-center justify-between">
      <div>
        <h2 class="text-lg font-semibold text-slate-100">Profiles</h2>
        <p class="text-xs text-slate-400">
          {{ store.profiles.length }} stored
          <span v-if="store.active" class="ml-2 text-emerald-400">
            active: {{ store.active.name }}
          </span>
        </p>
      </div>
      <div class="flex items-center gap-2">
        <button
          class="inline-flex items-center gap-1.5 rounded-md border border-slate-700 bg-slate-800/60 px-2.5 py-1.5 text-xs text-slate-200 hover:bg-slate-700"
          :disabled="store.loading"
          @click="store.refresh()"
        >
          <RefreshCw :class="['h-3.5 w-3.5', store.loading && 'animate-spin']" />
          Refresh
        </button>
        <button
          class="inline-flex items-center gap-1.5 rounded-md bg-sky-600 px-2.5 py-1.5 text-xs font-medium text-white hover:bg-sky-500"
          @click="emit('open-subscribe')"
        >
          <Download class="h-3.5 w-3.5" />
          Import
        </button>
      </div>
    </header>

    <p v-if="store.lastError" class="rounded-md border border-rose-700/50 bg-rose-900/20 px-3 py-2 text-xs text-rose-200">
      {{ store.lastError }}
    </p>

    <p v-if="!store.loading && !store.profiles.length" class="rounded-md border border-dashed border-slate-700 px-4 py-6 text-center text-sm text-slate-400">
      No profiles yet. Click <em>Import</em> to add a subscription URL, import a yaml file,
      or paste the contents directly.
    </p>

    <ul class="space-y-2">
      <li
        v-for="p in store.profiles"
        :key="p.id"
        class="rounded-md border border-slate-700/60 bg-slate-800/40 px-3 py-2 transition hover:border-slate-600"
      >
        <div class="flex items-center gap-3">
          <div class="min-w-0 flex-1">
            <div class="flex items-center gap-2">
              <span class="truncate text-sm font-medium text-slate-100">{{ p.name }}</span>
              <span
                v-if="store.activeId === p.id"
                class="inline-flex items-center gap-1 rounded bg-emerald-900/40 px-1.5 py-0.5 text-[10px] text-emerald-300"
              >
                <Check class="h-2.5 w-2.5" />active
              </span>
              <span class="rounded bg-slate-700/50 px-1.5 py-0.5 text-[10px] text-slate-300">
                {{ p.node_count }} nodes
              </span>
            </div>

            <!-- Meta line -->
            <div class="mt-1 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-[11px] text-slate-400">
              <span>updated {{ formatRelative(p.updated_at) }}</span>
              <span v-if="expiry(p)" :class="['inline-flex items-center gap-1', expiry(p)!.color]">
                <CalendarClock class="h-3 w-3" />
                expires {{ expiry(p)!.date }} ({{ expiry(p)!.hint }})
              </span>
              <span v-if="p.url" class="truncate font-mono text-[10px] text-slate-500">{{ p.url }}</span>
            </div>

            <!-- Quota bar (M6) -->
            <div
              v-if="quota(p)"
              class="mt-2 flex items-center gap-2"
              :title="`Used ${formatBytes(quota(p)!.used)} of ${formatBytes(quota(p)!.total)}`"
            >
              <div class="relative h-1.5 flex-1 overflow-hidden rounded-full bg-slate-700/60">
                <div
                  class="absolute inset-y-0 left-0 transition-all"
                  :class="quotaBarClass(quota(p)!)"
                  :style="{ width: `${quota(p)!.frac * 100}%` }"
                ></div>
              </div>
              <span
                class="font-mono text-[10px]"
                :class="quota(p)!.over ? 'text-rose-300' : 'text-slate-400'"
              >
                {{ formatBytes(quota(p)!.used) }} / {{ formatBytes(quota(p)!.total) }}
                ({{ quota(p)!.pct }}%)
              </span>
            </div>
          </div>

          <div class="flex shrink-0 items-center gap-1.5">
            <button
              v-if="p.url"
              class="inline-flex items-center gap-1 rounded-md border border-emerald-800/60 bg-emerald-900/30 px-2 py-1 text-[11px] text-emerald-200 hover:bg-emerald-900/50 disabled:opacity-50"
              :disabled="updatingId === p.id"
              :title="`Re-fetch ${p.url}`"
              @click="updateOne(p.id)"
            >
              <RotateCw :class="['h-3 w-3', updatingId === p.id && 'animate-spin']" />
              <span v-if="updatingId === p.id">Updating…</span>
              <span v-else>Update</span>
            </button>
            <button
              v-if="store.activeId !== p.id"
              class="inline-flex items-center gap-1 rounded-md border border-sky-700 bg-sky-900/30 px-2 py-1 text-[11px] text-sky-200 hover:bg-sky-800/60 disabled:opacity-50"
              :disabled="busyId === p.id"
              @click="activateWithBusy(p.id)"
            >
              <Star class="h-3 w-3" />
              <span v-if="busyId === p.id">Activating…</span>
              <span v-else>Activate</span>
            </button>
            <button
              class="inline-flex items-center gap-1 rounded-md border border-rose-800/60 bg-rose-900/20 px-2 py-1 text-[11px] text-rose-200 hover:bg-rose-900/40"
              :title="`Delete ${p.name}`"
              @click="remove(p.id, p.name)"
            >
              <Trash2 class="h-3 w-3" />
            </button>
          </div>
        </div>
      </li>
    </ul>
  </section>
</template>
