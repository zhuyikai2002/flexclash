<script setup lang="ts">
/**
 * SystemProxyToggle — system proxy on/off card.
 *
 * Wraps the new full-card ToggleCard with the proxy-specific icon /
 * accent.  The whole card is clickable; the inner switch is decorative.
 */
import { onMounted, computed } from 'vue'
import { Network } from 'lucide-vue-next'
import { useProxyStore } from '@/stores/proxy'
import { useI18n } from '@/composables/useI18n'
import ToggleCard from './ToggleCard.vue'

const store = useProxyStore()
const { t } = useI18n()

onMounted(async () => {
  if (!store.status) {
    await store.refresh()
  }
})

const enabled = computed(() => store.enabled)
const port = computed(() => store.port)
const busy = computed(() => store.toggling)
const errMsg = computed(() => store.lastError)

const detail = computed(() => {
  if (!enabled.value) return ''
  return t('dashboard.toggles.system_proxy.current', {
    value: `127.0.0.1:${port.value ?? 7890}`,
  })
})

async function flip() {
  try {
    await store.toggle()
  } catch (e) {
    console.error('[proxy] toggle failed', e)
  }
}
</script>

<template>
  <ToggleCard
    :enabled="enabled"
    :busy="busy"
    :icon="Network"
    :title="t('dashboard.toggles.system_proxy.title')"
    :description="t('dashboard.toggles.system_proxy.description')"
    :detail="detail"
    :error="errMsg"
    accent="emerald"
    @toggle="flip"
  />
</template>
