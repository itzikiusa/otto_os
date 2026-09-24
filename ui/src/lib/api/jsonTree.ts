// Search for the response JSON tree: which nodes match a query (by key or by
// primitive value) and which containers must stay expanded to reveal them.
// Paths use JSONPath-ish notation (`$.data[0].id`) so a row can offer
// "Copy path". Pure + framework-free (see unit/jsonTree.test.ts).

export function childPath(parent: string, key: string | number): string {
  if (typeof key === 'number') return `${parent}[${key}]`;
  return /^[A-Za-z_$][\w$]*$/.test(key) ? `${parent}.${key}` : `${parent}[${JSON.stringify(key)}]`;
}

export interface TreeSearch {
  /** Paths whose key or primitive value contains the query. */
  hits: Set<string>;
  /** Container paths with a hit somewhere below (auto-expanded). */
  open: Set<string>;
}

/** Walk `root` (bounded: at most `limit` nodes) collecting query matches. */
export function searchTree(root: unknown, query: string, limit = 20_000): TreeSearch {
  const hits = new Set<string>();
  const open = new Set<string>();
  const q = query.trim().toLowerCase();
  if (!q) return { hits, open };
  let seen = 0;
  const visit = (value: unknown, path: string, key: string | number | null): boolean => {
    if (++seen > limit) return false;
    let found = key !== null && String(key).toLowerCase().includes(q);
    if (value !== null && typeof value === 'object') {
      let below = false;
      const entries = Array.isArray(value) ? value.map((v, i) => [i, v] as const) : Object.entries(value);
      for (const [k, v] of entries) {
        if (visit(v, childPath(path, k), k)) below = true;
      }
      if (below) open.add(path);
      if (found) hits.add(path);
      return found || below;
    }
    if (String(value).toLowerCase().includes(q)) found = true;
    if (found) hits.add(path);
    return found;
  };
  visit(root, '$', null);
  return { hits, open };
}

/** A one-line preview of a value for collapsed rows. */
export function preview(value: unknown): string {
  if (Array.isArray(value)) return `[${value.length} item${value.length === 1 ? '' : 's'}]`;
  if (value !== null && typeof value === 'object') {
    const n = Object.keys(value).length;
    return `{${n} key${n === 1 ? '' : 's'}}`;
  }
  return typeof value === 'string' ? JSON.stringify(value) : String(value);
}
