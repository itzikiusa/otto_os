// Pure tab-list transitions for the PR workspace (unit/prTabs.test.ts).

export interface PrTabs {
  /** Open PR numbers, in tab order. */
  open: number[];
  /** The PR in front (always one of `open`, or null when none is open). */
  active: number | null;
}

/**
 * Open `ns` (already-open ones just stay where they are). A foreground open
 * brings `ns[0]` to the front; a background open keeps the current front tab
 * (or fronts `ns[0]` when nothing is open). Past `max`, the oldest tabs that
 * are neither in front nor just opened are dropped.
 */
export function prTabsAfterOpen(t: PrTabs, ns: number[], background: boolean, max: number): PrTabs {
  const open = [...t.open];
  for (const n of ns) if (!open.includes(n)) open.push(n);
  const active = background && t.active !== null ? t.active : (ns[0] ?? t.active);
  const keep = new Set<number>([...(active !== null ? [active] : []), ...ns]);
  while (open.length > max) {
    const i = open.findIndex((n) => !keep.has(n));
    if (i < 0) break;
    open.splice(i, 1);
  }
  return { open, active };
}

/** Close `n`; the front moves to its right-hand neighbour, else the left. */
export function prTabsAfterClose(t: PrTabs, n: number): PrTabs {
  const i = t.open.indexOf(n);
  if (i < 0) return t;
  const open = t.open.filter((x) => x !== n);
  const active = t.active === n ? (open[Math.min(i, open.length - 1)] ?? null) : t.active;
  return { open, active };
}
