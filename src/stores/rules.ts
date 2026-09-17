// ============================================================================
// stores/rules.ts — Pinia store for the M10 /rules view.
//
// Source of truth: Mihomo `GET /rules` (no Rust proxy). The store holds:
//   * the raw rule list,
//   * user filter state (search + type chip),
//   * a derived `filtered` list with stable ordering and a hit count.
//
// Refresh strategy: fetch on demand (no auto-poll — rules are static
// across the kernel's lifetime; reload on profile change is enough).
// ============================================================================

import { defineStore } from 'pinia'
import { fetchRules } from '@/services/rules'
import { useNoticesStore } from '@/stores/notices'
import type { Rule, RuleType } from '@/types/clash'

export type RuleTypeFilter = 'all' | RuleType

interface RulesState_ {
  rules: Rule[]
  loading: boolean
  lastFetchAt: number | null
  /** Free-text search, case-insensitive, matches payload OR proxy. */
  search: string
  /** Type filter. `all` is the no-op default. */
  typeFilter: RuleTypeFilter
}

export const useRulesStore = defineStore('rules', {
  state: (): RulesState_ => ({
    rules: [],
    loading: false,
    lastFetchAt: null,
    search: '',
    typeFilter: 'all',
  }),

  getters: {
    /** Thin proxy over the unified notice channel — see stores/notices.ts. */
    lastError: (): string | null =>
      useNoticesStore().latestFor('rules')?.message ?? null,
    /** True iff the user has narrowed the list at all. */
    isFiltered: (s): boolean => s.search.trim() !== '' || s.typeFilter !== 'all',
    /** All distinct proxy targets in the current rule set (for the
     *  "by policy" filter chip strip on the UI). */
    policyOptions: (s): string[] => {
      const set = new Set<string>()
      for (const r of s.rules) set.add(r.proxy)
      return [...set].sort()
    },
    /** The active filter pipeline. */
    filtered(): Rule[] {
      const q = this.search.trim().toLowerCase()
      return this.rules.filter((r) => {
        if (this.typeFilter !== 'all' && r.type !== this.typeFilter) return false
        if (!q) return true
        return (
          r.payload.toLowerCase().includes(q) ||
          r.proxy.toLowerCase().includes(q) ||
          r.type.toLowerCase().includes(q)
        )
      })
    },
  },

  actions: {
    setSearch(s: string): void { this.search = s },
    setTypeFilter(f: RuleTypeFilter): void { this.typeFilter = f },
    clearFilters(): void {
      this.search = ''
      this.typeFilter = 'all'
    },

    async fetch(): Promise<void> {
      if (this.loading) return
      this.loading = true
      useNoticesStore().clearSource('rules')
      try {
        this.rules = await fetchRules()
        this.lastFetchAt = Date.now()
      } catch (e) {
        useNoticesStore().raiseError('rules', e)
      } finally {
        this.loading = false
      }
    },
  },
})
