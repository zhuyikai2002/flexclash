<script setup lang="ts">
/**
 * DiagnosticsDialog — the raw anomaly ring, for a human.
 *
 * The `anomalies` store already keeps the newest 50 `LogAnomaly` records so
 * fatal ones can be toasted. This is the other half: a read-only window onto
 * all of them, including the high-volume kinds (dial timeouts, refusals,
 * proxy switches) that are deliberately never shown as toasts.
 *
 * Scope is intentionally small — a table, a clear button, no filtering. This
 * is a troubleshooting aid for someone who already knows what a TLS error is,
 * not a dashboard.
 */
import { computed } from 'vue'
import { useI18n } from '@/composables/useI18n'
import { Activity, Trash2, X } from 'lucide-vue-next'
import { useAnomaliesStore, type AnomalyRecord } from '@/stores/anomalies'
import type { LogAnomaly } from '@/bindings'

const props = defineProps<{ open: boolean }>()
const emit = defineEmits<{ close: [] }>()

const { t } = useI18n()
const anomalies = useAnomaliesStore()

/** Newest first — the ring appends, but a log reads top-down. */
const rows = computed(() => [...anomalies.records].reverse())

/** The i18n key for an anomaly kind. */
function kindKey(a: LogAnomaly): string {
  return `anomalies.kind.${a.kind}`
}

/** What the anomaly was about: a host for name-based kinds, an address else. */
function targetOf(a: LogAnomaly): string {
  switch (a.kind) {
    case 'dns_resolve_failed':
    case 'tls_error':
      return a.host
    case 'dial_timeout':
    case 'connect_refused':
      return a.address
    case 'proxy_switch':
      return a.to
    default:
      return ''
  }
}

/** The free-text second column. */
function detailOf(a: LogAnomaly): string {
  switch (a.kind) {
    case 'dns_resolve_failed':
    case 'tls_error':
      return a.detail
    case 'dial_timeout':
      return a.elapsed_ms === null ? '' : `${a.elapsed_ms} ms`
    case 'proxy_switch':
      return a.from ? `${a.from} → ${a.to}` : ''
    default:
      return ''
  }
}

function timeOf(r: AnomalyRecord): string {
  return new Date(r.at).toLocaleTimeString()
}

/** Fatal kinds get a warm chip; the rest stay neutral so the eye can skip them. */
function isFatal(a: LogAnomaly): boolean {
  return a.kind === 'dns_resolve_failed' || a.kind === 'tls_error'
}
</script>

<template>
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
        v-if="props.open"
        class="fixed inset-0 z-[100] flex items-center justify-center p-4"
      >
        <div class="glass-scrim absolute inset-0" @click="emit('close')" />

        <div
          class="glass-popover relative w-full max-w-3xl max-h-[80vh] flex flex-col rounded-2xl"
        >
          <!-- Header -->
          <div class="flex items-center gap-3 border-b border-white/5 px-5 py-4">
            <div
              class="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-sky-500/10 ring-1 ring-sky-400/20"
            >
              <Activity class="h-4 w-4 text-sky-300" />
            </div>
            <div class="min-w-0 flex-1">
              <h2 class="text-base font-semibold text-zinc-100">
                {{ t('settings.diagnostics.log_title') }}
              </h2>
              <p class="mt-0.5 text-[11px] text-zinc-500">
                {{ t('settings.diagnostics.log_subtitle', { n: rows.length }) }}
              </p>
            </div>
            <button
              v-if="rows.length > 0"
              type="button"
              class="inline-flex items-center gap-1.5 rounded-lg border border-white/5 bg-white/[0.04] px-2.5 py-1.5 text-xs text-zinc-300 hover:bg-white/[0.08] transition-colors"
              @click="anomalies.clear()"
            >
              <Trash2 class="h-3.5 w-3.5" />
              {{ t('common.clear') }}
            </button>
            <button
              type="button"
              class="rounded-lg p-1.5 text-zinc-500 hover:bg-white/5 hover:text-zinc-200 transition-colors"
              :aria-label="t('common.close')"
              @click="emit('close')"
            >
              <X class="h-4 w-4" />
            </button>
          </div>

          <!-- Body -->
          <div class="flex-1 overflow-y-auto px-5 py-4">
            <div
              v-if="rows.length === 0"
              class="rounded-xl border border-dashed border-white/10 bg-white/[0.02] p-10 text-center text-sm text-zinc-500"
            >
              {{ t('settings.diagnostics.log_empty') }}
            </div>

            <table v-else class="w-full text-left text-xs">
              <thead class="text-[10px] uppercase tracking-wider text-zinc-500">
                <tr>
                  <th class="pb-2 pr-3 font-semibold">{{ t('settings.diagnostics.col_time') }}</th>
                  <th class="pb-2 pr-3 font-semibold">{{ t('settings.diagnostics.col_kind') }}</th>
                  <th class="pb-2 pr-3 font-semibold">{{ t('settings.diagnostics.col_target') }}</th>
                  <th class="pb-2 font-semibold">{{ t('settings.diagnostics.col_detail') }}</th>
                </tr>
              </thead>
              <tbody class="divide-y divide-white/5">
                <tr v-for="r in rows" :key="r.id" class="align-top">
                  <td class="py-2 pr-3 font-mono text-zinc-500 whitespace-nowrap">
                    {{ timeOf(r) }}
                  </td>
                  <td class="py-2 pr-3 whitespace-nowrap">
                    <span
                      :class="[
                        'inline-flex rounded-full px-1.5 py-0.5 text-[10px] font-medium',
                        isFatal(r.anomaly)
                          ? 'bg-rose-500/15 text-rose-300'
                          : 'bg-white/5 text-zinc-400',
                      ]"
                    >
                      {{ t(kindKey(r.anomaly)) }}
                    </span>
                  </td>
                  <td class="py-2 pr-3 font-mono text-zinc-300 break-all">
                    {{ targetOf(r.anomaly) || '—' }}
                  </td>
                  <td class="py-2 font-mono text-zinc-500 break-all">
                    {{ detailOf(r.anomaly) || '—' }}
                  </td>
                </tr>
              </tbody>
            </table>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>
