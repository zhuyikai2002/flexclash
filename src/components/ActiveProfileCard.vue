<script setup lang="ts">
/**
 * ActiveProfileCard.vue — Phase 8 dashboard micro-card.
 *
 *   ┌─────────────────────────────────────────────┐
 *   │ 当前配置                  [↻ 立即更新]      │
 *   │ ✈  My-Airport-Subscription                  │
 *   │ ─────────────────────────────── 65%         │
 *   │ 已用 162.3 GB  /  共 250.0 GB  · 剩余 87.7  │
 *   │ 📅 2025-04-12 到期  · 更新于 2 分钟前        │
 *   └─────────────────────────────────────────────┘
 *
 * Reads from `useProfilesStore` (already populated by the M4/M7
 * pipeline).  Falls back to a friendly empty state when the user
 * has not yet imported a subscription.
 */
import { computed } from 'vue'
import { Plane, RefreshCw, Loader2, Calendar, Clock, AlertTriangle } from 'lucide-vue-next'
import { useI18n } from '@/composables/useI18n'
import { useProfilesStore } from '@/stores/profiles'
import { formatBytes, formatRelative } from '@/utils/format'

const { t, locale } = useI18n()
const profiles = useProfilesStore()

const active = computed(() => profiles.active)

const usedPct = computed(() => {
  const a = active.value
  if (!a?.total_bytes || a.total_bytes === 0) return null
  return Math.min(100, Math.round(((a.used_bytes ?? 0) / a.total_bytes) * 100))
})

const used = computed(() => active.value?.used_bytes ?? 0)
const total = computed(() => active.value?.total_bytes ?? 0)
const remaining = computed(() => {
  if (!active.value?.total_bytes) return null
  return Math.max(0, active.value.total_bytes - (active.value.used_bytes ?? 0))
})

const expiry = computed(() => {
  const exp = active.value?.expire_at
  if (!exp) return null
  return new Date(exp)
})

const daysToExpiry = computed(() => {
  const e = expiry.value
  if (!e) return null
  const ms = e.getTime() - Date.now()
  return Math.max(-999, Math.floor(ms / 86_400_000))
})

const isExpired = computed(() => daysToExpiry.value !== null && daysToExpiry.value < 0)
const isCloseToExpiry = computed(() => daysToExpiry.value !== null && daysToExpiry.value >= 0 && daysToExpiry.value <= 7)

const expiryText = computed(() => {
  if (!expiry.value) return t('dashboard.active_profile.no_expiry')
  const d = expiry.value.toLocaleDateString(locale.value === 'zh-CN' ? 'zh-CN' : 'en-US', {
    year: 'numeric', month: '2-digit', day: '2-digit',
  })
  return isExpired.value
    ? t('dashboard.active_profile.expired', { date: d })
    : t('dashboard.active_profile.expires', { date: d })
})

const progressColor = computed(() => {
  if (usedPct.value === null) return 'bg-white/10'
  if (usedPct.value >= 90) return 'bg-gradient-to-r from-rose-500 to-amber-400'
  if (usedPct.value >= 70) return 'bg-gradient-to-r from-amber-400 to-yellow-300'
  return 'bg-gradient-to-r from-sky-500 to-indigo-400'
})

const updatedAgo = computed(() => {
  const ts = active.value?.updated_at
  if (!ts) return null
  return formatRelative(new Date(ts))
})

const isActiveUpdating = computed(() =>
  active.value ? !!profiles.updating[active.value.id] : false,
)

async function quickUpdate() {
  if (!active.value || isActiveUpdating.value) return
  await profiles.updateProfile(active.value.id)
}
</script>

<template>
  <section
    class="rounded-2xl border border-white/5 bg-white/[0.04] backdrop-blur-md p-5 space-y-3"
  >
    <header class="flex items-center justify-between">
      <h2 class="text-sm font-semibold text-zinc-100">
        {{ t('dashboard.active_profile.title') }}
      </h2>
      <button
        v-if="active"
        type="button"
        :disabled="isActiveUpdating"
        @click="quickUpdate"
        class="inline-flex items-center gap-1.5 rounded-lg border border-white/10 bg-white/[0.04] hover:bg-white/[0.08] disabled:opacity-60 text-zinc-200 px-2.5 py-1 text-xs font-medium transition-colors"
        :title="t('dashboard.active_profile.update_now')"
      >
        <Loader2 v-if="isActiveUpdating" class="w-3 h-3 animate-spin" />
        <RefreshCw v-else class="w-3 h-3" />
        <span>{{ isActiveUpdating ? t('dashboard.active_profile.updating') : t('dashboard.active_profile.update_now') }}</span>
      </button>
    </header>

    <!-- Empty state -->
    <div v-if="!active" class="flex items-start gap-3 py-3">
      <div class="w-9 h-9 rounded-lg bg-white/5 flex items-center justify-center shrink-0">
        <Plane class="w-4 h-4 text-zinc-500" />
      </div>
      <div>
        <div class="text-sm text-zinc-200 font-medium">
          {{ t('dashboard.active_profile.no_active') }}
        </div>
        <div class="text-[11px] text-zinc-500 mt-0.5">
          {{ t('dashboard.active_profile.no_active_desc') }}
        </div>
      </div>
    </div>

    <!-- Active state -->
    <div v-else class="space-y-3">
      <div class="flex items-center gap-2.5">
        <div class="w-9 h-9 rounded-lg bg-sky-500/10 ring-1 ring-sky-400/20 flex items-center justify-center shrink-0">
          <Plane class="w-4 h-4 text-sky-300" />
        </div>
        <div class="min-w-0 flex-1">
          <div class="text-sm text-zinc-100 font-medium truncate">
            {{ active.name }}
          </div>
          <div v-if="active.url" class="text-[10px] text-zinc-500 font-mono truncate">
            {{ active.url }}
          </div>
        </div>
      </div>

      <!-- Traffic bar -->
      <div v-if="active.total_bytes && active.total_bytes > 0" class="space-y-1.5">
        <div class="flex items-baseline justify-between text-[11px] text-zinc-400">
          <span>
            <span class="text-zinc-200 font-mono">{{ formatBytes(used) }}</span>
            <span class="text-zinc-500"> / </span>
            <span class="text-zinc-300 font-mono">{{ formatBytes(total) }}</span>
            <span class="text-zinc-500 ml-1.5">{{ t('dashboard.active_profile.used') }}</span>
          </span>
          <span v-if="remaining !== null" class="font-mono">
            {{ formatBytes(remaining) }} {{ t('dashboard.active_profile.remaining') }}
          </span>
        </div>
        <div class="h-1.5 w-full rounded-full bg-white/5 overflow-hidden">
          <div
            class="h-full rounded-full transition-all duration-300"
            :class="progressColor"
            :style="{ width: `${usedPct ?? 0}%` }"
          />
        </div>
        <div v-if="usedPct !== null" class="text-[10px] text-zinc-500 font-mono text-right">
          {{ usedPct }}%
        </div>
      </div>

      <!-- No quota -->
      <div v-else-if="active.used_bytes" class="text-[11px] text-zinc-400 font-mono">
        {{ t('dashboard.active_profile.used') }} <span class="text-zinc-200">{{ formatBytes(active.used_bytes) }}</span>
        <span class="text-zinc-500"> · </span>
        <span>{{ t('dashboard.active_profile.unlimited') }}</span>
      </div>

      <!-- Footer: expiry + last update -->
      <div class="flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px] text-zinc-400">
        <span
          v-if="expiry"
          :class="[
            'inline-flex items-center gap-1',
            isExpired ? 'text-rose-300/90' :
            isCloseToExpiry ? 'text-amber-300/90' : 'text-zinc-400'
          ]"
        >
          <AlertTriangle v-if="isExpired || isCloseToExpiry" class="w-3 h-3" />
          <Calendar v-else class="w-3 h-3" />
          {{ expiryText }}
        </span>
        <span v-if="updatedAgo" class="inline-flex items-center gap-1 text-zinc-500">
          <Clock class="w-3 h-3" />
          {{ t('dashboard.active_profile.updated_at', { time: updatedAgo }) }}
        </span>
      </div>
    </div>
  </section>
</template>
