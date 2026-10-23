// Test-case reorder for TestCasesTab (accessibility-responsive-6w1-02): the
// local display order, drag-and-drop, the Move buttons and ⌥↑/⌥↓, and the
// queued save of `order_idx`. Lifted out of the tab so the save queue and the
// focus rule are one unit with a behavioural test (unit/p09-product.test.ts).

import { tick } from 'svelte';
import { announce } from '../../lib/announce.svelte';
import { toastError } from '../../lib/toastError';

export interface CaseOrderHost {
  /** The test-case run on screen (its cases are what's reordered). */
  run(): { id: string } | null;
  /** Persist the run's case order (`product.reorderTestcases`). */
  save(runId: string, ids: string[]): Promise<unknown>;
}

export class CaseOrder {
  /** Flat ordered case ids (mirrors the run's cases but can be locally
   *  rearranged before persisting). */
  ids = $state<string[]>([]);
  dragSrcId = $state<string | null>(null);
  dragOverId = $state<string | null>(null);
  saving = $state(false);
  /** A move landed while a save was in flight: the Move buttons stay enabled,
   *  so the in-flight save's `finally` saves again with the then-current
   *  order — every move persists, the last one wins. */
  private pending = false;
  private readonly host: CaseOrderHost;

  constructor(host: CaseOrderHost) {
    this.host = host;
  }

  dragStart(id: string): void {
    this.dragSrcId = id;
  }

  dragOver(id: string, e: DragEvent): void {
    e.preventDefault();
    this.dragOverId = id;
  }

  dragLeave(): void {
    this.dragOverId = null;
  }

  drop(targetId: string): void {
    const src = this.dragSrcId;
    if (!src || src === targetId) {
      this.dragEnd();
      return;
    }
    // Reorder: move the dragged case to be just before targetId.
    const next = this.ids.filter((id) => id !== src);
    const idx = next.indexOf(targetId);
    if (idx === -1) {
      next.push(src);
    } else {
      next.splice(idx, 0, src);
    }
    this.ids = next;
    this.dragEnd();
    // Persist immediately after drop.
    void this.persist();
  }

  dragEnd(): void {
    this.dragSrcId = null;
    this.dragOverId = null;
  }

  async persist(): Promise<void> {
    const run = this.host.run();
    if (!run) return;
    if (this.saving) {
      this.pending = true;
      return;
    }
    this.saving = true;
    try {
      await this.host.save(run.id, [...this.ids]);
    } catch (e) {
      toastError('Couldn’t save the order', e);
    } finally {
      this.saving = false;
      if (this.pending) {
        this.pending = false;
        void this.persist();
      }
    }
  }

  // ── Keyboard reorder (a11y-6-04): drag is not the only way ─────────────────
  /** Move `id` one place up (-1) or down (+1) among `group` (the cards shown
   *  in its category), persist, and keep focus on the moved card: on the Move
   *  button that was pressed (`keepMove`), or on the card's checkbox when that
   *  button just hit the list edge or the move came from the checkbox. */
  async move(id: string, dir: -1 | 1, group: string[], keepMove: 'up' | 'down' | null = null): Promise<void> {
    const at = group.indexOf(id);
    const neighbour = group[at + dir];
    if (at === -1 || !neighbour) return;
    const next = [...this.ids];
    const a = next.indexOf(id);
    const b = next.indexOf(neighbour);
    if (a === -1 || b === -1) return;
    [next[a], next[b]] = [next[b], next[a]];
    this.ids = next;
    void this.persist();
    await tick();
    const card = document.querySelector<HTMLElement>(`[data-case-id="${CSS.escape(id)}"]`);
    // WebKit doesn't focus a clicked <button>, so the pressed button is passed
    // in rather than read from document.activeElement.
    const keep = keepMove ? card?.querySelector<HTMLButtonElement>(`[data-move="${keepMove}"]`) : null;
    // A Move button that just went disabled (top/bottom) can't hold focus.
    (keep && !keep.disabled ? keep : card?.querySelector<HTMLElement>('.case-checkbox'))?.focus();
    announce(`Moved to position ${group.indexOf(neighbour) + 1} of ${group.length}`);
  }

  /** ⌥↑ / ⌥↓ on a focused card control moves the case (queued like a click,
   *  so a move during a save still persists). Focus stays on that control. */
  onKey(e: KeyboardEvent, id: string, group: string[]): void {
    if (!e.altKey || (e.key !== 'ArrowUp' && e.key !== 'ArrowDown')) return;
    e.preventDefault();
    const move = (e.currentTarget as HTMLElement | null)?.dataset?.move;
    void this.move(id, e.key === 'ArrowUp' ? -1 : 1, group, move === 'up' || move === 'down' ? move : null);
  }
}
