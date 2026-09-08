// ============================================================================
// services/rules.ts — wrapper around Mihomo `GET /rules`.
//
// Phase R1: rules now come from the Rust façade command
// `get_mihomo_rules` instead of a renderer-side axios GET.
// ============================================================================

import { getRules } from './clash'
import type { Rule, RuleType } from '@/types/clash'

export type { Rule, RuleType }

export async function fetchRules(): Promise<Rule[]> {
  const res = await getRules()
  return res.rules ?? []
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
