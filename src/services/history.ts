// ============================================================================
// services/history.ts — frontend wrapper around the M10 SQLite-backed
// traffic history Tauri commands.
//
// Data path: Rust opens the DB at %LOCALAPPDATA%\com.flexclash.app\history.db,
// samples /traffic every 5s, and answers `get_traffic_history` with
// pre-aggregated buckets. The frontend never touches SQLite directly.
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

import { invoke } from '@tauri-apps/api/core'

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

/** Fetch one pre-aggregated range. Throws on backend errors. */
export async function getTrafficHistory(range: HistoryRange): Promise<TrafficHistory> {
  return await invoke<TrafficHistory>('get_traffic_history', { range })
}

/** Absolute path of the SQLite file (for the "data is stored at…" line). */
export async function getHistoryDbPath(): Promise<string> {
  return await invoke<string>('get_history_db_path')
}

/** Count of rows currently in the table (live-monitor chip). */
export async function getHistorySampleCount(): Promise<number> {
  return await invoke<number>('get_history_sample_count')
}
