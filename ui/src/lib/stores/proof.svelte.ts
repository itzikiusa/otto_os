// Proof Packs store: the workspace pack list, the open pack's detail, and a
// cheap per-work-item summary roll-up (keyed "<kind>:<work_item_id>") that the
// sidebar reads to show an inline proof chip on each session row.
//
// Fed by REST loads (Proof page / sidebar) and the events WS
// (proof_pack_updated). applyEvent is invoked ONLY from the WS dispatcher (never
// from a $derived), so its writes + refetches are safe — keep it that way to
// avoid the state_unsafe_mutation footgun.

import { listProofPacksPage, proofSummary, getProofPack, PROOF_SUMMARY_CHUNK, type ProofPackFilter } from '../api/proof';
import { loadErrorText } from '../loadError';
import type { OttoEvent, ProofPackDetail, ProofPackResp, ProofSummaryRow } from '../api/types';

/** Packs per keyset page. */
const PAGE = 100;

class ProofStore {
  /** The current workspace's proof packs (filtered list). */
  packs: ProofPackResp[] = $state([]);
  /** The open pack's full detail (right pane), or null. */
  detail: ProofPackDetail | null = $state(null);
  /** Per-work-item roll-up keyed "<kind>:<work_item_id>" — sidebar chips. */
  summaryByWorkItem: Record<string, ProofSummaryRow> = $state({});
  /** Whether the list/detail is loading. */
  private listLoading = $state(false);
  private detailLoading = $state(false);
  get loading(): boolean { return this.listLoading || this.detailLoading; }
  private listSeq = 0;
  private moreSeq = 0;
  /** Last failed list load (human text) — inline with Retry, never "no packs". */
  error: string | null = $state(null);
  /** Last failed detail load (human text) — inline with Retry in the right pane. */
  detailError: string | null = $state(null);
  /** The workspace the current data belongs to. */
  wsId: string | null = $state(null);
  /** The filter last used by loadPacks, so an event reload preserves the view. */
  private lastFilter: ProofPackFilter | undefined = undefined;
  /** Keyset cursor for the next page of `packs` (null = everything loaded). */
  nextCursor: string | null = $state(null);
  /** A "Load more" page is in flight. */
  loadingMore = $state(false);
  /** Whether the summary roll-up has been loaded for `wsId` (events patch it
   *  in place afterwards instead of refetching). */
  private summaryLoaded = false;

  /** Look up the roll-up for a work item (e.g. `summary('session', id)`). */
  summaryFor(kind: string, workItemId: string): ProofSummaryRow | null {
    return this.summaryByWorkItem[`${kind}:${workItemId}`] ?? null;
  }

  private scopeGeneration = 0;
  private setWorkspace(wsId: string): void {
    if (this.wsId === wsId) return;
    this.wsId = wsId;
    ++this.scopeGeneration;
    ++this.listSeq;
    ++this.moreSeq;
    this.closeDetail();
    this.packs = [];
    this.nextCursor = null;
    this.listLoading = false;
    this.loadingMore = false;
    this.error = null;
    this.summaryLoaded = false;
    this.askedKeys = new Set();
    this.summaryByWorkItem = {};
    this.pendingDetail.clear();
    this.needList = false;
    this.needSummary = false;
  }

  /** Load the workspace's packs (optionally filtered) into `packs`. */
  async loadPacks(wsId: string, filter?: ProofPackFilter): Promise<void> {
    this.setWorkspace(wsId);
    const seq = ++this.listSeq;
    ++this.moreSeq;
    if (this.wsId !== wsId || JSON.stringify(this.lastFilter) !== JSON.stringify(filter)) this.packs = [];
    this.wsId = wsId;
    this.lastFilter = filter;
    this.nextCursor = null;
    this.loadingMore = false;
    this.listLoading = true;
    this.error = null;
    try {
      const page = await listProofPacksPage(wsId, filter, PAGE);
      if (this.wsId !== wsId || this.listSeq !== seq) return;
      this.packs = page.packs;
      this.nextCursor = page.next;
      this.error = null;
    } catch (e) {
      if (this.wsId === wsId && this.listSeq === seq) this.error = loadErrorText(e);
    } finally {
      if (this.listSeq === seq) this.listLoading = false;
    }
  }

  /** Append only to the list generation that owns this cursor. */
  async loadMore(): Promise<void> {
    const wsId = this.wsId;
    const cursor = this.nextCursor;
    if (!wsId || !cursor || this.loadingMore || this.listLoading) return;
    const seq = this.listSeq;
    const request = ++this.moreSeq;
    this.loadingMore = true;
    try {
      const page = await listProofPacksPage(wsId, this.lastFilter, PAGE, cursor);
      if (this.wsId !== wsId || this.listSeq !== seq || this.moreSeq !== request) return;
      const have = new Set(this.packs.map((p) => p.id));
      this.packs = [...this.packs, ...page.packs.filter((p) => !have.has(p.id))];
      this.nextCursor = page.next;
      this.error = null;
    } catch (e) {
      if (this.listSeq === seq && this.moreSeq === request) this.error = loadErrorText(e);
    } finally {
      if (this.listSeq === seq && this.moreSeq === request) this.loadingMore = false;
    }
  }

  /** Work items (`"<kind>:<id>"`) already asked for in `wsId` — the scoped
   *  summary only ever fetches keys it has not asked for yet. */
  private askedKeys = new Set<string>();

  /** Load the cheap per-work-item summary roll-up for `wsId` (sidebar chips).
   *  With `workItems` (the rows the caller shows, `"<kind>:<id>"`) only those
   *  packs are read, and only the keys not asked for before — a sidebar that
   *  gains one session costs one tiny request, an unchanged one costs none.
   *  Without, the whole workspace (legacy full read). */
  async loadSummary(wsId: string, workItems?: string[]): Promise<void> {
    this.setWorkspace(wsId);
    const generation = this.scopeGeneration;
    try {
      if (!workItems) {
        const resp = await proofSummary(wsId);
        if (this.wsId !== wsId || generation !== this.scopeGeneration) return;
        const next: Record<string, ProofSummaryRow> = {};
        for (const r of resp.rows) next[`${r.work_item_kind}:${r.work_item_id}`] = r;
        this.summaryByWorkItem = next;
        this.summaryLoaded = true;
        return;
      }
      const fresh = workItems.filter((k) => !this.askedKeys.has(k));
      if (fresh.length === 0) {
        this.summaryLoaded = true;
        return;
      }
      for (const k of fresh) this.askedKeys.add(k);
      const rows: ProofSummaryRow[] = [];
      try {
        for (let i = 0; i < fresh.length; i += PROOF_SUMMARY_CHUNK) {
          const resp = await proofSummary(wsId, fresh.slice(i, i + PROOF_SUMMARY_CHUNK));
          if (this.wsId !== wsId || generation !== this.scopeGeneration) return;
          rows.push(...resp.rows);
        }
      } catch (e) {
        // Not answered: ask again next time.
        if (generation === this.scopeGeneration) {
          for (const k of fresh) this.askedKeys.delete(k);
        }
        throw e;
      }
      const next = { ...this.summaryByWorkItem };
      for (const r of rows) next[`${r.work_item_kind}:${r.work_item_id}`] = r;
      this.summaryByWorkItem = next;
      this.summaryLoaded = true;
    } catch {
      /* best-effort */
    }
  }

  /** Re-pull the summary for the current scope (an unpatchable event). */
  private reloadSummary(wsId: string): void {
    if (this.askedKeys.size === 0) {
      void this.loadSummary(wsId);
      return;
    }
    const keys = [...this.askedKeys];
    this.askedKeys = new Set();
    void this.loadSummary(wsId, keys);
  }

  /** Patch the summary row (and a loaded list entry) straight from a
   *  `proof_pack_updated` event that carries `badges`. Returns false when
   *  the event is too old to patch from (no badges) — caller refetches. */
  private patchFromEvent(ev: Extract<OttoEvent, { type: 'proof_pack_updated' }>): boolean {
    if (!ev.badges) return false;
    const key = `${ev.work_item_kind}:${ev.work_item_id}`;
    this.summaryByWorkItem = {
      ...this.summaryByWorkItem,
      [key]: {
        work_item_kind: ev.work_item_kind,
        work_item_id: ev.work_item_id,
        proof_pack_id: ev.proof_pack_id,
        status: ev.status,
        risk_score: ev.risk_score,
        done_score: ev.done_score ?? 0,
        badges: ev.badges,
      },
    };
    const i = this.packs.findIndex((p) => p.id === ev.proof_pack_id);
    if (i < 0) return false; // a pack the loaded list doesn't hold — reload it
    const cur = this.packs[i];
    const filter = this.lastFilter?.status;
    if (filter && filter !== ev.status) return false; // left the filtered view
    const next = [...this.packs];
    next[i] = {
      ...cur,
      badges: ev.badges,
      artifact_count: ev.artifact_count ?? cur.artifact_count,
      status: ev.status as ProofPackResp['status'],
      risk_score: ev.risk_score,
      done_score: ev.done_score ?? cur.done_score,
    };
    this.packs = next;
    return true;
  }

  /** Monotonic open() token: only the newest open may land (a slow pack A
   *  must never overwrite pack B opened after it). */
  private openSeq = 0;

  /** Open one pack's detail into the right pane. */
  async open(id: string): Promise<void> {
    const seq = ++this.openSeq;
    this.detailLoading = true;
    try {
      const d = await getProofPack(id);
      if (seq !== this.openSeq) return;
      this.detail = d;
      this.detailError = null;
    } catch (e) {
      if (seq !== this.openSeq) return;
      // A refresh failure of the SAME pack keeps it on screen; another pack's
      // detail never stands in for the one that failed to open.
      if (this.detail?.pack.id !== id) this.detail = null;
      this.detailError = loadErrorText(e);
    } finally {
      if (seq === this.openSeq) this.detailLoading = false;
    }
  }

  /** Mounted Proof pages. The pack list + open detail refresh on events only
   *  while one is; the sidebar summary chips always do. */
  private viewers = 0;
  private stale = false;
  watch(): () => void {
    this.viewers += 1;
    if (this.stale) {
      this.stale = false;
      if (this.detail) void this.refreshDetail();
    }
    let released = false;
    return () => {
      if (released) return;
      released = true;
      this.viewers = Math.max(0, this.viewers - 1);
    };
  }

  /** Trailing-coalesced event refresh: assembling a pack with 10 artifacts
   *  fired 10 events × (summary + list + detail) = 30 requests. */
  private pendingDetail = new Set<string>();
  /** Events that could not be patched in place (old emitter / pack not in
   *  the loaded list) — only these force a refetch. */
  private needSummary = false;
  private needList = false;
  private refreshTimer: ReturnType<typeof setTimeout> | null = null;
  private scheduleRefresh(): void {
    if (this.refreshTimer) return;
    this.refreshTimer = setTimeout(() => {
      this.refreshTimer = null;
      const wsId = this.wsId;
      if (wsId) {
        if (this.needSummary) this.reloadSummary(wsId);
        if (this.needList) {
          if (this.viewers > 0) void this.loadPacks(wsId, this.lastFilter);
          else this.stale = true;
        }
      }
      this.needSummary = false;
      this.needList = false;
      const openId = this.detail?.pack.id;
      if (openId && this.pendingDetail.has(openId)) {
        if (this.viewers > 0) void this.refreshDetail();
        else this.stale = true;
      }
      this.pendingDetail.clear();
    }, 300);
  }

  /** Refresh the open pack's detail (after a mutation). */
  async refreshDetail(): Promise<void> {
    if (this.detail) await this.open(this.detail.pack.id);
  }

  /** The signed-in identity changed (S13-02): nothing read with the previous
   *  token survives. Clears the list, detail and roll-up, invalidates loads in
   *  flight, and re-reads what is on screen for the current workspace. */
  identityChanged(): void {
    const wsId = this.wsId;
    const keys = [...this.askedKeys];
    this.closeDetail();
    ++this.scopeGeneration;
    ++this.listSeq;
    ++this.moreSeq;
    this.packs = [];
    this.nextCursor = null;
    this.loadingMore = false;
    this.listLoading = false;
    this.error = null;
    this.summaryByWorkItem = {};
    this.askedKeys = new Set();
    this.summaryLoaded = false;
    if (!wsId) return;
    if (this.viewers > 0) void this.loadPacks(wsId, this.lastFilter);
    void this.loadSummary(wsId, keys.length > 0 ? keys : undefined);
  }

  closeDetail(): void {
    ++this.openSeq;
    this.detailLoading = false;
    this.detail = null;
    this.detailError = null;
  }

  /** Route the proof-related WS events. Returns true when handled. */
  applyEvent(ev: OttoEvent): boolean {
    if (ev.type !== 'proof_pack_updated') return false;
    // Only refresh data the open workspace owns (a different workspace's pack
    // change isn't on screen).
    // Cheapest correct refresh (coalesced, 300 ms trailing): re-pull the
    // workspace summary so every sidebar chip reflects the new status/risk/
    // badges, and — while the page is mounted — reload the list preserving the
    // page's active filter and keep the open detail live.
    const ours = !!this.wsId && ev.workspace_id === this.wsId;
    const open = this.detail?.pack.id === ev.proof_pack_id;
    if (open) this.pendingDetail.add(ev.proof_pack_id);
    if (ours) {
      // The event carries status/risk/done/badges: patch in place. Only an
      // unpatchable event (old emitter, or a pack outside the loaded page)
      // costs a refetch — and the list one only while the page is mounted.
      const patched = this.summaryLoaded && this.patchFromEvent(ev);
      if (!patched) {
        if (!ev.badges || !this.summaryLoaded) this.needSummary = true;
        if (this.packs.length > 0 || this.viewers > 0) this.needList = true;
      }
    }
    if ((ours && (this.needSummary || this.needList)) || open) this.scheduleRefresh();
    return true;
  }
}

export const proof = new ProofStore();
