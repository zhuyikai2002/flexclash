// ============================================================================
// services/connections.ts — M8: connection monitor service layer.
//
// Wraps the raw Mihomo `/connections` axios calls with:
//   * Typed response parsing
//   * Connection → ConnectionRow projection (flatten metadata)
//   * Frame-to-frame byte counters used by the store to derive speed
// ============================================================================

import {
  closeAllConnections,
  closeConnection as mihomoCloseConnection,
  getConnections,
} from '@/services/clash'
import type { Connection, ConnectionRow } from '@/types/clash'

/** Pull the active connection list from Mihomo. */
export async function fetchConnections() {
  return await getConnections()
}

/** Drop a single connection by id. Throws if mihomo does not support
 *  per-id deletion (older builds) so the UI can fall back to close-all. */
export async function dropConnection(id: string): Promise<void> {
  try {
    await mihomoCloseConnection(id)
  } catch (e) {
    // Surface the error so the caller can decide on a fallback.
    throw e
  }
}

/** Drop all connections. */
export async function dropAllConnections(): Promise<void> {
  await closeAllConnections()
}

/** Project a raw `Connection` into a flat `ConnectionRow` for the UI. */
export function projectConnection(c: Connection): ConnectionRow {
  const meta = c.metadata
  const host = (meta?.host || c.host || c.dstIP || c.dst || '').trim()
  const process = (meta?.process || '').trim()
  const processPath = (meta?.processPath || '').trim()
  const src = c.src || (meta ? `${meta.sourceIP}:${meta.sourcePort}` : '')
  const dst = c.dst || (meta ? `${meta.destinationIP}:${meta.destinationPort}` : '')
  const sourceIP = meta?.sourceIP || c.srcIP || ''
  const sourcePort = meta?.sourcePort || ''
  const destinationIP = meta?.destinationIP || c.dstIP || ''
  const destinationPort = meta?.destinationPort || ''
  const chains = Array.isArray(c.chains) ? c.chains : []
  const policy = chains.length ? chains[chains.length - 1] : ''
  return {
    id: c.id,
    host,
    process,
    processPath,
    src,
    dst,
    sourceIP,
    sourcePort,
    destinationIP,
    destinationPort,
    network: (meta?.network || c.network || 'tcp') as ConnectionRow['network'],
    type: (meta?.type || c.type || 'Unknown') as ConnectionRow['type'],
    chains,
    rule: c.rule || '',
    policy,
    upload: Number(c.upload) || 0,
    download: Number(c.download) || 0,
    uploadSpeed: 0,
    downloadSpeed: 0,
    start: c.start || '',
  }
}

/** Compute a cheap content hash used to skip identical frames. */
export function hashConnections(rows: ConnectionRow[]): string {
  if (rows.length === 0) return '0:0:0'
  // Count + totals + max-id (cheap stable signature, not cryptographic).
  let totalUp = 0
  let totalDown = 0
  let maxId = ''
  for (const r of rows) {
    totalUp += r.upload
    totalDown += r.download
    if (r.id > maxId) maxId = r.id
  }
  return `${rows.length}:${totalUp}:${totalDown}:${maxId}`
}

/** Derive per-row speed by diffing against the previous frame. */
export function deriveSpeeds(
  current: ConnectionRow[],
  previous: Map<string, { upload: number; download: number }>,
  dtMs: number,
): void {
  if (dtMs <= 0) return
  for (const row of current) {
    const prev = previous.get(row.id)
    if (prev) {
      const dUp = Math.max(0, row.upload - prev.upload)
      const dDown = Math.max(0, row.download - prev.download)
      // bytes/sec
      row.uploadSpeed = (dUp * 1000) / dtMs
      row.downloadSpeed = (dDown * 1000) / dtMs
    }
    previous.set(row.id, { upload: row.upload, download: row.download })
  }
}
