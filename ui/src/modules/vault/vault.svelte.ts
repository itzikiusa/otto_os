// Vault module store — the Obsidian-like memory browser. Talks to the memory
// API (list / search / links / graph / governance) scoped to the current workspace.

import { api } from '../../lib/api/client';
import { ws } from '../../lib/stores/workspace.svelte';
import { toasts } from '../../lib/toast.svelte';
import type {
  Memory,
  MemoryGraphData,
  MemoryHit,
  MemoryLink,
  MemoryQuery,
} from '../../lib/api/types';

// ---------------------------------------------------------------------------
// Module-local governance types (mirror of crates/otto-memory/src/governance.rs)
// ---------------------------------------------------------------------------

export type MemoryState = 'suggested' | 'accepted' | 'stale' | 'contradicted';

export interface SetStateReq {
  state: MemoryState;
}

export interface ForgetResp {
  undo_token: string;
}

export interface UndoForgetReq {
  undo_token: string;
}

export interface MergeReq {
  ids: string[];
  title: string;
  body: string;
}

export interface MergeResp {
  memory: Memory;
}

export interface SplitPart {
  title: string;
  body: string;
}

export interface SplitReq {
  parts: SplitPart[];
}

export interface SplitResp {
  memories: Memory[];
}

export type GovImportKind = 'agents-md' | 'claude-md' | 'cursorrules' | 'custom';

export interface GovImportReq {
  kind: GovImportKind;
  content: string;
  label?: string;
}

export interface GovImportResp {
  imported: number;
  import_id: string;
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

class VaultStore {
  items = $state<Memory[]>([]);
  hits = $state<MemoryHit[]>([]);
  selected = $state<Memory | null>(null);
  links = $state<MemoryLink[]>([]);
  graph = $state<MemoryGraphData | null>(null);
  query = $state('');
  collection = $state<string>('');
  stateFilter = $state<MemoryState | ''>('');
  loading = $state(false);
  mode = $state<'list' | 'graph'>('list');

  // Merge UI state
  mergeMode = $state(false);
  mergeIds = $state<string[]>([]);

  /** Distinct collections present, for the filter chips. */
  collections = $derived.by(() => {
    const set = new Set<string>();
    for (const m of this.items) set.add(m.collection);
    return [...set].sort();
  });

  /** What the left list shows: ranked hits when searching, else recent items. */
  visible = $derived.by(() => {
    const base = this.query.trim()
      ? this.hits.map((h) => h.memory)
      : this.items;
    const byCollection = this.collection
      ? base.filter((m) => m.collection === this.collection)
      : base;
    return this.stateFilter
      ? byCollection.filter((m) => (m as Memory & { state?: string }).state === this.stateFilter)
      : byCollection;
  });

  /** Notes that link TO the selected note (backlinks). */
  backlinks = $derived.by(() => {
    const id = this.selected?.id;
    if (!id) return [];
    return this.links.filter((l) => l.dst_id === id);
  });

  async load(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId) return;
    this.loading = true;
    try {
      this.items = await api.get<Memory[]>(`/workspaces/${wsId}/memories?limit=200&include_inactive=true`);
    } finally {
      this.loading = false;
    }
  }

  async search(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId) return;
    if (!this.query.trim()) {
      this.hits = [];
      return;
    }
    const q: MemoryQuery = { text: this.query, mode: 'hybrid', k: 50 };
    this.loading = true;
    try {
      this.hits = await api.post<MemoryHit[]>(`/workspaces/${wsId}/memory/search`, q);
    } finally {
      this.loading = false;
    }
  }

  async select(m: Memory): Promise<void> {
    this.selected = m;
    this.links = [];
    const wsId = ws.currentId;
    if (!wsId) return;
    this.links = await api.get<MemoryLink[]>(`/workspaces/${wsId}/memories/${m.id}/links`);
  }

  async loadGraph(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId) return;
    this.loading = true;
    try {
      this.graph = await api.get<MemoryGraphData>(`/workspaces/${wsId}/memory/graph`);
    } finally {
      this.loading = false;
    }
  }

  // -- legacy forget (hard active=0 via DELETE) ---------------------------

  async forget(m: Memory): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId) return;
    await api.del(`/workspaces/${wsId}/memories/${m.id}`);
    if (this.selected?.id === m.id) this.selected = null;
    await this.load();
  }

  // -- governance ---------------------------------------------------------

  /** Transition a memory's lifecycle state. */
  async setState(m: Memory, state: MemoryState): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId) return;
    const updated = await api.post<Memory>(
      `/workspaces/${wsId}/memory/${m.id}/state`,
      { state } satisfies SetStateReq,
    );
    this._replaceItem(updated);
    if (this.selected?.id === m.id) this.selected = updated;
    toasts.success('State updated', `Memory is now "${state}"`);
  }

  /** Soft-delete a memory; shows an undo toast for `ttl` seconds. */
  async softForget(m: Memory): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId) return;
    const resp = await api.post<ForgetResp>(`/workspaces/${wsId}/memory/${m.id}/forget`, {});
    if (this.selected?.id === m.id) this.selected = null;
    await this.load();
    // Show a 7-second undo toast.
    const token = resp.undo_token;
    toasts.push('info', 'Memory forgotten', 'Undo', 7000);
    // Store the undo callback on the window for the Undo button in the toast.
    // (The toast system doesn't support actions, so we expose it on the store.)
    this._pendingUndo = { token, wsId, memId: m.id };
  }

  _pendingUndo: { token: string; wsId: string; memId: string } | null = $state(null);

  /** Execute the pending undo (called by the toast Undo button). */
  async undoForget(): Promise<void> {
    const p = this._pendingUndo;
    if (!p) return;
    this._pendingUndo = null;
    const restored = await api.post<Memory>(
      `/workspaces/${p.wsId}/memory/${p.memId}/forget/undo`,
      { undo_token: p.token } satisfies UndoForgetReq,
    );
    this._replaceItem(restored);
    toasts.success('Memory restored', restored.title);
  }

  /** Toggle a memory in the merge selection. */
  toggleMergeSelect(m: Memory): void {
    if (this.mergeIds.includes(m.id)) {
      this.mergeIds = this.mergeIds.filter((id) => id !== m.id);
    } else {
      this.mergeIds = [...this.mergeIds, m.id];
    }
  }

  /** Execute merge: create merged memory from selected ids. */
  async executeMerge(title: string, body: string): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId || this.mergeIds.length < 2) return;
    const resp = await api.post<MergeResp>(`/workspaces/${wsId}/memory/merge`, {
      ids: this.mergeIds,
      title,
      body,
    } satisfies MergeReq);
    this.mergeMode = false;
    this.mergeIds = [];
    await this.load();
    this.selected = resp.memory;
    toasts.success('Merged', `Created "${resp.memory.title}"`);
  }

  /** Split the selected memory into parts. */
  async executeSplit(parts: SplitPart[]): Promise<void> {
    const wsId = ws.currentId;
    const m = this.selected;
    if (!wsId || !m || parts.length < 2) return;
    const resp = await api.post<SplitResp>(
      `/workspaces/${wsId}/memory/${m.id}/split`,
      { parts } satisfies SplitReq,
    );
    await this.load();
    this.selected = resp.memories[0] ?? null;
    toasts.success('Split', `Created ${resp.memories.length} memories`);
  }

  /** Import a governance file (AGENTS.md / CLAUDE.md / .cursorrules). */
  async importGoverned(kind: GovImportKind, content: string, label?: string): Promise<GovImportResp> {
    const wsId = ws.currentId;
    if (!wsId) throw new Error('no workspace');
    const resp = await api.post<GovImportResp>(`/workspaces/${wsId}/memory/import`, {
      kind,
      content,
      label,
    } satisfies GovImportReq);
    await this.load();
    toasts.success('Imported', `${resp.imported} memories created`);
    return resp;
  }

  /** Replace one item in `items` in-place (after a state/patch mutation). */
  _replaceItem(updated: Memory): void {
    this.items = this.items.map((m) => (m.id === updated.id ? updated : m));
  }
}

export const vault = new VaultStore();
