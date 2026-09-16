// ============================================================================
// services/history.ts — frontend wrapper around the SQLite-backed traffic
// history Tauri commands.
//
// Data path: Rust opens the DB at %LOCALAPPDATA%\com.flexclash.app\history.db,
// samples /traffic every 5s, and answers `get_traffic_history` with
// pre-aggregated buckets. The frontend never touches SQLite directly.
//
// `TrafficHistory` / `HistoryPoint` are the generated bindings types — note
// that the Rust side types `range` as a plain `String`, so the narrow union
// the UI offers is declared here (as `HistoryRange`) and is *not* an IPC
// mirror.
// ============================================================================

import { commands, type TrafficHistory } from '@/bindings'
import { call, inTauri } from '@/utils/tauri-bridge'

/** The three windows the history view offers. A renderer-side choice, not a
 *  Rust enum: the command accepts any string. */
export type HistoryRange = '1h' | '24h' | '7d'

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
  if (!inTauri('get_traffic_history')) return { ...EMPTY_HISTORY, range }
  return call(commands.getTrafficHistory(range))
}

/** Absolute path of the SQLite file (for the "data is stored at…" line). */
export async function getHistoryDbPath(): Promise<string> {
  if (!inTauri('get_history_db_path')) return '(browser preview — no DB)'
  return await commands.getHistoryDbPath()
}

/** Count of rows currently in the table (live-monitor chip). */
export async function getHistorySampleCount(): Promise<number> {
  if (!inTauri('get_history_sample_count')) return 0
  return call(commands.getHistorySampleCount())
}
