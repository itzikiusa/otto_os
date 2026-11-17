// Variables that keep their place across steps (I2-08). A DAP
// `variables_reference` is only valid for one stop, so the open state is keyed
// by PATH (`Locals/order/items`) and re-applied after every stop by loading the
// open paths breadth-first; values are remembered by path so a row whose value
// changed since the previous stop can be marked.
//
// Import-free (type-only imports), so Node runs it directly in unit tests.

export interface VarLike {
  name: string;
  value: string;
  var_type?: string | null;
  variables_reference: number;
}

export interface ScopeLike {
  name: string;
  variables_reference: number;
}

/** `Locals/order/items` — a separator that can't be confused with a key. */
export function varPath(parent: string, name: string): string {
  return `${parent}␟${name}`;
}

/** Readable form of a path for tests and titles. */
export function pathLabel(path: string): string {
  return path.split('␟').join('/');
}

/** Upper bound of variable requests one stop's re-expansion may make. */
export const REEXPAND_CAP = 20;

/**
 * Re-open every open path after a stop. Breadth-first: a scope's variables
 * load before its children's, and the walk stops after `cap` requests.
 * `known(ref)` returns already-loaded children; `load(ref)` fetches them.
 * Resolves the number of requests made.
 */
export async function reexpandOpenPaths(
  scopes: readonly ScopeLike[],
  openScopes: Readonly<Record<string, boolean>>,
  openPaths: Readonly<Record<string, boolean>>,
  known: (ref: number) => readonly VarLike[] | undefined,
  load: (ref: number) => Promise<readonly VarLike[]>,
  cap = REEXPAND_CAP,
): Promise<number> {
  let requests = 0;
  let level: { path: string; ref: number }[] = scopes
    .filter((s) => openScopes[s.name] && s.variables_reference > 0)
    .map((s) => ({ path: s.name, ref: s.variables_reference }));
  while (level.length > 0) {
    const next: { path: string; ref: number }[] = [];
    for (const node of level) {
      let children = known(node.ref);
      if (!children) {
        if (requests >= cap) return requests;
        requests++;
        children = await load(node.ref);
      }
      for (const v of children) {
        const p = varPath(node.path, v.name);
        if (v.variables_reference > 0 && openPaths[p]) next.push({ path: p, ref: v.variables_reference });
      }
    }
    level = next;
  }
  return requests;
}

/** Values of the visible tree by path (open scopes and open paths only). */
export function collectValues(
  scopes: readonly ScopeLike[],
  openScopes: Readonly<Record<string, boolean>>,
  openPaths: Readonly<Record<string, boolean>>,
  known: (ref: number) => readonly VarLike[] | undefined,
): Record<string, string> {
  const out: Record<string, string> = {};
  const walk = (parent: string, ref: number, depth: number): void => {
    if (depth > 12) return;
    for (const v of known(ref) ?? []) {
      const p = varPath(parent, v.name);
      out[p] = v.value;
      if (v.variables_reference > 0 && openPaths[p]) walk(p, v.variables_reference, depth + 1);
    }
  };
  for (const s of scopes) if (openScopes[s.name] && s.variables_reference > 0) walk(s.name, s.variables_reference, 0);
  return out;
}

/**
 * Remembers the previous stop's values. `observe(stopKey, values)` is called
 * whenever the visible values change; on a new stop key the values seen
 * during the last stop become "previous". A path absent last stop is new, not
 * changed.
 */
export class VarChangeTracker {
  #key = '';
  #seen: Record<string, string> = {};
  previous: Record<string, string> = {};

  observe(stopKey: string, values: Readonly<Record<string, string>>): void {
    if (stopKey !== this.#key) {
      if (this.#key) this.previous = this.#seen;
      this.#key = stopKey;
      this.#seen = {};
    }
    this.#seen = { ...this.#seen, ...values };
  }

  isChanged(path: string, value: string): boolean {
    return Object.prototype.hasOwnProperty.call(this.previous, path) && this.previous[path] !== value;
  }
}

const IDENT = /^[A-Za-z_$][\w$]*$/;

/** Best-effort expression for a nested variable: `order['items'][0].sku`.
 *  Sequences index by number, maps by quoted key, objects by attribute. */
export function copyExpression(chain: readonly VarLike[]): string {
  let expr = '';
  for (let i = 0; i < chain.length; i++) {
    const v = chain[i];
    if (i === 0) { expr = v.name; continue; }
    const parentType = (chain[i - 1].var_type ?? '').toLowerCase();
    const name = v.name;
    if (/^\[.*\]$/.test(name)) expr += name;
    else if (/^\d+$/.test(name)) expr += `[${name}]`;
    else if (/dict|map/.test(parentType) || !IDENT.test(name)) expr += `[${JSON.stringify(name).replace(/^"|"$/g, "'")}]`;
    else expr += `.${name}`;
  }
  return expr;
}
