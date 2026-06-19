// Vault module store — the Obsidian-like memory browser. Talks to the memory
// API (list / search / links / graph) scoped to the current workspace.

import { api } from '../../lib/api/client';
import { ws } from '../../lib/stores/workspace.svelte';
import type {
  Memory,
  MemoryGraphData,
  MemoryHit,
  MemoryLink,
  MemoryQuery,
} from '../../lib/api/types';

class VaultStore {
  items = $state<Memory[]>([]);
  hits = $state<MemoryHit[]>([]);
  selected = $state<Memory | null>(null);
  links = $state<MemoryLink[]>([]);
  graph = $state<MemoryGraphData | null>(null);
  query = $state('');
  collection = $state<string>('');
  loading = $state(false);
  mode = $state<'list' | 'graph'>('list');

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
    return this.collection ? base.filter((m) => m.collection === this.collection) : base;
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
      this.items = await api.get<Memory[]>(`/workspaces/${wsId}/memories?limit=200`);
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

  async forget(m: Memory): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId) return;
    await api.del(`/workspaces/${wsId}/memories/${m.id}`);
    if (this.selected?.id === m.id) this.selected = null;
    await this.load();
  }
}

export const vault = new VaultStore();
