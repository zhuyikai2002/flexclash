<script setup lang="ts">
/**
 * SilentAutostartToggle — elevated (Task Scheduler) autostart card.
 *
 * Sits next to `AutoStartToggle` and is deliberately mutually exclusive
 * with it (enforced in Rust, mirrored in the store).
 *
 * Why two cards instead of one:
 *   - The registry entry starts the app with a filtered token. Nothing to
 *     authorise, but TUN mode then needs UAC every time it is switched on.
 *   - The scheduled task starts the app with an administrator token, so TUN
 *     never prompts — at the cost of a one-time UAC prompt when enabling,
 *     and of the app always running elevated.
 *
 * Users who do not use TUN should keep the registry switch; users who do
 * should use this one. Hiding that trade-off behind a single toggle would
 * silently escalate privileges for people who never asked for it.
 */
import { onMounted, computed } from 'vue'
import { ShieldCheck } from 'lucide-vue-next'
import { useDesktopStore } from '@/stores/desktop'
import { useI18n } from '@/composables/useI18n'
import ToggleCard from './ToggleCard.vue'

const store = useDesktopStore()
const { t } = useI18n()

onMounted(async () => {
  if (!store.initialised) {
    await store.init()
  }
})

const enabled = computed(() => store.taskEnabled)
const busy = computed(() => store.taskBusy)
const errMsg = computed(() => store.taskError)
const available = computed(() => store.taskAvailable)

const detail = computed(() =>
  enabled.value ? t('dashboard.toggles.autostart_elevated.active') : '',
)

async function flip() {
  try {
    await store.toggleSilentAutostart()
  } catch (e) {
    // The error is already in `store.taskError`; this only keeps the
    // rejection from surfacing as an unhandled promise warning.
    console.error('[autostart] elevated toggle failed', e)
  }
}
</script>

<template>
  <ToggleCard
    v-if="available"
    :enabled="enabled"
    :busy="busy"
    :icon="ShieldCheck"
    :title="t('dashboard.toggles.autostart_elevated.title')"
    :description="t('dashboard.toggles.autostart_elevated.description')"
    :detail="detail"
    :error="errMsg"
    :hint="t('dashboard.toggles.autostart_elevated.hint')"
    accent="sky"
    @toggle="flip"
  />
</template>
