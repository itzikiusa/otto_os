// The Tools view's rows (pure; unit/mcpHome.test.ts): the Otto tool catalog
// collapsed to one row per category ("Git · 5 of 10 on"), the tools of the
// open categories under it, a search that opens every category it matches,
// and each tool's state in words. One flat list, so the view can window it.

export interface CatalogTool {
  name: string;
  description: string;
  category?: string | null;
  enabled: boolean;
  mutating: boolean;
  approval_exempt?: boolean;
  auto_approved_by?: unknown[];
}

export type CatalogRow<T extends CatalogTool> =
  | { kind: 'cat'; key: string; cat: string; on: number; total: number; open: boolean; tools: T[] }
  | { kind: 'tool'; key: string; cat: string; tool: T };

/** Categories in catalog order with every tool of each (unfiltered). */
export function byCategory<T extends CatalogTool>(tools: readonly T[]): { cat: string; tools: T[] }[] {
  const order: string[] = [];
  const map = new Map<string, T[]>();
  for (const t of tools) {
    const cat = t.category || 'Other';
    if (!map.has(cat)) {
      map.set(cat, []);
      order.push(cat);
    }
    map.get(cat)!.push(t);
  }
  return order.map((cat) => ({ cat, tools: map.get(cat)! }));
}

/** Header rows for every category with a match; tool rows for the open ones
 *  (a search opens every category it matches). Counts are the category's. */
export function catalogRows<T extends CatalogTool>(tools: readonly T[], open: ReadonlySet<string>, query: string): CatalogRow<T>[] {
  const q = query.trim().toLowerCase();
  const rows: CatalogRow<T>[] = [];
  for (const g of byCategory(tools)) {
    const shown = q ? g.tools.filter((t) => `${t.name} ${t.description}`.toLowerCase().includes(q)) : g.tools;
    if (shown.length === 0) continue;
    const isOpen = !!q || open.has(g.cat);
    rows.push({ kind: 'cat', key: `cat:${g.cat}`, cat: g.cat, on: g.tools.filter((t) => t.enabled).length, total: g.tools.length, open: isOpen, tools: g.tools });
    if (isOpen) for (const t of shown) rows.push({ kind: 'tool', key: `tool:${t.name}`, cat: g.cat, tool: t });
  }
  return rows;
}

/** A tool's state in words: Off · Allowed · Asks first · Runs without asking. */
export function toolState(t: CatalogTool, approvalsOn: boolean): 'Off' | 'Allowed' | 'Asks first' | 'Runs without asking' {
  if (!t.enabled) return 'Off';
  if (!t.mutating) return 'Allowed';
  if (!approvalsOn || t.approval_exempt || (t.auto_approved_by?.length ?? 0) > 0) return 'Runs without asking';
  return 'Asks first';
}

/** The description's first sentence (the rest is behind Details). */
export function firstSentence(text: string | null | undefined): string {
  const s = (text ?? '').trim();
  const m = s.match(/^[\s\S]*?[.!?](?=\s|$)/);
  const first = (m ? m[0] : s).split('\n')[0];
  return first.length > 160 ? `${first.slice(0, 157)}…` : first;
}
