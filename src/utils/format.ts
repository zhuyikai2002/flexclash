// ============================================================================
// utils/format.ts — shared formatting helpers.
// ============================================================================

/**
 * Best-effort relative time formatter. Accepts ISO-8601 strings, unix
 * millisecond timestamps, or `Date` instances. Returns "just now" for the
 * future / very-recent, otherwise a coarse "5m ago / 2h ago / 3d ago" form.
 */
export function formatRelative(input: string | number | Date): string {
  const date = input instanceof Date
    ? input
    : new Date(typeof input === 'number' ? input : input)
  const t = date.getTime()
  if (Number.isNaN(t)) return '-'

  const diffSec = Math.round((Date.now() - t) / 1000)
  if (diffSec < 0)        return 'just now'
  if (diffSec < 30)       return 'just now'
  if (diffSec < 60)       return `${diffSec}s ago`
  if (diffSec < 3600)     return `${Math.floor(diffSec / 60)}m ago`
  if (diffSec < 86400)    return `${Math.floor(diffSec / 3600)}h ago`
  if (diffSec < 86400*30) return `${Math.floor(diffSec / 86400)}d ago`
  if (diffSec < 86400*365) return `${Math.floor(diffSec / (86400*30))}mo ago`
  return `${Math.floor(diffSec / (86400*365))}y ago`
}

/**
 * Format a byte count in human-friendly units (B / KB / MB / GB / TB).
 * Returns the placeholder when the value is undefined or null.
 */
export function formatBytes(n: number | null | undefined, placeholder = '-'): string {
  if (n === null || n === undefined) return placeholder
  if (n <= 0) return '0 B'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  let i = 0
  let v = n
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++ }
  return `${v.toFixed(v >= 100 ? 0 : v >= 10 ? 1 : 2)} ${units[i]}`
}
