// UI affordances mirror current server decisions. Execution is always checked
// again by the backend; a cached button state is never an authorization token.
import { untrack } from 'svelte';
import { accessApi } from '../api/access';
import { getToken } from '../api/client';
import { auth } from './auth.svelte';
import type { Capability, EffectiveAccess, Feature, ResourceKind } from '../api/types';

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
  async load(kind: ResourceKind, id: string, child?: string, force = false): Promise<void> {
    const key = this.key(kind, id, child);
    const pending = this.loading.get(key);
    if (pending) return pending;
    if (!force && untrack(() => this.entries[key]?.expires) > Date.now()) return;
    const generation = this.generation;
    let task!: Promise<void>;
    task = (async () => {
      try {
        const value = await accessApi.capabilities(kind, id, child);
        if (generation !== this.generation || key !== this.key(kind, id, child)) return;
        const before = this.entries[key]?.value ?? null;
        this.entries[key] = { value, expires: Date.now() + 30000, kind, id, child };
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
    this.notify({ type: 'reset', identity });
  }
  async refresh() {
    const entries = Object.values(this.entries);
    await Promise.allSettled(entries.map((e) => this.load(e.kind, e.id, e.child, true)));
  }
}
export const resourceAccess = new ResourceAccessStore();
if (typeof window !== 'undefined') {
  window.setInterval(() => {
    if (!document.hidden) void resourceAccess.refresh();
  }, 15000);
  window.addEventListener('focus', () => {
    void resourceAccess.refresh();
  });
  window.addEventListener('otto:auth-changed', () => resourceAccess.invalidate(true));
  window.addEventListener('storage', (event) => {
    if (event.key === 'otto_token') resourceAccess.invalidate(true);
  });
}
