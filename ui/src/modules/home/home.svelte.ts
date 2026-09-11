// Home dashboard state: up to MAX_VIEWS views, each holding up to MAX_BOXES
// boxes on a 12-column grid, with an optional 30-second auto-rotation between
// views. Persisted per device in localStorage (same idiom as the sidebar
// personalization in ui.svelte.ts) — the layout is a personal preference, not
// workspace data, so it never goes through the daemon.

import { HOME_KINDS, isHomeKind, kindDef, type HomeBoxKind } from './kinds';
import type { Feature } from '../../lib/api/types';

export const MAX_VIEWS = 4;
export const MAX_BOXES = 8;
export const COLS = 12;
/** Smallest footprint a box can be resized to (columns × rows). */
export const MIN_W = 3;
export const MIN_H = 2;
export const MAX_H = 12;
/** Grid row unit + gap, in px — HomePage's grid uses the same numbers. */
export const ROW_PX = 72;
export const GAP_PX = 12;
/** Auto-rotation period. */
export const ROTATE_MS = 30_000;

export interface HomeBox {
  id: string;
  kind: HomeBoxKind;
  /** Column span (MIN_W..COLS). */
  w: number;
  /** Row span (MIN_H..MAX_H). */
  h: number;
  /** Kind-specific settings (dashboard id, window, days…). */
  config: Record<string, unknown>;
}

export interface HomeView {
  id: string;
  name: string;
  boxes: HomeBox[];
}

const LS = {
  views: 'otto_home_views',
  active: 'otto_home_active',
  rotate: 'otto_home_rotate',
};

function lsGet(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}
function lsSet(key: string, val: string): void {
  try {
    localStorage.setItem(key, val);
  } catch {
    /* private mode */
  }
}

function uid(): string {
  return Math.random().toString(36).slice(2, 10) + Date.now().toString(36).slice(-4);
}

function clamp(n: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, n));
}

/** Parse + sanitize a persisted layout; anything malformed is dropped rather
 *  than crashing the page (a stale kind from an older build, hand-edited JSON…). */
function loadViews(): HomeView[] {
  const raw = lsGet(LS.views);
  if (!raw) return [];
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    const views: HomeView[] = [];
    for (const v of parsed.slice(0, MAX_VIEWS)) {
      if (!v || typeof v !== 'object') continue;
      const o = v as Partial<HomeView>;
      const boxes: HomeBox[] = [];
      for (const b of Array.isArray(o.boxes) ? o.boxes.slice(0, MAX_BOXES) : []) {
        if (!b || typeof b !== 'object') continue;
        const bo = b as Partial<HomeBox>;
        if (!isHomeKind(bo.kind)) continue;
        const def = kindDef(bo.kind);
        boxes.push({
          id: typeof bo.id === 'string' && bo.id ? bo.id : uid(),
          kind: bo.kind,
          w: clamp(Number(bo.w) || def.w, MIN_W, COLS),
          h: clamp(Number(bo.h) || def.h, MIN_H, MAX_H),
          config: bo.config && typeof bo.config === 'object' ? { ...bo.config } : {},
        });
      }
      views.push({
        id: typeof o.id === 'string' && o.id ? o.id : uid(),
        name: typeof o.name === 'string' && o.name.trim() ? o.name : `View ${views.length + 1}`,
        boxes,
      });
    }
    return views;
  } catch {
    return [];
  }
}

class HomeStore {
  views: HomeView[] = $state(loadViews());
  activeIndex = $state(clamp(Number(lsGet(LS.active)) || 0, 0, MAX_VIEWS - 1));
  /** Auto-slide between views every ROTATE_MS. Default ON (the ask). */
  autoRotate = $state(lsGet(LS.rotate) !== '0');
  /** Box currently zoomed to fill the page (null = normal grid). */
  zoomedId: string | null = $state(null);
  /** Rotation is suspended while the user is mid-gesture (resize/drag) or a
   *  sheet is open — bumping a view out from under a drag would be hostile. */
  interacting = $state(false);
  /** Direction of the last view change, for the slide transition. */
  slideDir: 1 | -1 = $state(1);
  /** Bumped on every rotation-timer reset so the progress bar can restart. */
  rotationEpoch = $state(0);

  active: HomeView | null = $derived(this.views[this.activeIndex] ?? this.views[0] ?? null);

  zoomed: HomeBox | null = $derived.by(() => {
    const id = this.zoomedId;
    if (!id) return null;
    for (const v of this.views) {
      const b = v.boxes.find((x) => x.id === id);
      if (b) return b;
    }
    return null;
  });

  private persist(): void {
    lsSet(LS.views, JSON.stringify(this.views));
    lsSet(LS.active, String(this.activeIndex));
  }

  /** First visit: seed one view with the kinds this user can actually see so the
   *  page is never a blank canvas. No-op when anything is already persisted. */
  ensureDefault(can: (f: Feature) => boolean): void {
    if (this.views.length > 0) return;
    const boxes: HomeBox[] = [];
    for (const kind of ['sessions', 'mission-control', 'k8s', 'db-dashboard'] as HomeBoxKind[]) {
      const def = kindDef(kind);
      if (can(def.feature)) boxes.push({ id: uid(), kind, w: def.w, h: def.h, config: {} });
    }
    this.views = [{ id: uid(), name: 'Overview', boxes }];
    this.activeIndex = 0;
    this.persist();
  }

  // ── Views ────────────────────────────────────────────────────────────────

  addView(name?: string): HomeView | null {
    if (this.views.length >= MAX_VIEWS) return null;
    const v: HomeView = { id: uid(), name: name?.trim() || `View ${this.views.length + 1}`, boxes: [] };
    this.views = [...this.views, v];
    this.goTo(this.views.length - 1);
    this.persist();
    return v;
  }

  renameView(id: string, name: string): void {
    const n = name.trim();
    if (!n) return;
    this.views = this.views.map((v) => (v.id === id ? { ...v, name: n } : v));
    this.persist();
  }

  removeView(id: string): void {
    const idx = this.views.findIndex((v) => v.id === id);
    if (idx < 0) return;
    this.views = this.views.filter((v) => v.id !== id);
    if (this.activeIndex >= this.views.length) this.activeIndex = Math.max(0, this.views.length - 1);
    this.persist();
  }

  goTo(i: number): void {
    if (this.views.length === 0) return;
    const n = ((i % this.views.length) + this.views.length) % this.views.length;
    if (n === this.activeIndex) return;
    this.slideDir = n > this.activeIndex ? 1 : -1;
    this.activeIndex = n;
    this.zoomedId = null;
    this.rotationEpoch += 1;
    lsSet(LS.active, String(n));
  }
  next(): void {
    if (this.views.length < 2) return;
    this.slideDir = 1;
    this.activeIndex = (this.activeIndex + 1) % this.views.length;
    this.zoomedId = null;
    this.rotationEpoch += 1;
    lsSet(LS.active, String(this.activeIndex));
  }
  prev(): void {
    if (this.views.length < 2) return;
    this.slideDir = -1;
    this.activeIndex = (this.activeIndex - 1 + this.views.length) % this.views.length;
    this.zoomedId = null;
    this.rotationEpoch += 1;
    lsSet(LS.active, String(this.activeIndex));
  }

  setAutoRotate(on: boolean): void {
    this.autoRotate = on;
    this.rotationEpoch += 1;
    lsSet(LS.rotate, on ? '1' : '0');
  }

  /** Whether the timer should be ticking right now. */
  get rotating(): boolean {
    return this.autoRotate && this.views.length > 1 && this.zoomedId === null && !this.interacting;
  }

  // ── Boxes ────────────────────────────────────────────────────────────────

  private patchView(viewId: string, f: (v: HomeView) => HomeView): void {
    this.views = this.views.map((v) => (v.id === viewId ? f(v) : v));
    this.persist();
  }

  addBox(viewId: string, kind: HomeBoxKind, config: Record<string, unknown> = {}): HomeBox | null {
    const v = this.views.find((x) => x.id === viewId);
    if (!v || v.boxes.length >= MAX_BOXES) return null;
    const def = kindDef(kind);
    const box: HomeBox = { id: uid(), kind, w: def.w, h: def.h, config };
    this.patchView(viewId, (x) => ({ ...x, boxes: [...x.boxes, box] }));
    return box;
  }

  removeBox(viewId: string, boxId: string): void {
    if (this.zoomedId === boxId) this.zoomedId = null;
    this.patchView(viewId, (x) => ({ ...x, boxes: x.boxes.filter((b) => b.id !== boxId) }));
  }

  resizeBox(viewId: string, boxId: string, w: number, h: number): void {
    const cw = clamp(Math.round(w), MIN_W, COLS);
    const ch = clamp(Math.round(h), MIN_H, MAX_H);
    const v = this.views.find((x) => x.id === viewId);
    const b = v?.boxes.find((x) => x.id === boxId);
    if (!b || (b.w === cw && b.h === ch)) return;
    this.patchView(viewId, (x) => ({
      ...x,
      boxes: x.boxes.map((bb) => (bb.id === boxId ? { ...bb, w: cw, h: ch } : bb)),
    }));
  }

  /** Move a box to `targetIndex` within its view (drag-reorder / menu). */
  moveBox(viewId: string, boxId: string, targetIndex: number): void {
    this.patchView(viewId, (x) => {
      const from = x.boxes.findIndex((b) => b.id === boxId);
      if (from < 0) return x;
      const boxes = [...x.boxes];
      const [b] = boxes.splice(from, 1);
      boxes.splice(clamp(targetIndex, 0, boxes.length), 0, b);
      return { ...x, boxes };
    });
  }

  /** Move a box to another view (appended). Refuses when the target is full. */
  moveBoxToView(fromViewId: string, boxId: string, toViewId: string): boolean {
    const from = this.views.find((v) => v.id === fromViewId);
    const to = this.views.find((v) => v.id === toViewId);
    const box = from?.boxes.find((b) => b.id === boxId);
    if (!from || !to || !box || to.boxes.length >= MAX_BOXES) return false;
    this.views = this.views.map((v) => {
      if (v.id === fromViewId) return { ...v, boxes: v.boxes.filter((b) => b.id !== boxId) };
      if (v.id === toViewId) return { ...v, boxes: [...v.boxes, box] };
      return v;
    });
    this.persist();
    return true;
  }

  updateBoxConfig(viewId: string, boxId: string, patch: Record<string, unknown>): void {
    this.patchView(viewId, (x) => ({
      ...x,
      boxes: x.boxes.map((b) => (b.id === boxId ? { ...b, config: { ...b.config, ...patch } } : b)),
    }));
  }

  toggleZoom(boxId: string): void {
    this.zoomedId = this.zoomedId === boxId ? null : boxId;
  }

  /** Kinds this user may add (RBAC view gate). */
  availableKinds(can: (f: Feature) => boolean) {
    return HOME_KINDS.filter((k) => can(k.feature));
  }
}

export const home = new HomeStore();
