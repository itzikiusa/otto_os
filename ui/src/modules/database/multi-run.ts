// Pure helpers for the "Run on…" multi-target dialog (MultiRunDialog.svelte):
// value-list parsing, target building, the typed-confirmation phrase and the
// summary line. No runtime imports — unit-tested in ui/unit/dbMultiRun.test.ts.

import type {
  DbClusterMode,
  DbMultiRunPlan,
  DbMultiRunSummary,
  DbMultiRunTarget,
  SchemaNode,
} from '../../lib/api/types';

/**
 * Split a parameter's value list. One value per LINE when the text has a
 * newline (so values may contain commas), else comma-separated. Values are
 * trimmed and blanks dropped — `1, 2,3` and `1\n2\n3` both give three values.
 */
export function parseValues(text: string): string[] {
  const parts = text.includes('\n') ? text.split('\n') : text.split(',');
  return parts.map((v) => v.trim()).filter((v) => v.length > 0);
}

/** The `node` to scope a target to from a schema-root entry: the database /
 *  schema name, or a Redis keyspace path (`kdb:N`). `null` = not a scope. */
export function nodeOf(n: SchemaNode): string | null {
  if (n.kind === 'database' || n.kind === 'schema') return n.label;
  if (n.kind === 'keyspace') return n.id;
  return null;
}

/** The scopes a connection's schema root offers (`value` → `label`). */
export function scopeOptions(root: SchemaNode[]): { value: string; label: string }[] {
  const out: { value: string; label: string }[] = [];
  for (const n of root) {
    const v = nodeOf(n);
    if (v != null && !out.some((o) => o.value === v)) out.push({ value: v, label: n.label });
  }
  return out;
}

/** Sentinel for "the connection's default database" in a pick list. */
export const DEFAULT_SCOPE = '';

/**
 * Targets from the picker state: connection id → picked scopes (in the order
 * the connections were picked). An empty scope list targets the connection's
 * default database once. ClickHouse cluster choice applies to every target;
 * per-target overrides (from the preview) win.
 */
export function buildTargets(
  order: string[],
  picks: Record<string, string[]>,
  cluster: { mode: DbClusterMode; name?: string },
  overrides: Record<string, { mode: DbClusterMode; name?: string }> = {},
): DbMultiRunTarget[] {
  const out: DbMultiRunTarget[] = [];
  for (const id of order) {
    const scopes = picks[id];
    if (!scopes) continue;
    const list = scopes.length > 0 ? scopes : [DEFAULT_SCOPE];
    for (const s of list) {
      const key = targetKey(id, s || null);
      const c = overrides[key] ?? cluster;
      out.push({
        connection_id: id,
        node: s || null,
        cluster_mode: c.mode,
        ...(c.mode === 'custom' ? { cluster_name: (c.name ?? '').trim() } : {}),
      });
    }
  }
  return out;
}

/** Stable key of a target (connection + scope) for per-target overrides. */
export function targetKey(connectionId: string, node: string | null | undefined): string {
  return `${connectionId}\u0000${node ?? ''}`;
}

/**
 * What the person must type to confirm the guarded writes of a plan: the
 * connection's name when every guarded write goes to ONE connection (the
 * single-run convention), else `RUN <n>` — the number of guarded runs, so the
 * size of what is about to happen is spelled out.
 */
export function confirmPhrase(plan: DbMultiRunPlan): string {
  const guarded = plan.runs.filter((r) => r.needs_confirm);
  const conns = [...new Set(guarded.map((r) => plan.targets[r.target]?.connection_name ?? ''))];
  if (conns.length === 1 && conns[0]) return conns[0];
  return `RUN ${guarded.length}`;
}

export function phraseMatches(typed: string, phrase: string): boolean {
  return typed.trim().toLowerCase() === phrase.trim().toLowerCase();
}

/** `3 ok · 1 failed · 2 pending` — zero counts omitted; `0 runs` when empty. */
export function summaryText(s: DbMultiRunSummary): string {
  const parts: string[] = [];
  if (s.ok) parts.push(`${s.ok} ok`);
  if (s.failed) parts.push(`${s.failed} failed`);
  if (s.running) parts.push(`${s.running} running`);
  if (s.pending) parts.push(`${s.pending} pending`);
  if (s.skipped) parts.push(`${s.skipped} skipped`);
  if (s.cancelled) parts.push(`${s.cancelled} canceled`);
  return parts.length ? parts.join(' · ') : `${s.total} runs`;
}

/** Human cluster line for a planned ClickHouse target. */
export function clusterLine(c: NonNullable<DbMultiRunPlan['targets'][number]['cluster']>): string {
  if (c.applied) {
    const why =
      c.mode === 'custom'
        ? 'your choice'
        : c.source === 'macro'
          ? 'from the {cluster} macro'
          : 'from system.clusters';
    return `ON CLUSTER ${c.applied} (${why})`;
  }
  switch (c.source) {
    case 'not_needed':
      return c.mode === 'off' ? 'ON CLUSTER off' : 'No DDL — nothing to rewrite';
    case 'not_cluster':
      return 'Single node — no ON CLUSTER';
    case 'replicated_database':
      return `${c.database_engine ?? 'Replicated'} database — DDL replicates itself`;
    case 'ambiguous':
      return 'Several clusters — pick one';
    case 'probe_failed':
      return 'Cluster check failed — no ON CLUSTER';
    default:
      return 'No ON CLUSTER';
  }
}
