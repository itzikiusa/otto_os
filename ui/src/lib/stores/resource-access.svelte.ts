// UI affordances mirror current server decisions. Execution is always checked
// again by the backend; a cached button state is never an authorization token.
import { untrack } from 'svelte';
import { accessApi } from '../api/access';
import { getToken } from '../api/client';
import { auth } from './auth.svelte';
import { createLimiter, mapLimit } from '../poll';
import { appLive, liveQuery, LIVE_SAFETY_MS } from '../live';
import type { Capability, EffectiveAccess, Feature, ResourceKind } from '../api/types';

/** A decision stays usable while it can still be corrected: 30 s when the
 *  event socket is down (the old 15 s re-check keeps it fresh), past the
 *  safety net while `resource_access_changed` events keep it honest. */
const OFFLINE_TTL_MS = 30_000;
function ttl(): number {
  return appLive.connected() ? LIVE_SAFETY_MS + 60_000 : OFFLINE_TTL_MS;
}

/** Every capability check of this document shares this gate (r3-04-04): a
 *  DB page with N connections or a k8s view with 300 namespaces used to fire
 *  one unbounded request per resource — each a distinct URL, so each also a
 *  CORS preflight — queued ahead of the user's first click. */
const checkGate = createLimiter(3);

type Entry = {
  value: EffectiveAccess | null;
  expires: number;
  kind: ResourceKind;
  id: string;
  child?: string;
};
export type ResourceAccessChange =
  | { type: 'reset'; identity: boolean }
  | {
      type: 'decision';
      kind: ResourceKind;
      id: string;
      child?: string;
      before: EffectiveAccess | null;
      after: EffectiveAccess | null;
    };
class ResourceAccessStore {
  private listeners = new Set<(change: ResourceAccessChange) => void>();
  private token: string | null = typeof window === 'undefined' ? null : getToken();
  private tokenEpoch = 0;
  subscribe(listener: (change: ResourceAccessChange) => void) {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }
  private notify(change: ResourceAccessChange) {
    for (const listener of this.listeners) listener(change);
  }
  private identityKey() {
    const token = typeof window === 'undefined' ? null : getToken();
    if (token !== this.token) {
      this.token = token;
      this.tokenEpoch++;
    }
    return this.tokenEpoch;
  }

  private entries: Record<string, Entry> = $state({});
  private loading = new Map<string, Promise<void>>();
  private generation = 0;
  private key(kind: ResourceKind, id: string, child?: string) {
    return JSON.stringify([auth.me?.id, this.identityKey(), kind, id, child ?? null]);
  }
  async load(kind: ResourceKind, id: string, child?: string, force = false, background = false): Promise<void> {
    const key = this.key(kind, id, child);
    const pending = this.loading.get(key);
    if (pending) return pending;
    if (!force && untrack(() => this.entries[key]?.expires) > Date.now()) return;
    const generation = this.generation;
    let task!: Promise<void>;
    task = (async () => {
      try {
        const value = await checkGate(() => accessApi.capabilities(kind, id, child, background));
        if (generation !== this.generation || key !== this.key(kind, id, child)) return;
        const before = this.entries[key]?.value ?? null;
        this.entries[key] = { value, expires: Date.now() + ttl(), kind, id, child };
        this.notify({ type: 'decision', kind, id, child, before, after: value });
      } catch {
        if (generation === this.generation && key === this.key(kind, id, child)) {
          const before = this.entries[key]?.value ?? null;
          this.entries[key] = { value: null, expires: Date.now() + 3000, kind, id, child };
          if (before) this.notify({ type: 'decision', kind, id, child, before, after: null });
        }
      } finally {
        if (this.loading.get(key) === task) this.loading.delete(key);
      }
    })();
    this.loading.set(key, task);
    return task;
  }
  private batches = new Map<symbol, { kind: ResourceKind; id: string; children: string[]; names: Set<string> }>();
  private batchLoading = new Map<symbol, Promise<void>>();
  /** Mounted catalogue ownership: release removes unused child decisions and
   * their periodic refresh work, including responses still in flight. */
  retainChildren(kind: ResourceKind, id: string, children: string[]): () => void {
    const owner = Symbol();
    this.batches.set(owner, { kind, id, children: [...new Set(children)], names: new Set(children) });
    void this.loadChildren(owner);
    return () => {
      const batch = this.batches.get(owner);
      this.batches.delete(owner);
      if (!batch) return;
      for (const child of batch.children) {
        if (![...this.batches.values()].some((b) => b.kind === kind && b.id === id && b.names.has(child))) {
          delete this.entries[this.key(kind, id, child)];
        }
      }
    };
  }
  private async loadChildren(owner: symbol, background = false): Promise<void> {
    const pending = this.batchLoading.get(owner);
    if (pending) return pending;
    const batch = this.batches.get(owner);
    if (!batch) return;
    const { kind, id, children } = batch;
    const generation = this.generation;
    const identity = this.key(kind, id);
    const owns = () => this.batches.has(owner) && generation === this.generation && identity === this.key(kind, id);
    const install = (child: string, value: EffectiveAccess | null) => {
      if (!owns()) return;
      const key = this.key(kind, id, child);
      const before = this.entries[key]?.value ?? null;
      this.entries[key] = { value, expires: Date.now() + (value ? ttl() : 3000), kind, id, child };
      this.notify({ type: 'decision', kind, id, child, before, after: value });
    };
    const task = (async () => {
      for (let offset = 0; offset < children.length && owns(); offset += 1000) {
        const chunk = children.slice(offset, offset + 1000);
        try {
          const values = await checkGate(() => accessApi.capabilitiesBatch(kind, id, chunk, background));
          chunk.forEach((child, i) => install(child, values[i]?.child === child ? values[i] : null));
        } catch { chunk.forEach((child) => install(child, null)); }
      }
    })();
    this.batchLoading.set(owner, task);
    try { await task; } finally { if (this.batchLoading.get(owner) === task) this.batchLoading.delete(owner); }
  }
  private isBatched(e: Entry): boolean {
    return e.child !== undefined && [...this.batches.values()].some((b) => b.kind === e.kind && b.id === e.id && b.names.has(e.child!));
  }
  can(
    kind: ResourceKind,
    id: string,
    operation: string,
    legacyFeature: Feature,
    legacyCapability: Capability,
    child?: string,
  ): boolean {
    const entry = this.entries[this.key(kind, id, child)];
    if (!entry?.value || entry.expires <= Date.now()) return false;
    if (entry.value.mode === 'legacy') return auth.can(legacyFeature, legacyCapability);
    return auth.can(legacyFeature, 'view') && entry.value.operations[operation]?.allowed === true;
  }
  get(kind: ResourceKind, id: string, child?: string): EffectiveAccess | null {
    return this.entries[this.key(kind, id, child)]?.value ?? null;
  }
  invalidate(identity = false) {
    this.generation++;
    this.entries = {};
    this.loading.clear();
    this.batchLoading.clear();
    if (identity) for (const owner of this.batches.keys()) void this.loadChildren(owner);
    this.notify({ type: 'reset', identity });
  }
  /** Re-check every cached decision, at most 2 requests at a time: the cache
   *  holds one entry per resource ever shown (every connection row), and an
   *  unbounded fan-out took every webview socket each 15 s tick. */
  async refresh(match?: { kind?: string; resource_id?: string }) {
    await mapLimit([...this.batches.entries()].filter(([, b]) => !match?.kind || (b.kind === match.kind && (!match.resource_id || b.id === match.resource_id))), 2,
      ([owner]) => this.loadChildren(owner, true));
    const entries = Object.values(this.entries).filter(
      (e) => !this.isBatched(e) && (!match?.kind || (e.kind === match.kind && (!match.resource_id || e.id === match.resource_id))),
    );
    await mapLimit(entries, 2, (e) => this.load(e.kind, e.id, e.child, true, true).catch(() => {}));
  }
  /** Re-check only the decisions whose TTL ran out (window focus). While the
   *  event socket is up that is ~none — `resource_access_changed` keeps them
   *  honest — so a Cmd-Tab back no longer re-checks every connection and
   *  namespace seen this session. */
  async refreshStale() {
    const now = Date.now();
    await mapLimit([...this.batches.entries()].filter(([, b]) => b.children.some((child) => (this.entries[this.key(b.kind, b.id, child)]?.expires ?? 0) <= now)), 2,
      ([owner]) => this.loadChildren(owner, true));
    const stale = Object.values(this.entries).filter((e) => !this.isBatched(e) && e.expires <= now);
    await mapLimit(stale, 2, (e) => this.load(e.kind, e.id, e.child, true, true).catch(() => {}));
  }
}
export const resourceAccess = new ResourceAccessStore();
if (typeof window !== 'undefined') {
  // Event-fed (TRANSPORT_PLAN stage 2): an access write broadcasts
  // `resource_access_changed` — re-check just that resource (or everything
  // for a group/role/grant/membership change). The 15 s re-check of EVERY
  // cached resource now runs only while the event socket is down; otherwise a
  // 5-min safety net + a refresh after reconnects. Same chain rules as before
  // (no overlap, paused while hidden, jittered).
  liveQuery({ run: () => resourceAccess.refresh(), on: [], fallbackMs: 15_000, immediate: false });
  appLive.on(['resource_access_changed'], (ev) => {
    const kind = typeof ev.kind === 'string' ? ev.kind : undefined;
    const resource_id = typeof ev.resource_id === 'string' ? ev.resource_id : undefined;
    void resourceAccess.refresh(kind ? { kind, resource_id } : undefined);
  });
  window.addEventListener('focus', () => void resourceAccess.refreshStale());
  window.addEventListener('otto:auth-changed', () => resourceAccess.invalidate(true));
  window.addEventListener('storage', (event) => {
    if (event.key === 'otto_token') resourceAccess.invalidate(true);
  });
}
