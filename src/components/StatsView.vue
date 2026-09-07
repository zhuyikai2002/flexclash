<script setup lang="ts">
// ============================================================================
// StatsView — M10 /rules + traffic history composite view.
// Splitting it out keeps RulesView/TrafficHistoryChart reusable from
// anywhere (e.g. a future "Reports" tab in settings).
// ============================================================================

import TrafficHistoryChart from './TrafficHistoryChart.vue'
import RulesView from './RulesView.vue'
</script>

<template>
  <div class="space-y-6">
    <TrafficHistoryChart />
    <RulesView />
  </div>
</template>
