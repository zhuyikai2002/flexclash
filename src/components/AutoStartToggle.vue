<script setup lang="ts">
/**
 * AutoStartToggle — Windows auto-start on/off card.
 */
import { onMounted, computed } from 'vue'
import { Power } from 'lucide-vue-next'
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

const enabled = computed(() => store.enabled)
const busy = computed(() => store.busy)
const errMsg = computed(() => store.lastError)
const silent = computed(() => store.silent)

const detail = computed(() => {
  if (silent.value) {
    return t('time.just_now')
  }
  return ''
})

async function flip() {
  try {
    await store.toggle()
  } catch (e) {
    console.error('[autostart] toggle failed', e)
  }
}
</script>

<template>
  <ToggleCard
    :enabled="enabled"
    :busy="busy"
    :icon="Power"
    :title="t('dashboard.toggles.autostart.title')"
    :description="t('dashboard.toggles.autostart.description')"
    :detail="detail"
    :error="errMsg"
    :hint="`HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run\\FlexClash`"
    accent="indigo"
    @toggle="flip"
  />
</template>
