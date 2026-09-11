// Sidebar session-order store — the runes half over the pure helpers in
// `./sessionOrder` (re-exported here). "Recent" renders the store's list
// unchanged; the first drag implies "Manual" and seeds the order from the list
// AS RENDERED so nothing jumps. Persisted per workspace (winKey-namespaced).
export * from './sessionOrder';
import { winKey } from '../win';
import type { Id } from '../api/types';
import { LS_SESSION_ORDER, reorder, type OrderMode } from './sessionOrder';

class SessionOrderStore {
  mode: OrderMode = $state('recent');
  order: Id[] = $state([]);
  private wsKey = 'scratch';

  /** Accepts only `{ mode ∈ {'recent','manual'}, order: string[] }`; anything else → defaults. */
  load(wsKey: string): void {
    this.wsKey = wsKey;
    let mode: OrderMode = 'recent';
    let order: Id[] = [];
    try {
      const raw = localStorage.getItem(winKey(LS_SESSION_ORDER + wsKey));
      const o = raw ? (JSON.parse(raw) as { mode?: unknown; order?: unknown } | null) : null;
      if (o && (o.mode === 'recent' || o.mode === 'manual') && Array.isArray(o.order)) {
        mode = o.mode;
        order = o.order.filter((x): x is Id => typeof x === 'string');
      }
    } catch {
      /* corrupt/private mode — defaults */
    }
    this.mode = mode;
    this.order = order;
  }

  /** First drag implies Manual: order = displayed (the list AS RENDERED, so
   *  nothing jumps), mode = 'manual', then the drop is applied. Re-seeding from
   *  the rendered list on EVERY drag also materialises sessions that were still
   *  "new on top" (rows only drag with no filter active, so it is the full list). */
  dragTo(displayed: Id[], fromId: Id, toId: Id): void {
    const base = displayed.length > 0 ? displayed : this.order;
    this.mode = 'manual';
    this.order = reorder(base, fromId, toId);
    this.persist();
  }

  /** 'manual' with an empty order seeds from `displayed`; 'recent' keeps
   *  `order` (so switching back and forth is lossless). */
  setMode(m: OrderMode, displayed: Id[] = []): void {
    this.mode = m;
    if (m === 'manual' && this.order.length === 0) this.order = [...displayed];
    this.persist();
  }

  reset(): void {
    this.mode = 'recent';
    this.order = [];
    this.persist();
  }

  private persist(): void {
    try {
      localStorage.setItem(
        winKey(LS_SESSION_ORDER + this.wsKey),
        JSON.stringify({ mode: this.mode, order: this.order }),
      );
    } catch {
      /* private mode */
    }
  }
}

export const sessionOrder = new SessionOrderStore();
