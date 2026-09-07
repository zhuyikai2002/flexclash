// ============================================================================
// services/rules.ts — frontend wrapper around Mihomo `GET /rules`.
//
// Per the M10 brief: "前端直连 Mihomo REST/WS" for rules. Rust does not
// proxy rules; the UI talks straight to 127.0.0.1:9091 like the
// connections / proxies panels do.
// ============================================================================

import { axios } from './clash'
import type { RulesResponse, Rule, RuleType } from '@/types/clash'

export type { Rule, RuleType, RulesResponse }

export async function fetchRules(): Promise<Rule[]> {
  const r = await axios.get<RulesResponse>('/rules')
  return r.data.rules ?? []
}

/** Group rule types by category for the filter chip strip. */
export const RULE_TYPE_GROUPS: ReadonlyArray<{
  label: string
  types: readonly RuleType[]
}> = [
  { label: 'Domain', types: ['Domain', 'DomainSuffix', 'DomainKeyword', 'DomainRegex'] },
  { label: 'IP',     types: ['GEOIP', 'IPCIDR', 'IPCIDR6', 'SrcIPCIDR'] },
  { label: 'Port',   types: ['SrcPort', 'DstPort', 'SrcPortRange', 'DstPortRange'] },
  { label: 'Process', types: ['Process', 'ProcessPath', 'ProcessName', 'UID'] },
  { label: 'Logical', types: ['AND', 'OR', 'NOT', 'MATCH'] },
  { label: 'Other',  types: ['RuleSet', 'Network'] },
]
