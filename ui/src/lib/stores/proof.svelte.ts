// Proof Packs store: the workspace pack list, the open pack's detail, and a
// cheap per-work-item summary roll-up (keyed "<kind>:<work_item_id>") that the
// sidebar reads to show an inline proof chip on each session row.
//
// Fed by REST loads (Proof page / sidebar) and the events WS
// (proof_pack_updated). applyEvent is invoked ONLY from the WS dispatcher (never
// from a $derived), so its writes + refetches are safe — keep it that way to
// avoid the state_unsafe_mutation footgun.

import { listProofPacks, proofSummary, getProofPack, type ProofPackFilter } from '../api/proof';
import type { OttoEvent, ProofPackDetail, ProofPackResp, ProofSummaryRow } from '../api/types';

class ProofStore {
  /** The current workspace's proof packs (filtered list). */
  packs: ProofPackResp[] = $state([]);
  /** The open pack's full detail (right pane), or null. */
  detail: ProofPackDetail | null = $state(null);
  /** Per-work-item roll-up keyed "<kind>:<work_item_id>" — sidebar chips. */
  summaryByWorkItem: Record<string, ProofSummaryRow> = $state({});
  /** Whether the list/detail is loading. */
  loading = $state(false);
  /** The workspace the current data belongs to. */
  wsId: string | null = $state(null);
  /** The filter last used by loadPacks, so an event reload preserves the view. */
  private lastFilter: ProofPackFilter | undefined = undefined;

  /** Look up the roll-up for a work item (e.g. `summary('session', id)`). */
  summaryFor(kind: string, workItemId: string): ProofSummaryRow | null {
    return this.summaryByWorkItem[`${kind}:${workItemId}`] ?? null;
  }

  /** Load the workspace's packs (optionally filtered) into `packs`. */
  async loadPacks(wsId: string, filter?: ProofPackFilter): Promise<void> {
    this.wsId = wsId;
    this.lastFilter = filter;
    this.loading = true;
    try {
      this.packs = await listProofPacks(wsId, filter);
    } catch {
      this.packs = [];
    } finally {
      this.loading = false;
    }
  }

  /** Load the cheap per-work-item summary roll-up for `wsId` (sidebar chips). */
  async loadSummary(wsId: string): Promise<void> {
    this.wsId = wsId;
    try {
      const resp = await proofSummary(wsId);
      const next: Record<string, ProofSummaryRow> = {};
      for (const r of resp.rows) next[`${r.work_item_kind}:${r.work_item_id}`] = r;
      this.summaryByWorkItem = next;
    } catch {
      /* best-effort */
    }
  }

  /** Open one pack's detail into the right pane. */
  async open(id: string): Promise<void> {
    this.loading = true;
    try {
      this.detail = await getProofPack(id);
    } catch {
      this.detail = null;
    } finally {
      this.loading = false;
    }
  }

  /** Refresh the open pack's detail (after a mutation). */
  async refreshDetail(): Promise<void> {
    if (this.detail) await this.open(this.detail.pack.id);
  }

  closeDetail(): void {
    this.detail = null;
  }

  /** Route the proof-related WS events. Returns true when handled. */
  applyEvent(ev: OttoEvent): boolean {
    if (ev.type !== 'proof_pack_updated') return false;
    // Only refresh data the open workspace owns (a different workspace's pack
    // change isn't on screen).
    if (this.wsId && ev.workspace_id === this.wsId) {
      // Cheapest correct refresh: re-pull the workspace summary so every sidebar
      // chip reflects the new status/risk/badges, and reload the list preserving
      // the page's active filter (never clobber the chosen view).
      void this.loadSummary(this.wsId);
      void this.loadPacks(this.wsId, this.lastFilter);
    }
    // Keep the open detail live regardless of the list filter.
    if (this.detail?.pack.id === ev.proof_pack_id) {
      void this.refreshDetail();
    }
    return true;
  }
}

export const proof = new ProofStore();
