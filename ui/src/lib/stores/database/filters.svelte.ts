// Database Explorer store — quick-filter chips: the chips that own the active
// query tab's WHERE clause. The DatabaseStore façade (../database.svelte.ts)
// owns an instance and delegates `filters` and the chip methods here; the host
// supplies the active tab, the connection's engine (identifier quoting and
// literals follow it) and the statement / persistence hooks.

import type { ConnectionKind } from '../../api/types';
import { condToSql, parseFilterValText, toFilterVal, type FilterCond } from '../../database/filter-chips';
import type { QueryTab } from './model';
import { extractWhereBody, rewriteWhere } from './statement';

/** What the filters store reads from (and asks of) its DatabaseStore host. */
export interface FiltersHost {
  activeTab(): QueryTab | null;
  /** The selected connection's kind — `condToSql` quotes and escapes for it. */
  engine(): ConnectionKind | null;
  /** Replace the active tab's statement (the façade's `setStatement`). */
  setStatement(statement: string): void;
  persistTabs(): void;
}

export class DbFiltersStore {
  private readonly host: FiltersHost;
  constructor(host: FiltersHost) {
    this.host = host;
  }

  // ── Quick filters (chips that own the active tab's WHERE clause) ───────────
  // Chips accumulate without running the query — the user adds more, then runs.
  // Repeated equals on a column collapse into IN / NOT IN; include vs exclude
  // are separate directions. Applying rewrites the statement's WHERE in place.

  /** Quick-filter chips for the active tab. */
  get filters(): FilterCond[] {
    return this.host.activeTab()?.filters ?? [];
  }

  /** On the first chip, fold any hand-written WHERE into a removable raw chip
   * so chips can safely own the WHERE from then on. */
  private absorbExistingWhere(t: QueryTab): void {
    if (t.filters.length > 0) return;
    const existing = extractWhereBody(t.statement);
    if (existing && existing.trim()) t.filters.push({ kind: 'raw', text: existing.trim() });
  }

  /** Add a value-based filter from a cell (include = equals, exclude = not). */
  addQuickFilter(column: string, value: unknown, mode: 'include' | 'exclude'): void {
    const t = this.host.activeTab();
    if (!t || !column) return;
    this.absorbExistingWhere(t);
    const op = mode === 'include' ? 'in' : 'not_in';
    const fv = toFilterVal(value);
    let cond = t.filters.find(
      (c): c is Extract<FilterCond, { kind: 'col' }> =>
        c.kind === 'col' && c.column === column && c.op === op,
    );
    if (!cond) {
      cond = { kind: 'col', column, op, values: [] };
      t.filters.push(cond);
    }
    if (!cond.values.some((v) => v.raw === fv.raw && v.isNull === fv.isNull)) cond.values.push(fv);
    this.applyFilters();
  }

  /** Add an empty (value-less) filter on a column, to be filled in the bar. */
  addColumnFilter(column: string): void {
    const t = this.host.activeTab();
    if (!t || !column) return;
    this.absorbExistingWhere(t);
    if (!t.filters.some((c) => c.kind === 'col' && c.column === column)) {
      t.filters.push({ kind: 'col', column, op: 'in', values: [] });
    }
    this.applyFilters();
  }

  /** Add a typed value to an existing column chip. */
  addFilterValue(condIndex: number, text: string): void {
    const t = this.host.activeTab();
    const c = t?.filters[condIndex];
    if (!t || !c || c.kind !== 'col' || !text.trim()) return;
    const fv = parseFilterValText(text);
    if (!c.values.some((v) => v.raw === fv.raw && v.isNull === fv.isNull)) c.values.push(fv);
    this.applyFilters();
  }

  removeFilterValue(condIndex: number, valIndex: number): void {
    const t = this.host.activeTab();
    const c = t?.filters[condIndex];
    if (!t || !c || c.kind !== 'col') return;
    c.values.splice(valIndex, 1);
    this.applyFilters();
  }

  removeFilterCond(condIndex: number): void {
    const t = this.host.activeTab();
    if (!t) return;
    t.filters.splice(condIndex, 1);
    this.applyFilters();
  }

  /** Flip a column chip between include (IN) and exclude (NOT IN). */
  toggleFilterMode(condIndex: number): void {
    const t = this.host.activeTab();
    const c = t?.filters[condIndex];
    if (!t || !c || c.kind !== 'col') return;
    c.op = c.op === 'in' ? 'not_in' : 'in';
    this.applyFilters();
  }

  clearFilters(): void {
    const t = this.host.activeTab();
    if (!t) return;
    t.filters = [];
    this.applyFilters();
  }

  /** Take over the WHERE from a cell "Query by value" / "Add to query" action:
   * set the rewritten statement AND drop any quick-filter chips — otherwise the
   * chip bar would desync and the next chip op (`applyFilters`) would silently
   * rebuild the WHERE from the stale chips, discarding the spliced condition.
   * Clears the chips WITHOUT `applyFilters` (which would strip the new WHERE).
   * Does NOT run. */
  setStatementFromCellFilter(statement: string): void {
    const t = this.host.activeTab();
    if (!t) return;
    t.filters = [];
    this.host.setStatement(statement);
  }

  /** Rewrite the active statement's WHERE from the chips (does NOT run). */
  private applyFilters(): void {
    const t = this.host.activeTab();
    if (!t) return;
    // Identifier quoting, string escaping and boolean literals follow the
    // connection's engine (`sql-dialect.ts`) — Postgres gets `"col"`, no
    // backslash doubling and TRUE/FALSE.
    const engine = this.host.engine();
    const body = t.filters
      .map((c) => condToSql(c, engine))
      .filter((s) => s.trim())
      .join(' AND ');
    t.statement = rewriteWhere(t.statement, body);
    this.host.persistTabs();
  }
}
