// Split-layout store — the runes half over the pure tree ops in
// `./splitLayout` (re-exported here so components import ONE module). Owns the
// per-workspace layout tree, the focused leaf, the in-flight pane drag, and
// the tiled-view order; `workspace.svelte.ts` projects `panes`/`focusedPane`/
// `splitAxis` off it (this module never imports the workspace store — no cycle).
export * from './splitLayout';
import { winKey } from '../win';
import type { Id } from '../api/types';
import {
  MAX_PANES,
  LS_PANES,
  LS_TILE_ORDER,
  leaf,
  leaves as leavesOf,
  findLeaf,
  parentOf,
  insertBeside,
  removeLeaf,
  swapLeaves,
  moveLeaf,
  setFrac as setFracOf,
  replaceSession as replaceSessionIn,
  retain as retainIn,
  applyPreset as buildPreset,
  parseLayout,
  reorderIds,
  neighbour,
  type Axis,
  type Leaf,
  type LayoutNode,
  type Preset,
  type Rect,
  type Side,
} from './splitLayout';

/** Restore a persisted split-gutter fraction (0.2–0.8), else the 50/50 default.
 *  Legacy per-WINDOW keys — read only by restore() → fromLegacy as the root
 *  fraction of a v1 payload; never written again. */
function readFrac(key: string): number {
  try {
    const v = Number(localStorage.getItem(winKey(key)));
    return Number.isFinite(v) && v >= 0.2 && v <= 0.8 ? v : 0.5;
  } catch {
    return 0.5;
  }
}

class SplitLayoutStore {
  tree: LayoutNode | null = $state(null);
  focusedKey: string | null = $state(null);
  /** C3a: a pane drag is in progress (drop veils become hit-testable). */
  dragKey: string | null = $state(null);
  tileOrder: Id[] = $state([]);
  private wsKey = 'scratch';
  private persistTimer: ReturnType<typeof setTimeout> | null = null;
  /** False between `bindKey()` and the `restore()` that follows it. In that
   *  window the tree on screen is still the PREVIOUS workspace's (or nothing at
   *  all, on a reload) while `wsKey` already names the workspace being loaded,
   *  so ANY write would overwrite the very payload `restore()` is about to read
   *  — which is exactly how a reload used to collapse a 3-pane split back to
   *  one leaf (the route→store effect opens the hash's session while
   *  `refreshSessions()` is still in flight). Writes are skipped until then;
   *  what the route opened is re-applied by `restore()` (see `pendingSession`). */
  private hydrated = true;
  /** The session `setFocusedSession()` was asked for before hydration — folded
   *  back on top of the restored tree so a route-opened session is not lost. */
  private pendingSession: Id | null = null;

  leaves: Leaf[] = $derived(leavesOf(this.tree));
  /** Projection read by ws.panes — sessions in leaf (reading) order; duplicates possible. */
  panes: Id[] = $derived(this.leaves.map((l) => l.session));
  /** Index of the focused leaf in `leaves`; 0 when focusedKey is null/unknown (never −1). */
  focusedIndex: number = $derived(Math.max(0, this.leaves.findIndex((l) => l.key === this.focusedKey)));
  focusedLeaf: Leaf | null = $derived(this.leaves[this.focusedIndex] ?? null);
  /** Root axis ('col' for a bare leaf/empty) — what ws.splitAxis reports. */
  axis: Axis = $derived(this.tree?.kind === 'split' ? this.tree.axis : 'col');

  /** Pin the persistence key WITHOUT loading anything. `select()` calls this
   *  synchronously before it awaits the session refresh: the route→store
   *  effect can `openSession` during that await, and its persist must land
   *  under the workspace being selected — not under the previous key — or the
   *  `restore()` that follows reads an empty payload and drops the pane. */
  bindKey(wsKey: string): void {
    this.wsKey = wsKey;
    this.hydrated = false;
    this.pendingSession = null;
  }

  /** Load the workspace's layout: v2 as-is, a v1 `{panes, axis}` payload through
   *  fromLegacy with the old window fractions at the root (then re-persisted as
   *  v2 — a one-way migration), nothing → a bare leaf of `fallback`. */
  restore(wsKey: string, valid: (id: Id) => boolean, fallback: Id | null): void {
    this.wsKey = wsKey;
    let raw: string | null = null;
    let tileRaw: string | null = null;
    try {
      raw = localStorage.getItem(winKey(LS_PANES + wsKey));
      tileRaw = localStorage.getItem(winKey(LS_TILE_ORDER + wsKey));
    } catch {
      /* private mode */
    }
    let parsed = { tree: null as LayoutNode | null, focused: null as string | null };
    try {
      parsed = parseLayout(raw, valid, readFrac('otto_split_col_frac'), readFrac('otto_split_row_frac'));
    } catch {
      /* corrupt payload — single-pane default */
    }
    this.tree = parsed.tree ?? (fallback ? leaf(fallback) : null);
    this.focusedKey = parsed.focused ?? this.leaves[0]?.key ?? null;
    let order: Id[] = [];
    try {
      const arr: unknown = tileRaw ? JSON.parse(tileRaw) : [];
      if (Array.isArray(arr)) order = arr.filter((x): x is Id => typeof x === 'string');
    } catch {
      /* corrupt/private mode */
    }
    this.tileOrder = order;
    // Writes are live again — the payload has been read.
    const pending = this.pendingSession;
    this.pendingSession = null;
    this.hydrated = true;
    // A session the route opened while the sessions were loading rides on top
    // of the restored tree: its own leaf takes focus when it already has one,
    // otherwise it lands in the focused pane (plain `openSession` semantics).
    if (pending != null && valid(pending)) this.setFocusedSession(pending);
    // v1 (or absent) payload with something on screen → write it back as v2 now.
    let isV2 = false;
    try {
      isV2 = raw != null && (JSON.parse(raw) as { v?: unknown } | null)?.v === 2;
    } catch {
      /* not v2 */
    }
    if (!isV2 && this.tree) this.persist();
  }

  persist(): void {
    if (this.persistTimer) {
      clearTimeout(this.persistTimer);
      this.persistTimer = null;
    }
    if (!this.hydrated) return;
    try {
      localStorage.setItem(
        winKey(LS_PANES + this.wsKey),
        JSON.stringify({ v: 2, tree: this.tree, focused: this.focusedKey }),
      );
    } catch {
      /* private mode */
    }
  }

  persistTileOrder(): void {
    try {
      localStorage.setItem(winKey(LS_TILE_ORDER + this.wsKey), JSON.stringify(this.tileOrder));
    } catch {
      /* private mode */
    }
  }

  /** Forget the tree (removeWorkspace). Does NOT persist. */
  reset(): void {
    this.tree = null;
    this.focusedKey = null;
    this.tileOrder = [];
  }

  focusIndex(i: number): void {
    const l = this.leaves[i];
    if (l) this.focusedKey = l.key;
  }

  focusKey(key: string): void {
    if (findLeaf(this.tree, key)) this.focusedKey = key;
  }

  /** Re-point focus at the first leaf when the focused one is gone. */
  private fixFocus(): void {
    if (!this.focusedKey || !findLeaf(this.tree, this.focusedKey)) this.focusedKey = this.leaves[0]?.key ?? null;
  }

  /** openSession: an empty tree becomes a leaf of `id`; a session ALREADY on
   *  screen just takes focus in the pane that holds it; otherwise the focused
   *  leaf's session is replaced (its key kept).
   *
   *  The "already on screen" arm matters beyond the click that asks for it:
   *  `closeTab` routes to the neighbour it fell back to, and the route→store
   *  effect replays that as an open — which used to stamp the fallback over
   *  whatever the (re-focused) first pane was showing, duplicating one session
   *  across two panes and dropping another. */
  setFocusedSession(id: Id): void {
    if (!this.hydrated) this.pendingSession = id;
    const on = this.leaves.find((l) => l.session === id);
    if (on) {
      this.focusedKey = on.key;
      this.persist();
      return;
    }
    if (!this.tree) {
      const l = leaf(id);
      this.tree = l;
      this.focusedKey = l.key;
    } else {
      const key = this.focusedLeaf?.key ?? this.leaves[0]?.key;
      if (!key) return;
      const go = (n: LayoutNode): LayoutNode =>
        n.kind === 'leaf' ? (n.key === key ? { ...n, session: id } : n) : { ...n, a: go(n.a), b: go(n.b) };
      this.tree = go(this.tree);
      this.focusedKey = key;
    }
    this.persist();
  }

  /** ws.split(axis): clone the focused session into a new leaf beside it
   *  (right for col, below for row). False when empty or at MAX_PANES. */
  splitFocused(axis: Axis): boolean {
    const cur = this.focusedLeaf;
    if (!this.tree || !cur || this.leaves.length >= MAX_PANES) return false;
    const dup = leaf(cur.session);
    this.tree = insertBeside(this.tree, cur.key, dup, axis === 'col' ? 'right' : 'down');
    this.focusedKey = dup.key;
    this.persist();
    return true;
  }

  /** openInSplit / tileSessions. (1) already on screen → focus it (when asked);
   *  (2) empty → sole leaf; (3) at MAX_PANES → the focused leaf is REUSED and
   *  false is returned (the caller toasts; the pane still appears); (4) insert
   *  beside the focused leaf on the axis ORTHOGONAL to its parent split so
   *  repeated "open beside" builds a grid, not a 15-wide strip. */
  addBeside(id: Id, opts?: { focus?: boolean }): boolean {
    const focus = opts?.focus ?? true;
    const existing = this.leaves.find((l) => l.session === id);
    if (existing) {
      if (focus) this.focusKey(existing.key);
      return true;
    }
    if (!this.tree) {
      const l = leaf(id);
      this.tree = l;
      this.focusedKey = l.key;
      this.persist();
      return true;
    }
    if (this.leaves.length >= MAX_PANES) {
      this.setFocusedSession(id);
      return false;
    }
    const cur = this.focusedLeaf ?? this.leaves[0];
    const side: Side = parentOf(this.tree, cur.key)?.axis === 'col' ? 'down' : 'right';
    const fresh = leaf(id);
    this.tree = insertBeside(this.tree, cur.key, fresh, side);
    if (focus) this.focusedKey = fresh.key;
    this.persist();
    return true;
  }

  removeAt(i: number): void {
    const l = this.leaves[i];
    if (l) this.removeKey(l.key);
  }

  /** Close ONE pane: its split collapses into the sibling; focus moves to the
   *  sibling's first leaf when the closed pane held it (closeTab's neighbour rule). */
  removeKey(key: string): void {
    if (this.leaves.length <= 1 || !findLeaf(this.tree, key)) return;
    const parent = parentOf(this.tree, key);
    const sibling = parent ? (parent.a.key === key ? parent.b : parent.a) : null;
    this.tree = removeLeaf(this.tree, key);
    if (this.focusedKey === key) this.focusedKey = leavesOf(sibling)[0]?.key ?? this.leaves[0]?.key ?? null;
    this.fixFocus();
    this.persist();
  }

  /** closeTab: every leaf of `from` becomes `to` (dedupe: a replaced leaf whose
   *  target is already on screen vanishes instead); an emptied tree becomes a
   *  bare leaf of `to`. */
  replaceSession(from: Id, to: Id | null): void {
    this.tree = replaceSessionIn(this.tree, from, to);
    if (!this.tree && to) this.tree = leaf(to);
    this.fixFocus();
    this.persist();
  }

  /** reconcileTabs: drop leaves failing `keep`. True when something changed.
   *
   *  Deliberately does NOT persist. A workspace switch reconciles the OLD tree
   *  against the NEW workspace's sessions BEFORE restore() learns the new key,
   *  so a write here would land on the workspace we are leaving and wipe the
   *  layout we are about to come back to. Nothing is lost by skipping it:
   *  restore() re-filters every persisted leaf through `valid` (the tab list,
   *  persisted separately), so a stale leaf can never come back. */
  retain(keep: (id: Id) => boolean): boolean {
    const next = retainIn(this.tree, keep);
    if (next === this.tree) return false;
    this.tree = next;
    this.fixFocus();
    return true;
  }

  /** Write a debounced {@link setFrac} out NOW. Nothing pending ⇒ no-op.
   *  Called on `pagehide`: a reload inside the 150 ms window would otherwise
   *  drop the fraction the user just dragged (and restore the pre-drag tree). */
  flush(): void {
    if (this.persistTimer) this.persist();
  }

  /** Gutter drag calls this per pointermove — persistence is debounced. */
  setFrac(splitKey: string, frac: number): void {
    if (!this.tree) return;
    this.tree = setFracOf(this.tree, splitKey, frac);
    if (this.persistTimer) clearTimeout(this.persistTimer);
    this.persistTimer = setTimeout(() => {
      this.persistTimer = null;
      this.persist();
    }, 150);
  }

  /** Rebuild the same sessions (leaf order preserved) in a canonical shape;
   *  focus follows the focused SESSION. */
  applyPreset(p: Preset): void {
    const ids = this.panes;
    const focusedSession = this.focusedLeaf?.session ?? null;
    this.tree = buildPreset(ids, p);
    const target = focusedSession != null ? this.leaves.find((l) => l.session === focusedSession) : undefined;
    this.focusedKey = (target ?? this.leaves[0])?.key ?? null;
    this.persist();
  }

  /** Swap the SESSIONS of two slots; the focused SLOT keeps focus. */
  swap(k1: string, k2: string): void {
    if (!this.tree) return;
    this.tree = swapLeaves(this.tree, k1, k2);
    this.persist();
  }

  /** Move a leaf beside another; focus follows the moved LEAF. */
  move(key: string, targetKey: string, side: Side): void {
    if (!this.tree) return;
    this.tree = moveLeaf(this.tree, key, targetKey, side);
    this.focusKey(key);
    this.persist();
  }

  /** Keyboard move: the geometric neighbour on `side`, else a silent no-op
   *  (like every tiling WM — no toast). */
  moveFocused(side: Side, rects: Map<string, Rect>): void {
    const key = this.focusedLeaf?.key;
    if (!key) return;
    const n = neighbour(rects, key, side);
    if (n) this.move(key, n, side);
  }

  /** Swap the focused slot's session with the next one in reading order (wraps). */
  swapWithNext(): void {
    const L = this.leaves;
    if (L.length <= 1) return;
    const i = this.focusedIndex;
    const j = (i + 1) % L.length;
    this.swap(L[i].key, L[j].key);
  }

  setTileOrder(order: Id[]): void {
    this.tileOrder = order;
    this.persistTileOrder();
  }

  /** reorderIds over the FULL ordered id list the caller passes (so unknown
   *  ids get materialised on the first drag). */
  moveTile(ordered: Id[], fromId: Id, toId: Id): void {
    this.setTileOrder(reorderIds(ordered, fromId, toId));
  }
}

export const layout = new SplitLayoutStore();

// The only debounced write in this store is the gutter drag; `pagehide` covers
// reload / navigation / tab close, which all beat a 150 ms timer easily.
if (typeof window !== 'undefined') {
  window.addEventListener('pagehide', () => layout.flush());
}
