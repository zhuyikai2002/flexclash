// ============================================================================
// Mihomo (Clash Meta) RESTful API types — Phase 2 surface only.
// Field semantics follow https://wiki.metacubex.one/en/api/ and the mihomo
// /help endpoint output. Fields not used by the FlexClash UI are intentionally
// omitted to keep this file focused and the bundle small.
// ============================================================================

// ----- /version -------------------------------------------------------------

export interface MihomoVersion {
  meta: boolean
  version: string
}

// ----- /traffic  (WebSocket only — pushed messages) -------------------------

export interface TrafficSample {
  /** Upload bytes/sec, since the previous sample. */
  up: number
  /** Download bytes/sec, since the previous sample. */
  down: number
}

// ----- /proxies -------------------------------------------------------------

export type ProxyType =
  | 'Direct'
  | 'Global'
  | 'Rule'
  | 'Reject'
  | 'Selector'
  | 'URLTest'
  | 'Fallback'
  | 'LoadBalance'
  | 'Relay'
  | 'Compatible'

export interface ProxyDelayHistory {
  /** ISO-8601 timestamp. */
  time: string
  /** Measured delay in ms; 0 means unreachable. */
  delay: number
  /** Optional smoothed/EMA value. */
  meanDelay?: number
}

/** A single node inside a Selector / URLTest / Fallback / LoadBalance / Relay. */
export interface ProxyNode {
  name: string
  type: string
  udp?: boolean
  xudp?: boolean
  tfo?: boolean
}

/** Full description of a proxy group (or single node). */
export interface Proxy {
  name: string
  type: ProxyType
  /** Available child proxies for group types. */
  all?: string[]
  /** Currently-selected child for group types. */
  now?: string
  /** Recent delay samples for URLTest/Fallback/LoadBalance. */
  history?: ProxyDelayHistory[]
  udp?: boolean
  xudp?: boolean
  tfo?: boolean
  /** Single fallback proxy (Fallback type). */
  proxy?: ProxyNode
  /** Candidate proxies (LoadBalance type). */
  proxies?: ProxyNode[]
}

export interface ProxyProvider {
  name: string
  type: 'HTTP' | 'File' | 'Compatible'
  proxies: ProxyNode[]
  /** VehicleType info from mihomo; left loose intentionally. */
  vehicleType?: string
  /** ISO-8601 last update time, if known. */
  updatedAt?: string
}

export interface ProxiesResponse {
  proxies: Record<string, Proxy>
  providers?: Record<string, ProxyProvider>
}

/** Body for `PUT /proxies/:name` (selector switch). */
export interface SelectProxyBody {
  name: string
}

// ----- /connections ---------------------------------------------------------

export type ConnectionNetwork = 'tcp' | 'udp'
export type ConnectionType = 'HTTP' | 'HTTPS' | 'SOCKS' | 'SOCKS5' | 'Redir' | 'TProxy' | 'TUN' | 'Unknown'

/** Nested metadata block returned by mihomo for every connection. */
export interface ConnectionMetadata {
  network: ConnectionNetwork
  type: ConnectionType
  sourceIP: string
  sourcePort: string
  destinationIP: string
  destinationPort: string
  /** Sniffed / resolved hostname. */
  host: string
  /** Process that opened the socket, when available. */
  process?: string
  processPath?: string
  dnsMode?: string
  specialProxy?: string
  specialRules?: string
}

export interface Connection {
  id: string
  /** Flattened host (mihomo may omit this if `metadata.host` is unset). */
  host: string
  /** Source IP:port. */
  src: string
  /** Destination IP:port (after routing). */
  dst: string
  srcIP?: string
  dstIP?: string
  network: ConnectionNetwork
  type: ConnectionType
  /** Chained proxy names — last element is the egress. */
  chains: string[]
  /** Rule that matched this connection, e.g. `DOMAIN-SUFFIX,example.com,PROXY`. */
  rule: string
  /** Captured rule payload if any. */
  rulePayload?: string
  /** Bytes uploaded over this connection so far. */
  upload: number
  /** Bytes downloaded over this connection so far. */
  download: number
  /** ISO-8601 timestamp when the connection started (mihomo 1.19). */
  start: string
  /** Sniffed hostname (if sniffer enabled). */
  sniffHost?: string
  /** Sniffed protocol (e.g. "TLS", "HTTP/1.1"). */
  sniffResult?: string
  /** mihomo 1.19 nests the fields above under `metadata`. */
  metadata?: ConnectionMetadata
}

export interface ConnectionsResponse {
  /** Cumulative download bytes across all active connections. */
  downloadTotal: number
  /** Cumulative upload bytes across all active connections. */
  uploadTotal: number
  /** Connection-count limiter info. */
  connections: Connection[]
  /** Optional: total byte stats (only on /connections?withdetail=1 or ws). */
  memory?: number
}

/**
 * Flat row used by the dashboard virtual list. The store holds rows as
 * `shallowRef<ConnectionRow[]>` so we hand-roll the speed derivative on
 * each frame to keep the data path free of `ref()` overhead.
 */
export interface ConnectionRow {
  id: string
  host: string
  process: string
  processPath: string
  src: string
  dst: string
  sourceIP: string
  sourcePort: string
  destinationIP: string
  destinationPort: string
  network: ConnectionNetwork
  type: ConnectionType
  chains: string[]
  /** The matched rule, e.g. `DOMAIN-SUFFIX,example.com,PROXY`. */
  rule: string
  /** Derived short rule name (just the proxy at the end). */
  policy: string
  upload: number
  download: number
  /** Bytes/sec, derived from frame-to-frame delta. */
  uploadSpeed: number
  downloadSpeed: number
  /** ISO-8601 start time. */
  start: string
  /** True when mihomo flagged this connection as closing/erroring. */
  closing?: boolean
}

// ----- /configs -------------------------------------------------------------

export type Mode = 'rule' | 'global' | 'direct'
export type LogLevel = 'debug' | 'info' | 'warning' | 'error' | 'silent'

export interface Config {
  port?: number
  /** @deprecated prefer mixed-port */
  'socks-port'?: number
  'mixed-port'?: number
  'allow-lan'?: boolean
  mode: Mode
  'log-level': LogLevel
  'external-controller'?: string
  secret?: string
  ipv6?: boolean
  tun?: { enable: boolean; stack?: string; [k: string]: unknown }
  profile?: { 'store-selected'?: boolean; [k: string]: unknown }
  [k: string]: unknown
}

// ----- /rules ---------------------------------------------------------------

export type RuleType =
  | 'Domain'
  | 'DomainSuffix'
  | 'DomainKeyword'
  | 'DomainRegex'
  | 'GEOIP'
  | 'IPCIDR'
  | 'IPCIDR6'
  | 'SrcIPCIDR'
  | 'SrcPort'
  | 'DstPort'
  | 'SrcPortRange'
  | 'DstPortRange'
  | 'Process'
  | 'ProcessPath'
  | 'ProcessName'
  | 'UID'
  | 'Network'
  | 'RuleSet'
  | 'AND'
  | 'OR'
  | 'NOT'
  | 'MATCH'

export interface Rule {
  type: RuleType
  payload: string
  proxy: string
}

export interface RulesResponse {
  rules: Rule[]
}

// ----- /proxies/:name/delay  -----------------------------------------------

export interface ProxyDelayResponse {
  /** Delay in ms; 0 means unreachable. */
  delay: number
}

// ============================================================================
// Profile management (M4) — mirror `commands::profile` Rust wire types.
// ============================================================================

export interface ProfileMeta {
  id: string
  name: string
  url: string
  node_count: number
  updated_at: string
  file_path: string
  used_bytes?: number
  remaining_bytes?: number
  total_bytes?: number
  expire_at?: string
}

export type ReloadStatus = 'reloaded' | 'staged' | 'failed'

export interface ReloadResult {
  status: ReloadStatus
  detail: string
}
