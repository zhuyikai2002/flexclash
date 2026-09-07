<script setup lang="ts">
/**
 * TrafficHistoryChart — M10 native Canvas line chart.
 * (Hand-rolled to keep bundle small; see git history for the 200-line write-up.)
 */
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useHistoryStore } from '@/stores/history'
import { useI18n } from '@/composables/useI18n'

const history = useHistoryStore()
const { t } = useI18n()

const wrap = ref<HTMLDivElement | null>(null)
const canvas = ref<HTMLCanvasElement | null>(null)
const hover = ref<{ x: number; y: number; idx: number } | null>(null)

let ro: ResizeObserver | null = null
let dpr = 1
let cssW = 0
let cssH = 0

const buckets = computed(() => history.current?.buckets ?? [])
const bucketMs = computed(() => history.current?.bucket_ms ?? 60_000)

const fmtBytes = (b: number): string => {
  if (b <= 0) return '0 B'
  const u = ['B', 'KB', 'MB', 'GB']
  let i = 0; let n = b
  while (n >= 1024 && i < u.length - 1) { n /= 1024; i++ }
  return `${n.toFixed(n >= 100 ? 0 : n >= 10 ? 1 : 2)} ${u[i]}`
}

const fmtTime = (ts: number, range: string): string => {
  const d = new Date(ts)
  if (range === '1h')  return `${d.getHours().toString().padStart(2, '0')}:${d.getMinutes().toString().padStart(2, '0')}`
  if (range === '24h') return `${d.getHours().toString().padStart(2, '0')}:00`
  return `${d.getMonth() + 1}/${d.getDate()} ${d.getHours().toString().padStart(2, '0')}:00`
}

function resize() {
  if (!canvas.value || !wrap.value) return
  dpr = window.devicePixelRatio || 1
  cssW = wrap.value.clientWidth
  cssH = wrap.value.clientHeight || 180
  canvas.value.width  = Math.max(1, Math.floor(cssW * dpr))
  canvas.value.height = Math.max(1, Math.floor(cssH * dpr))
  canvas.value.style.width  = cssW + 'px'
  canvas.value.style.height = cssH + 'px'
  draw()
}

function draw() {
  const c = canvas.value
  if (!c) return
  const ctx = c.getContext('2d')
  if (!ctx) return
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
  ctx.clearRect(0, 0, cssW, cssH)

  const bs = buckets.value
  if (bs.length === 0) {
    ctx.fillStyle = '#52525b'
    ctx.font = '12px ui-sans-serif, system-ui'
    ctx.textAlign = 'center'
    ctx.textBaseline = 'middle'
    ctx.fillText(t('stats.chart.no_data'), cssW / 2, cssH / 2)
    return
  }

  const PAD_L = 44, PAD_R = 8, PAD_T = 6, PAD_B = 22
  const W = cssW - PAD_L - PAD_R
  const H = cssH - PAD_T - PAD_B
  const xAt = (i: number) => PAD_L + (i / (bs.length - 1 || 1)) * W
  const maxV = Math.max(1, ...bs.map((b) => Math.max(b.upload, b.download)))
  const yAt = (v: number) => PAD_T + H - (v / maxV) * H

  ctx.strokeStyle = '#27272a'
  ctx.fillStyle = '#71717a'
  ctx.font = '10px ui-monospace, monospace'
  ctx.textBaseline = 'middle'
  ctx.textAlign = 'right'
  for (let i = 0; i <= 3; i++) {
    const y = PAD_T + (H * i) / 3
    ctx.beginPath(); ctx.moveTo(PAD_L, y); ctx.lineTo(PAD_L + W, y); ctx.stroke()
    const val = maxV * (1 - i / 3)
    ctx.fillText(fmtBytes(val), PAD_L - 4, y)
  }
  ctx.textAlign = 'center'
  ctx.textBaseline = 'top'
  const range = history.range
  for (const i of [0, Math.floor((bs.length - 1) / 2), bs.length - 1]) {
    const x = xAt(i)
    ctx.beginPath(); ctx.moveTo(x, PAD_T + H); ctx.lineTo(x, PAD_T + H + 3); ctx.strokeStyle = '#3f3f46'; ctx.stroke()
    ctx.fillStyle = '#71717a'
    ctx.fillText(fmtTime(bs[i].ts, range), x, PAD_T + H + 6)
  }

  drawLine(ctx, bs, 'upload', '#34d399', xAt, yAt)
  drawLine(ctx, bs, 'download', '#818cf8', xAt, yAt)

  if (hover.value) {
    const x = xAt(hover.value.idx)
    ctx.strokeStyle = '#a1a1aa'
    ctx.setLineDash([3, 3])
    ctx.beginPath(); ctx.moveTo(x, PAD_T); ctx.lineTo(x, PAD_T + H); ctx.stroke()
    ctx.setLineDash([])
  }
}

function drawLine(
  ctx: CanvasRenderingContext2D,
  bs: { upload: number; download: number }[],
  key: 'upload' | 'download',
  color: string,
  xAt: (i: number) => number,
  yAt: (v: number) => number,
) {
  ctx.beginPath()
  ctx.moveTo(xAt(0), yAt(0))
  for (let i = 0; i < bs.length; i++) ctx.lineTo(xAt(i), yAt(bs[i][key]))
  ctx.lineTo(xAt(bs.length - 1), yAt(0))
  ctx.closePath()
  ctx.fillStyle = color + '22'
  ctx.fill()
  ctx.beginPath()
  for (let i = 0; i < bs.length; i++) {
    const x = xAt(i), y = yAt(bs[i][key])
    if (i === 0) ctx.moveTo(x, y); else ctx.lineTo(x, y)
  }
  ctx.strokeStyle = color
  ctx.lineWidth = 1.5
  ctx.stroke()
  if (hover.value) {
    const i = hover.value.idx
    ctx.beginPath(); ctx.arc(xAt(i), yAt(bs[i][key]), 3, 0, Math.PI * 2)
    ctx.fillStyle = color; ctx.fill()
  }
}

function onMove(e: MouseEvent) {
  const c = canvas.value
  if (!c) return
  const rect = c.getBoundingClientRect()
  const x = e.clientX - rect.left
  const bs = buckets.value
  if (bs.length === 0) { hover.value = null; return }
  const PAD_L = 44, PAD_R = 8
  const W = cssW - PAD_L - PAD_R
  const ratio = Math.max(0, Math.min(1, (x - PAD_L) / W))
  const idx = Math.round(ratio * (bs.length - 1))
  hover.value = { x: e.clientX, y: e.clientY, idx }
  draw()
}

function onLeave() {
  if (hover.value) { hover.value = null; draw() }
}

onMounted(() => {
  ro = new ResizeObserver(resize)
  if (wrap.value) ro.observe(wrap.value)
  resize()
})
onBeforeUnmount(() => { ro?.disconnect(); ro = null })
watch(() => [history.current, history.range], () => { hover.value = null; draw() })

const rangeOptions = [
  { id: '1h'  as const, label: t('stats.range.1h') },
  { id: '24h' as const, label: t('stats.range.24h') },
  { id: '7d'  as const, label: t('stats.range.7d') },
]
</script>

<template>
  <section class="rounded-2xl border border-white/5 bg-white/[0.04] p-4 backdrop-blur-md">
    <header class="mb-3 flex items-center justify-between">
      <div class="flex items-center gap-3 text-xs">
        <span class="inline-flex items-center gap-1.5">
          <span class="w-2.5 h-0.5 bg-emerald-400 inline-block" />
          <span class="text-zinc-400">{{ t('dashboard.traffic.upload') }} <span class="font-mono text-zinc-200">{{ fmtBytes(history.totalUpload) }}</span></span>
        </span>
        <span class="inline-flex items-center gap-1.5">
          <span class="w-2.5 h-0.5 bg-indigo-400 inline-block" />
          <span class="text-zinc-400">{{ t('dashboard.traffic.download') }} <span class="font-mono text-zinc-200">{{ fmtBytes(history.totalDownload) }}</span></span>
        </span>
      </div>
      <div class="inline-flex items-center gap-0.5 rounded-lg border border-white/5 bg-white/[0.02] p-0.5">
        <button
          v-for="r in rangeOptions"
          :key="r.id"
          @click="history.setRange(r.id)"
          :class="[
            'px-2.5 py-0.5 text-[11px] rounded-md transition-colors',
            history.range === r.id
              ? 'bg-white/10 text-zinc-100'
              : 'text-zinc-400 hover:text-zinc-200'
          ]"
        >{{ r.label }}</button>
      </div>
    </header>
    <div ref="wrap" class="relative w-full" style="height: 180px;">
      <canvas
        ref="canvas"
        class="block w-full h-full"
        @mousemove="onMove"
        @mouseleave="onLeave"
      />
      <div
        v-if="hover && buckets[hover.idx]"
        class="pointer-events-none absolute z-10 -translate-x-1/2 -translate-y-full px-2 py-1
               rounded-lg border border-white/10 bg-zinc-900/95 backdrop-blur-md text-[10px] font-mono whitespace-nowrap shadow-2xl"
        :style="{ left: hover.x - (wrap?.getBoundingClientRect().left ?? 0) + 'px', top: '4px' }"
      >
        <div class="text-zinc-400">{{ fmtTime(buckets[hover.idx].ts, history.range) }}</div>
        <div class="text-emerald-400">↑ {{ fmtBytes(buckets[hover.idx].upload) }}</div>
        <div class="text-indigo-400">↓ {{ fmtBytes(buckets[hover.idx].download) }}</div>
      </div>
    </div>
    <footer class="mt-2 text-[10px] text-zinc-500 font-mono flex items-center justify-between">
      <span>bucket: {{ bucketMs / 1000 }}s</span>
      <span v-if="history.sampleCount !== null">{{ history.sampleCount.toLocaleString() }} samples buffered</span>
    </footer>
  </section>
</template>
