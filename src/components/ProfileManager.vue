<script setup lang="ts">
/**
 * ProfileManager — list, activate, delete, update + quota progress.
 *
 * Phase 9 / 2025 enhancements:
 *   - WINDOW-LEVEL drag & drop: drop .yaml / .yml files anywhere on
 *     the page; an elegant full-screen dashed overlay is shown
 *     while dragging.
 *   - "⋯" overflow menu on every card with three actions: Rename,
 *     Open in system editor, Reveal in file explorer (and Delete is
 *     moved into the same menu so the card stays tidy).
 *   - Cards show the file basename (`19f29131ce0.yaml`) as a
 *     monospace subtitle so the user can correlate the friendly
 *     alias (e.g. "主力机场") with the on-disk file.
 *
 * The store still owns all mutations; this component is a thin
 * presentation layer.
 */
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import {
  Trash2,
  RefreshCw,
  Star,
  Check,
  Download,
  RotateCw,
  CalendarClock,
  MoreHorizontal,
  Pencil,
  ExternalLink,
  FolderOpen,
} from 'lucide-vue-next'
import { useProfilesStore } from '@/stores/profiles'
import {
  openProfileInEditor,
  revealProfileFile,
} from '@/services/profile'
import { formatBytes, formatRelative } from '@/utils/format'
import { useI18n } from '@/composables/useI18n'
import type { ProfileMeta } from '@/types/clash'

const store = useProfilesStore()
const { t } = useI18n()

const emit = defineEmits<{ (e: 'open-subscribe'): void }>()

onMounted(() => {
  void store.refresh()
  // Window-level drag/drop.  We listen on `window` rather than the
  // section so the dashed overlay covers the entire viewport, and
  // so dropping near the bottom of the scroll area still works.
  window.addEventListener('dragenter', onDragEnter)
  window.addEventListener('dragover',   onDragOver)
  window.addEventListener('dragleave',  onDragLeave)
  window.addEventListener('drop',       onDrop)
})

onBeforeUnmount(() => {
  window.removeEventListener('dragenter', onDragEnter)
  window.removeEventListener('dragover',  onDragOver)
  window.removeEventListener('dragleave', onDragLeave)
  window.removeEventListener('drop',      onDrop)
})

// ============================================================================
// Drag & drop
// ============================================================================
// We track an integer counter (`dragDepth`) instead of a boolean so the
// overlay doesn't flicker when the cursor crosses internal element
// boundaries — every child element re-fires `dragenter` / `dragleave`.
// ============================================================================
const isDragging = ref(false)
const dragDepth = ref(0)
let dragCounterTimer: ReturnType<typeof setTimeout> | null = null

function onDragEnter(e: DragEvent) {
  if (!hasFiles(e)) return
  e.preventDefault()
  dragDepth.value += 1
  if (dragCounterTimer) clearTimeout(dragCounterTimer)
  // Belt-and-suspenders: if the browser fires `dragenter` without a
  // matching `dragleave` (some Win32 / WebView2 builds do this after
  // a window focus change) we still reset the overlay after a beat.
  dragCounterTimer = setTimeout(() => {
    if (dragDepth.value > 0) dragDepth.value = 0
    isDragging.value = false
  }, 8000)
  isDragging.value = true
}
function onDragOver(e: DragEvent) {
  if (!hasFiles(e)) return
  e.preventDefault()
  if (e.dataTransfer) e.dataTransfer.dropEffect = 'copy'
}
function onDragLeave(e: DragEvent) {
  if (!hasFiles(e)) return
  e.preventDefault()
  dragDepth.value = Math.max(0, dragDepth.value - 1)
  if (dragDepth.value === 0) isDragging.value = false
}
function hasFiles(e: DragEvent): boolean {
  const types = e.dataTransfer?.types
  if (!types) return false
  // 'Files' is the standard type, but some browsers also report
  // 'application/x-moz-file' etc.  Check by name prefix instead.
  for (let i = 0; i < types.length; i++) {
    if (types[i] === 'Files' || types[i] === 'application/x-moz-file') return true
  }
  return false
}

const importToast = ref<{ kind: 'ok' | 'err'; text: string } | null>(null)
let toastTimer: ReturnType<typeof setTimeout> | null = null
function flashToast(kind: 'ok' | 'err', text: string) {
  importToast.value = { kind, text }
  if (toastTimer) clearTimeout(toastTimer)
  toastTimer = setTimeout(() => { importToast.value = null }, 3500)
}

async function onDrop(e: DragEvent) {
  e.preventDefault()
  dragDepth.value = 0
  isDragging.value = false
  const files = e.dataTransfer?.files
  if (!files || files.length === 0) return
  const accepted: File[] = []
  for (const f of Array.from(files)) {
    const lower = f.name.toLowerCase()
    if (lower.endsWith('.yaml') || lower.endsWith('.yml')) accepted.push(f)
  }
  if (accepted.length === 0) {
    flashToast('err', t('profiles.drag_drop_invalid'))
    return
  }
  let ok = 0
  for (const f of accepted) {
    try {
      const text = await f.text()
      const display = f.name.replace(/\.(ya?ml)$/i, '')
      await store.pasteYaml(display, text)
      ok += 1
    } catch (err) {
      console.error('[profile] drop import failed:', err)
    }
  }
  if (ok > 0) {
    flashToast('ok', `${ok}/${accepted.length} imported`)
  } else {
    flashToast('err', t('profiles.drag_drop_invalid'))
  }
}

// ============================================================================
// Card-level: helpers
// ============================================================================
function basename(p: ProfileMeta): string {
  const path = p.file_path
  const idx = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'))
  return idx >= 0 ? path.slice(idx + 1) : path
}

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

// ============================================================================
// Card-level: action menu + rename dialog
// ============================================================================
const menuOpenId = ref<string | null>(null)
const renamingId = ref<string | null>(null)
const renameDraft = ref('')
const renameError = ref<string | null>(null)
const renameBusy = ref(false)

function openMenu(id: string) {
  menuOpenId.value = menuOpenId.value === id ? null : id
}
function closeMenu() {
  menuOpenId.value = null
}
function startRename(p: ProfileMeta) {
  renamingId.value = p.id
  renameDraft.value = p.name
  renameError.value = null
  closeMenu()
}
function cancelRename() {
  renamingId.value = null
  renameDraft.value = ''
  renameError.value = null
}
async function submitRename() {
  if (!renamingId.value) return
  const next = renameDraft.value.trim()
  if (next.length === 0) {
    renameError.value = 'name cannot be empty'
    return
  }
  if (next.length > 80) {
    renameError.value = 'name too long (>80)'
    return
  }
  renameBusy.value = true
  try {
    await store.rename(renamingId.value, next)
    cancelRename()
  } catch (e) {
    renameError.value = e instanceof Error ? e.message : String(e)
  } finally {
    renameBusy.value = false
  }
}

async function openInEditor(id: string) {
  closeMenu()
  try {
    await openProfileInEditor(id)
  } catch (e) {
    console.error('[profile] open in editor failed:', e)
    flashToast('err', e instanceof Error ? e.message : String(e))
  }
}
async function revealInExplorer(id: string) {
  closeMenu()
  try {
    await revealProfileFile(id)
  } catch (e) {
    console.error('[profile] reveal failed:', e)
    flashToast('err', e instanceof Error ? e.message : String(e))
  }
}
async function removeWithConfirm(p: ProfileMeta) {
  closeMenu()
  if (!confirm(t('profiles.delete_confirm_body', { name: p.name }))) return
  await store.remove(p.id)
}

// Close the popover when the user clicks anywhere else.
function onWindowClick(e: MouseEvent) {
  if (!menuOpenId.value) return
  const t = e.target as HTMLElement | null
  if (!t) return
  if (t.closest('[data-profile-menu]')) return
  closeMenu()
}
onMounted(() => window.addEventListener('mousedown', onWindowClick))
onBeforeUnmount(() => window.removeEventListener('mousedown', onWindowClick))
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
        <p class="mt-1 text-[11px] text-zinc-500">
          {{ t('profiles.drag_drop_hint') }}
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

    <p v-if="importToast" :class="[
      'rounded-lg border px-3 py-2 text-xs',
      importToast.kind === 'ok'
        ? 'border-emerald-500/20 bg-emerald-500/10 text-emerald-300'
        : 'border-rose-500/20 bg-rose-500/10 text-rose-300',
    ]">
      {{ importToast.text }}
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
              <!-- Display name (alias) -->
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

            <!-- File basename subtitle so the alias can be correlated
                 with the on-disk file.  Shown in monospace to look
                 like a path. -->
            <div class="mt-0.5 font-mono text-[10.5px] text-zinc-500 truncate" :title="p.file_path">
              {{ basename(p) }}
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
            <!-- "⋯" overflow menu -->
            <div class="relative" data-profile-menu>
              <button
                class="inline-flex h-7 w-7 items-center justify-center rounded-lg border border-white/5 bg-white/[0.04] text-zinc-300 hover:bg-white/[0.10] hover:text-zinc-100 transition-colors"
                :aria-expanded="menuOpenId === p.id"
                :aria-label="t('profiles.more_actions')"
                :title="t('profiles.more_actions')"
                @click="openMenu(p.id)"
              >
                <MoreHorizontal class="h-3.5 w-3.5" />
              </button>
              <div
                v-if="menuOpenId === p.id"
                role="menu"
                class="absolute right-0 top-9 z-50 w-48 overflow-hidden rounded-xl border border-white/10 bg-zinc-900/95 shadow-2xl shadow-black/50 backdrop-blur-md"
              >
                <button
                  class="flex w-full items-center gap-2 px-3 py-2 text-left text-[12px] text-zinc-100 hover:bg-white/[0.06]"
                  @click="startRename(p)"
                >
                  <Pencil class="h-3.5 w-3.5 text-zinc-400" />
                  {{ t('profiles.rename') }}
                </button>
                <button
                  class="flex w-full items-center gap-2 px-3 py-2 text-left text-[12px] text-zinc-100 hover:bg-white/[0.06]"
                  @click="openInEditor(p.id)"
                >
                  <ExternalLink class="h-3.5 w-3.5 text-zinc-400" />
                  {{ t('profiles.open_in_editor') }}
                </button>
                <button
                  class="flex w-full items-center gap-2 px-3 py-2 text-left text-[12px] text-zinc-100 hover:bg-white/[0.06]"
                  @click="revealInExplorer(p.id)"
                >
                  <FolderOpen class="h-3.5 w-3.5 text-zinc-400" />
                  {{ t('profiles.reveal_in_explorer') }}
                </button>
                <div class="my-1 h-px bg-white/5" />
                <button
                  class="flex w-full items-center gap-2 px-3 py-2 text-left text-[12px] text-rose-300 hover:bg-rose-500/10"
                  @click="removeWithConfirm(p)"
                >
                  <Trash2 class="h-3.5 w-3.5" />
                  {{ t('profiles.delete') }}
                </button>
              </div>
            </div>
          </div>
        </div>
      </li>
    </ul>

    <!-- =========================================================================
         DRAG-AND-DROP FULLSCREEN OVERLAY
         =========================================================================
         The overlay covers the entire viewport (fixed inset-0) and is
         purely decorative; it has pointer-events: none so drop events
         fall through to `window`.  We only show it when at least one
         dragenter has fired and we haven't yet seen a matching
         dragleave / drop.
    ========================================================================== -->
    <Transition
      enter-active-class="transition duration-150 ease-out"
      enter-from-class="opacity-0"
      enter-to-class="opacity-100"
      leave-active-class="transition duration-150 ease-in"
      leave-from-class="opacity-100"
      leave-to-class="opacity-0"
    >
      <div
        v-if="isDragging"
        class="pointer-events-none fixed inset-0 z-[60] flex items-center justify-center"
      >
        <div
          class="absolute inset-3 rounded-3xl border-2 border-dashed border-sky-500/60 bg-sky-500/[0.06]"
        />
        <div
          class="relative rounded-2xl border border-sky-500/40 bg-sky-500/10 px-8 py-5 text-center shadow-2xl shadow-sky-500/20"
        >
          <Download class="mx-auto h-7 w-7 text-sky-300" />
          <p class="mt-2 text-sm font-medium text-sky-100">
            {{ t('profiles.drag_drop_overlay_title') }}
          </p>
          <p class="mt-1 text-[11px] text-sky-200/80">
            {{ t('profiles.drag_drop_overlay_hint') }}
          </p>
        </div>
      </div>
    </Transition>

    <!-- =========================================================================
         RENAME MODAL
         ==========================================================================
         Lightweight confirm-modal style: backdrop + centered card.
         Keyboard support: Enter to confirm, Esc to cancel.
    ========================================================================== -->
    <Transition
      enter-active-class="transition duration-150 ease-out"
      enter-from-class="opacity-0"
      enter-to-class="opacity-100"
      leave-active-class="transition duration-150 ease-in"
      leave-from-class="opacity-100"
      leave-to-class="opacity-0"
    >
      <div
        v-if="renamingId"
        class="fixed inset-0 z-[70] flex items-center justify-center bg-black/50 backdrop-blur-sm"
        @keydown.esc="cancelRename"
        @keydown.enter="submitRename"
      >
        <div
          class="w-[min(420px,90vw)] rounded-2xl border border-white/10 bg-zinc-900/95 p-5 shadow-2xl shadow-black/60"
          role="dialog"
          aria-modal="true"
        >
          <h3 class="text-sm font-semibold text-zinc-100">
            {{ t('profiles.rename_title') }}
          </h3>
          <input
            v-model="renameDraft"
            type="text"
            :placeholder="t('profiles.rename_placeholder')"
            autofocus
            :disabled="renameBusy"
            class="mt-3 w-full rounded-lg border border-white/10 bg-zinc-950/60 px-3 py-2 text-sm text-zinc-100 placeholder:text-zinc-500 focus:border-sky-500/60 focus:outline-none focus:ring-1 focus:ring-sky-500/30"
            @keydown.esc="cancelRename"
            @keydown.enter="submitRename"
          />
          <p v-if="renameError" class="mt-2 text-[11px] text-rose-300 font-mono">
            {{ renameError }}
          </p>
          <div class="mt-4 flex items-center justify-end gap-2">
            <button
              class="rounded-lg border border-white/5 bg-white/[0.04] px-3 py-1.5 text-xs text-zinc-300 hover:bg-white/[0.08]"
              :disabled="renameBusy"
              @click="cancelRename"
            >
              {{ t('common.cancel') }}
            </button>
            <button
              class="rounded-lg bg-sky-500 px-3 py-1.5 text-xs font-medium text-white hover:bg-sky-400 disabled:opacity-50"
              :disabled="renameBusy"
              @click="submitRename"
            >
              {{ t('common.confirm') }}
            </button>
          </div>
        </div>
      </div>
    </Transition>
  </section>
</template>
