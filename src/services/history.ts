// ============================================================================
// services/history.ts — frontend wrapper around the M10 SQLite-backed
// traffic history Tauri commands.
//
// Data path: Rust opens the DB at %LOCALAPPDATA%\com.flexclash.app\history.db,
// samples /traffic every 5s, and answers `get_traffic_history` with
// pre-aggregated buckets. The frontend never touches SQLite directly.
//
// `safeInvoke` (and `safeInvokeOr` for the read-only probes) is used so
// the renderer can boot in a plain browser preview without DevTools errors.
//
// Wire shape (mirror of `store::queries::TrafficHistory`):
//   {
//     range: "1h" | "24h" | "7d",
//     bucket_ms: number,           // ms per bucket
//     buckets: { ts, upload, download }[],
//     total_upload: number,
//     total_download: number,
//   }
// ============================================================================

import { safeInvoke, safeInvokeOr } from '@/utils/tauri-bridge'

export type HistoryRange = '1h' | '24h' | '7d'

export interface HistoryPoint {
  /** Bucket start, epoch ms. */
  ts: number
  /** Bytes uploaded in this bucket. */
  upload: number
  /** Bytes downloaded in this bucket. */
  download: number
}

export interface TrafficHistory {
  range: HistoryRange
  bucket_ms: number
  buckets: HistoryPoint[]
  total_upload: number
  total_download: number
}

const EMPTY_HISTORY: TrafficHistory = {
  range: '1h',
  bucket_ms: 0,
  buckets: [],
  total_upload: 0,
  total_download: 0,
}

/** Fetch one pre-aggregated range. Throws on backend errors. In browser
 *  preview returns an empty shell so the chart can still render axes. */
export async function getTrafficHistory(range: HistoryRange): Promise<TrafficHistory> {
  return await safeInvokeOr<TrafficHistory>(
    'get_traffic_history',
    { ...EMPTY_HISTORY, range },
    { range },
  )
}

/** Absolute path of the SQLite file (for the "data is stored at…" line). */
export async function getHistoryDbPath(): Promise<string> {
  return await safeInvokeOr<string>('get_history_db_path', '(browser preview — no DB)')
}

/** Count of rows currently in the table (live-monitor chip). */
export async function getHistorySampleCount(): Promise<number> {
  return await safeInvokeOr<number>('get_history_sample_count', 0)
}
