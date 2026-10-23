// The API client's draft helpers: opening a saved request or a history entry
// as a draft (in a tab that loses nobody's edits), duplicating, the dirty
// check, saving a draft, and the cache of full saved rows behind the tree's
// summaries. Split out of apiClient.svelte.ts (arch-6-03); the store forwards
// every public name and keeps the `requests` / `fullRequests` lists.

import { api } from '../../api/client';
import { toasts } from '../../toast.svelte';
import { loadErrorText } from '../../loadError';
import { forDuplicate, unmaskHistory } from '../../api/apiSecretShapes';
import type { ApiHistoryEntry, ApiKeyVal, ApiRequest, ExecuteApiReq, Id, UpsertApiRequestReq } from '../../api/types';
import type { ApiClientStore } from '../apiClient.svelte';
import {
  blankDraft, draftToExtras, extrasToDraft, FULL_REQUESTS_MAX, historyResponse, requestItem, type ApiDraft,
} from './model';

/** The store members this part reads and calls (its whole contract with the façade). */
export type DraftsHost = Pick<
  ApiClientStore,
  | 'base'
  | 'draft'
  | 'fullRequests'
  | 'replayIssue'
  | 'requests'
  | 'saveRequest'
  | 'tabSet'
  | 'wsId'
>;

export class ApiDrafts {
  private fullInflight = new Map<Id, Promise<ApiRequest | null>>();

  constructor(private readonly host: DraftsHost) {}

  /** Persist the current draft into a collection. Returns the saved request. */
  async saveDraft(name: string, collectionId: Id | null): Promise<ApiRequest | null> {
    const d = this.host.draft;
    const issue = this.host.replayIssue(d);
    if (issue) { toasts.error('Incomplete history request', issue); return null; }
    const wid = this.host.wsId(), tabId = d.tabId;
    const submittedAuth = JSON.stringify(d.auth);
    const body: UpsertApiRequestReq = {
      collection_id: collectionId,
      name,
      method: d.method,
      url: d.url,
      headers: d.headers,
      query: d.query,
      body_mode: d.body_mode,
      body: d.body,
      auth: d.auth,
      ssh_connection_id: d.ssh_connection_id ?? null,
      // Scripts / docs / settings / GraphQL variables / transport persist with
      // the request now — a reload no longer discards them.
      extras: draftToExtras(d),
    };
    const saved = await this.host.saveRequest(body, d.requestId ?? undefined);
    if (saved) {
      // Re-adopt the SAVED auth: secret members the daemon just moved to the
      // Keychain come back as `$secret` markers, and the builder shows them
      // masked instead of holding plaintext in tab-persisted localStorage.
      if (this.host.wsId() !== wid) return saved;
      const index = this.host.tabSet.tabs.findIndex(t => t.tabId === tabId);
      if (index >= 0) {
        const current = this.host.tabSet.tabs[index];
        this.host.tabSet.tabs[index] = { ...current, requestId: saved.id, name: saved.name,
          auth: JSON.stringify(current.auth) === submittedAuth ? {...saved.auth} : current.auth };
        this.host.tabSet.persist();
      }
      toasts.success('Request saved', saved.name);
    }
    return saved;
  }

  /** Open a fresh request in a new tab (optionally "in" a collection, which
   *  the save sheet then preselects). */
  newDraft(collectionHint: Id | null = null): void {
    this.host.tabSet.openTab({ ...blankDraft(), collectionHint });
  }

  /** Open a copy of the active request in a new, unsaved tab. Keychain-backed
   *  credentials are not copied (they belong to the original's Keychain item). */
  duplicateDraft(): void {
    const src = $state.snapshot(this.host.draft) as ApiDraft;
    const saved = src.requestId ? this.host.requests.find((r) => r.id === src.requestId) : undefined;
    const { blanked, ...copy } = forDuplicate(src);
    this.host.tabSet.openTab({
      ...copy,
      requestId: null,
      name: `${src.name?.trim() || this.host.tabSet.tabLabel(src)} copy`,
      collectionHint: saved?.collection_id ?? src.collectionHint ?? null,
    });
    if (blanked) toasts.info('Duplicated without stored credentials', 'Re-enter them in Auth before sending, or use an environment {{variable}}.');
  }

  /** Put a loaded draft in front WITHOUT losing anyone's edits: a tab that
   *  already shows the same saved request is focused as-is; the active tab is
   *  reused only while it holds nothing unsaved (a pristine blank draft or an
   *  unmodified saved request); otherwise the draft opens in a new tab. */
  placeDraft(d: ApiDraft): void {
    if (d.requestId) {
      const open = this.host.tabSet.tabs.findIndex((t) => t.requestId === d.requestId);
      if (open >= 0) {
        this.host.tabSet.switchTab(open);
        return;
      }
    }
    // A tab with a send in flight is never repurposed — its result has a home.
    // Nor is one whose saved row isn't loaded yet (its dirtiness is unknown).
    const unknown = !!this.host.draft.requestId && !this.host.fullRequests.has(this.host.draft.requestId);
    if (unknown || this.isDirty(this.host.draft) || this.host.tabSet.slot(this.host.draft.tabId)?.sending) {
      this.host.tabSet.openTab(d);
      return;
    }
    const replaced = this.host.draft.tabId;
    this.host.draft = d;
    if (replaced !== this.host.draft.tabId) this.host.tabSet.dropSlot(replaced);
  }

  /** Load a saved request into the builder (persisted extras included). */
  loadRequestIntoDraft(r: ApiRequest): void {
    // The tab's saved baseline (dirty check, secret stripping).
    if (this.host.fullRequests.get(r.id) !== r) this.cacheFull(r);
    this.placeDraft(extrasToDraft(
      {
        tabId: crypto.randomUUID(),
        requestId: r.id,
        name: r.name,
        kind: 'http',
        method: r.method,
        url: r.url,
        headers: r.headers.map((h) => ({ ...h })),
        query: r.query.map((q) => ({ ...q })),
        body_mode: r.body_mode,
        body: r.body,
        auth: { ...r.auth },
        ssh_connection_id: r.ssh_connection_id ?? null,
        proto: '',
        grpc_method: '',
      },
      r.extras,
    ));
  }

  /** Open a saved request by id: its full row is fetched (or reused from
   *  `fullRequests`) first; a tab already showing it is focused at once. */
  async openRequest(id: Id): Promise<boolean> {
    const open = this.host.tabSet.tabs.findIndex((t) => t.requestId === id);
    if (open >= 0) {
      this.host.tabSet.switchTab(open);
      return true;
    }
    const wid = this.host.wsId();
    const full = await this.ensureRequest(id);
    if (!full || this.host.wsId() !== wid) return false;
    this.loadRequestIntoDraft(full);
    return true;
  }

  /** The full saved request `id` — cached, else one `GET requests/{id}`
   *  (concurrent callers share it). Null (with a toast) when it can't load. */
  async ensureRequest(id: Id): Promise<ApiRequest | null> {
    const have = this.host.fullRequests.get(id);
    if (have) return have;
    const base = this.host.base();
    if (!base) return null;
    let p = this.fullInflight.get(id);
    if (!p) {
      p = api
        .get<ApiRequest>(`${base}/requests/${encodeURIComponent(id)}`)
        .then((r) => {
          if (this.host.base() === base) this.cacheFull(r);
          return r;
        })
        .catch((e: unknown) => {
          toasts.error('Couldn’t open the saved request', loadErrorText(e));
          return null;
        })
        .finally(() => this.fullInflight.delete(id));
      this.fullInflight.set(id, p);
    }
    return p;
  }

  /** Cache a full row; beyond FULL_REQUESTS_MAX the oldest entries not open
   *  in a tab are dropped. */
  cacheFull(r: ApiRequest): void {
    const next = new Map(this.host.fullRequests);
    next.delete(r.id);
    next.set(r.id, r);
    if (next.size > FULL_REQUESTS_MAX) {
      const open = new Set(this.host.tabSet.tabs.map((t) => t.requestId));
      for (const k of next.keys()) {
        if (next.size <= FULL_REQUESTS_MAX) break;
        if (!open.has(k)) next.delete(k);
      }
    }
    this.host.fullRequests = next;
  }

  /** A saved (or re-fetched) full request: cache it and patch its tree row. */
  upsertRequest(r: ApiRequest): void {
    this.cacheFull(r);
    const item = requestItem(r);
    this.host.requests = this.host.requests.some((x) => x.id === r.id)
      ? this.host.requests.map((x) => (x.id === r.id ? item : x))
      : [...this.host.requests, item];
  }

  dropRequest(id: Id): void {
    this.host.requests = this.host.requests.filter((r) => r.id !== id);
    if (this.host.fullRequests.has(id)) {
      const next = new Map(this.host.fullRequests);
      next.delete(id);
      this.host.fullRequests = next;
    }
  }

  /** True when a tab's draft differs from its saved request (or is a
   * non-empty unsaved draft) — includes the extras fields (scripts / docs /
   * settings / GraphQL variables / transport). Drives the tab's unsaved dot. */
  isDirty(d: ApiDraft): boolean {
    const saved = d.requestId ? this.host.fullRequests.get(d.requestId) : undefined;
    // Saved row not fetched yet (a restored tab, loading): not dirty for the
    // dot — `placeDraft` treats it as dirty so it never reuses the tab.
    if (d.requestId && !saved && this.host.requests.some((r) => r.id === d.requestId)) return false;
    if (!saved) {
      // Unsaved draft: dirty once anything meaningful was entered.
      return Boolean(
        d.url.trim() || d.body.trim() || d.headers.length || d.query.length ||
        d.auth.type !== 'none' || Object.keys(draftToExtras(d)).length > 1,
      );
    }
    const kv = (rows: ApiKeyVal[]): string =>
      JSON.stringify(rows.filter((r) => r.key.trim() !== '' || r.value.trim() !== ''));
    return (
      (d.name.trim() !== '' && d.name !== saved.name) ||
      d.method !== saved.method ||
      d.url !== saved.url ||
      d.body !== saved.body ||
      d.body_mode !== saved.body_mode ||
      kv(d.headers) !== kv(saved.headers) ||
      kv(d.query) !== kv(saved.query) ||
      JSON.stringify(d.auth) !== JSON.stringify(saved.auth) ||
      (d.ssh_connection_id ?? null) !== (saved.ssh_connection_id ?? null) ||
      JSON.stringify(draftToExtras(d)) !== JSON.stringify(saved.extras ?? {v: 1})
    );
  }

  /** Load a history entry's request snapshot into the builder (best-effort).
   *  History stores credentials MASKED (`***`); those are refilled from the
   *  saved request the entry ran (by `request_id`, else the one saved request
   *  with the same method + URL), otherwise blanked with a re-enter hint —
   *  the mask itself is never sent as a credential. */
  async loadHistoryIntoDraft(h: ApiHistoryEntry): Promise<void> {
    const snap = (h.request ?? {}) as Partial<ExecuteApiReq> & { request_id?: Id | null; source?: unknown; request_truncated?: boolean };
    const resultOnly = snap.source === 'automation_run';
    let multipartFieldsComplete = false;
    if (snap.body_mode === 'multipart') {
      try { multipartFieldsComplete = Array.isArray(JSON.parse(snap.body ?? '')); } catch { /* capped JSON is incomplete */ }
    }
    const method = snap.method ?? h.method;
    const url = snap.url ?? h.url;
    let savedId = snap.request_id && this.host.requests.some((r) => r.id === snap.request_id) ? snap.request_id : undefined;
    if (!savedId && !resultOnly) {
      const same = this.host.requests.filter((r) => r.method === method && r.url === url);
      if (same.length === 1) savedId = same[0].id;
    }
    const wid = this.host.wsId();
    const saved = savedId ? (await this.ensureRequest(savedId)) ?? undefined : undefined;
    if (this.host.wsId() !== wid) return;
    const un = unmaskHistory(
      { auth: snap.auth ?? { type: 'none' }, headers: snap.headers ?? [], query: snap.query ?? [] },
      saved,
    );
    this.placeDraft({
      tabId: crypto.randomUUID(),
      requestId: null,
      name: '',
      kind: 'http',
      method,
      url,
      headers: un.headers,
      query: un.query,
      body_mode: snap.body_mode ?? 'none',
      body: snap.body ?? '',
      historyReplay: resultOnly
        ? {reason:'This automation history entry contains results only. Open the current saved request to run it again.', ...(savedId ? {requestId:savedId} : {})}
        : snap.request_truncated && !multipartFieldsComplete
          ? {reason:'History retained only a body preview. Replace the body before sending or saving.',body:snap.body ?? ''}
          : undefined,
      auth: un.auth,
      ssh_connection_id: snap.ssh_connection_id ?? null,
      proto: '',
      grpc_method: '',
    });
    // Show the response the entry already stored (no resend — matters for
    // non-idempotent calls), flagged so the pane says where it came from.
    const stored = historyResponse(h);
    this.host.tabSet.patchSlot(this.host.draft.tabId, {
      resp: stored.resp, error: stored.error, tests: [], logs: [],
      history: stored.resp || stored.error
        ? { at: h.executed_at, truncated: stored.resp?.truncated ?? false }
        : null,
    });
    const replayIssue = this.host.replayIssue();
    if (replayIssue) toasts.info('Incomplete history request', replayIssue);
    if (un.blanked) {
      toasts.info('Re-enter credentials', 'History stores secrets masked — fill the blanked credential fields before sending.');
    }
  }
}
