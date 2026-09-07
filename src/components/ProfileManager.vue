<script setup lang="ts">
/**
 * ProfileManager — list, activate, delete, update + quota progress.
 * (M6 logic preserved; restyled to match the new design system.)
 */
import { computed, onMounted, ref } from 'vue'
import { Trash2, RefreshCw, Star, Check, Download, RotateCw, CalendarClock } from 'lucide-vue-next'
import { useProfilesStore } from '@/stores/profiles'
import { formatBytes, formatRelative } from '@/utils/format'
import { useI18n } from '@/composables/useI18n'
import type { ProfileMeta } from '@/types/clash'

const store = useProfilesStore()
const { t } = useI18n()

const emit = defineEmits<{ (e: 'open-subscribe'): void }>()

onMounted(() => { void store.refresh() })

interface QuotaInfo { used: number; total: number; frac: number; pct: number; over: boolean }
function quota(p: ProfileMeta): QuotaInfo | null {
  if (p.used_bytes == null || p.total_bytes == null) return null
  const used = Math.max(0, p.used_bytes)
  const total = Math.max(1, p.total_bytes)
  const real = used / total
  return { used, total, frac: Math.min(1, real), pct: Math.round(real * 100), over: real > 1 }
}
function quotaBarClass(q: QuotaInfo): string {
  if (q.over) return 'bg-rose-500'
  if (q.pct >= 80) return 'bg-amber-500'
  if (q.pct >= 50) return 'bg-amber-400'
  return 'bg-emerald-500'
}

interface ExpiryInfo { date: string; hint: string; color: string }
function expiry(p: ProfileMeta): ExpiryInfo | null {
  if (!p.expire_at) return null
  const d = new Date(p.expire_at)
  if (Number.isNaN(d.getTime())) return null
  const now = Date.now()
  const ms = d.getTime() - now
  const days = Math.round(ms / 86_400_000)
  let hint = `in ${days}d`
  let color = 'text-zinc-300'
  if (ms < 0) { hint = `${Math.abs(days)}d ago`; color = 'text-rose-400' }
  else if (days <= 3)  color = 'text-rose-300'
  else if (days <= 14) color = 'text-amber-300'
  return { date: d.toLocaleDateString(), hint, color }
}

const busyId = ref<string | null>(null)
const updatingId = computed(() => {
  for (const [id, v] of Object.entries(store.updating)) if (v) return id
  return null
})

async function activate(id: string) {
  const r = await store.activateProfile(id)
  if (r.status === 'failed') console.error('[profile] activate failed:', r.detail)
}
async function activateWithBusy(id: string) {
  busyId.value = id
  try { await activate(id) } finally { busyId.value = null }
}
async function updateOne(id: string) {
  try { await store.updateProfile(id) } catch (e) { console.error('[profile] update failed:', e) }
}
async function remove(id: string, name: string) {
  if (!confirm(t('profiles.delete_confirm_body', { name }))) return
  await store.remove(id)
}
</script>

<template>
  <section class="space-y-3">
    <header class="flex items-center justify-between">
      <div>
        <h2 class="text-base font-semibold tracking-tight text-zinc-100">
          {{ t('profiles.title') }}
        </h2>
        <p class="text-xs text-zinc-400 mt-0.5">
          {{ store.profiles.length }} stored
          <span v-if="store.active" class="ml-2 text-emerald-400">
            {{ t('profiles.active') }}: {{ store.active.name }}
          </span>
        </p>
      </div>
      <div class="flex items-center gap-2">
        <button
          class="inline-flex items-center gap-1.5 rounded-lg border border-white/5 bg-white/[0.04] px-2.5 py-1.5 text-xs text-zinc-200 hover:bg-white/[0.08] hover:border-white/10 transition-colors"
          :disabled="store.loading"
          @click="store.refresh()"
        >
          <RefreshCw :class="['h-3.5 w-3.5', store.loading && 'animate-spin']" />
          {{ t('common.refresh') }}
        </button>
        <button
          class="inline-flex items-center gap-1.5 rounded-lg bg-indigo-500 px-2.5 py-1.5 text-xs font-medium text-white hover:bg-indigo-400 transition-colors"
          @click="emit('open-subscribe')"
        >
          <Download class="h-3.5 w-3.5" />
          {{ t('common.import') }}
        </button>
      </div>
    </header>

    <p v-if="store.lastError" class="rounded-lg border border-rose-500/20 bg-rose-500/10 px-3 py-2 text-xs text-rose-300">
      {{ store.lastError }}
    </p>

    <p v-if="!store.loading && !store.profiles.length" class="rounded-2xl border border-dashed border-white/10 bg-white/[0.02] px-4 py-6 text-center text-sm text-zinc-400">
      {{ t('profiles.empty') }}. Click <em>{{ t('common.import') }}</em> to add a subscription.
    </p>

    <ul class="space-y-2">
      <li
        v-for="p in store.profiles"
        :key="p.id"
        class="rounded-2xl border border-white/5 bg-white/[0.04] hover:border-white/10 hover:bg-white/[0.06] transition-all px-3 py-2.5"
      >
        <div class="flex items-center gap-3">
          <div class="min-w-0 flex-1">
            <div class="flex items-center gap-2">
              <span class="truncate text-sm font-medium text-zinc-100">{{ p.name }}</span>
              <span
                v-if="store.activeId === p.id"
                class="inline-flex items-center gap-1 rounded-md bg-emerald-500/15 px-1.5 py-0.5 text-[10px] text-emerald-300"
              >
                <Check class="h-2.5 w-2.5" />{{ t('profiles.active') }}
              </span>
              <span class="rounded-md bg-white/5 px-1.5 py-0.5 text-[10px] text-zinc-300">
                {{ p.node_count }} nodes
              </span>
            </div>

            <div class="mt-1 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-[11px] text-zinc-400">
              <span>{{ t('profiles.subscription.last_update', { time: formatRelative(p.updated_at) }) }}</span>
              <span v-if="expiry(p)" :class="['inline-flex items-center gap-1', expiry(p)!.color]">
                <CalendarClock class="h-3 w-3" />
                {{ expiry(p)!.date }} ({{ expiry(p)!.hint }})
              </span>
              <span v-if="p.url" class="truncate font-mono text-[10px] text-zinc-500">{{ p.url }}</span>
            </div>

            <div
              v-if="quota(p)"
              class="mt-2 flex items-center gap-2"
              :title="`Used ${formatBytes(quota(p)!.used)} of ${formatBytes(quota(p)!.total)}`"
            >
              <div class="relative h-1.5 flex-1 overflow-hidden rounded-full bg-white/5">
                <div
                  class="absolute inset-y-0 left-0 transition-all"
                  :class="quotaBarClass(quota(p)!)"
                  :style="{ width: `${quota(p)!.frac * 100}%` }"
                ></div>
              </div>
              <span
                class="font-mono text-[10px]"
                :class="quota(p)!.over ? 'text-rose-300' : 'text-zinc-400'"
              >
                {{ formatBytes(quota(p)!.used) }} / {{ formatBytes(quota(p)!.total) }}
                ({{ quota(p)!.pct }}%)
              </span>
            </div>
          </div>

          <div class="flex shrink-0 items-center gap-1.5">
            <button
              v-if="p.url"
              class="inline-flex items-center gap-1 rounded-lg border border-emerald-500/20 bg-emerald-500/10 px-2 py-1 text-[11px] text-emerald-300 hover:bg-emerald-500/20 disabled:opacity-50 transition-colors"
              :disabled="updatingId === p.id"
              :title="t('profiles.subscription.update_now')"
              @click="updateOne(p.id)"
            >
              <RotateCw :class="['h-3 w-3', updatingId === p.id && 'animate-spin']" />
              <span v-if="updatingId === p.id">…</span>
              <span v-else>{{ t('profiles.subscription.update_now') }}</span>
            </button>
            <button
              v-if="store.activeId !== p.id"
              class="inline-flex items-center gap-1 rounded-lg border border-indigo-500/30 bg-indigo-500/15 px-2 py-1 text-[11px] text-indigo-200 hover:bg-indigo-500/25 disabled:opacity-50 transition-colors"
              :disabled="busyId === p.id"
              @click="activateWithBusy(p.id)"
            >
              <Star class="h-3 w-3" />
              <span v-if="busyId === p.id">…</span>
              <span v-else>{{ t('common.start') }}</span>
            </button>
            <button
              class="inline-flex items-center gap-1 rounded-lg border border-rose-500/20 bg-rose-500/10 px-2 py-1 text-[11px] text-rose-300 hover:bg-rose-500/20 transition-colors"
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
