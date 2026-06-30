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
  CodeRepo,
  CodeSymbol,
  FullGraph,
  IndexResult,
  IndexRepoReq,
  RepoBrain,
  VaultBackend,
  VaultBackendReq,
  VaultDocReq,
  VaultHealth,
  VaultInstallPlan,
  VaultInstallResult,
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

// -- Embedder configuration (mirror of crates/otto-server/src/embedder.rs) ----

export type EmbedderProvider = 'stub' | 'openai' | 'voyage';

export interface EmbedderStatus {
  provider: string;
  model: string | null;
  dim: number | null;
  active: boolean;
  key_present: boolean;
}

export interface SetEmbedderReq {
  provider: EmbedderProvider;
  api_key?: string;
}

// ---------------------------------------------------------------------------
// Vault v2 — top-level tabs
// ---------------------------------------------------------------------------

export type VaultTab = 'knowledge' | 'graph' | 'repos' | 'symbols' | 'backends' | 'brain';

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

  // Embedder configuration state
  embedder = $state<EmbedderStatus | null>(null);
  embedderBusy = $state(false);

  // -- Vault v2: code intelligence + remote backends ----------------------
  // The active top-level tab. Knowledge keeps the original Obsidian-like
  // memory browser; the rest surface the code "Repo Brain".
  tab = $state<VaultTab>('knowledge');

  // Indexed code repositories (files/symbols/edges counts + status).
  repos = $state<CodeRepo[]>([]);
  reposLoading = $state(false);
  indexing = $state(false);
  lastIndex = $state<IndexResult | null>(null);

  // Symbol browser.
  symbols = $state<CodeSymbol[]>([]);
  symbolQuery = $state('');
  symbolRepoId = $state<string>('');
  symbolsLoading = $state(false);

  // Unified knowledge+code graph (the headline Obsidian-style view).
  fullGraph = $state<FullGraph | null>(null);
  graphRepoId = $state<string>('');
  fullGraphLoading = $state(false);
  /** A node id to centre/focus in the graph (bumped from the tree or a deep link). */
  focusNodeId = $state<string | null>(null);

  // Remote backends (Qdrant / SurrealDB / Ollama).
  backends = $state<VaultBackend[]>([]);
  backendsLoading = $state(false);

  // Repo Brain (focus → assembled context).
  brain = $state<RepoBrain | null>(null);
  brainFocus = $state('');
  brainBusy = $state(false);

  /** Search hits keyed by memory id, so the index can show "why selected" chips. */
  hitsById = $derived.by(() => {
    const m = new Map<string, MemoryHit>();
    for (const h of this.hits) m.set(h.memory.id, h);
    return m;
  });

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

  // -- embedder config -----------------------------------------------------

  /** Fetch the active Vault embedder (provider/model/dim). */
  async loadEmbedder(): Promise<void> {
    try {
      this.embedder = await api.get<EmbedderStatus>('/memory/embedder');
    } catch {
      // Non-fatal: the panel just stays hidden if status can't be read.
      this.embedder = null;
    }
  }

  /** Switch the embedder provider (optionally storing an API key), then refresh. */
  async setEmbedder(provider: EmbedderProvider, apiKey?: string): Promise<void> {
    this.embedderBusy = true;
    try {
      const body: SetEmbedderReq = { provider };
      if (apiKey && apiKey.trim()) body.api_key = apiKey.trim();
      this.embedder = await api.put<EmbedderStatus>('/memory/embedder', body);
      toasts.info(`Embedder set to ${this.embedder.model ?? provider}`);
    } catch (e) {
      toasts.error('Could not set embedder', e instanceof Error ? e.message : String(e));
    } finally {
      this.embedderBusy = false;
    }
  }

  /** Re-embed this workspace's memories under the active embedder. */
  async reindex(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId) return;
    this.embedderBusy = true;
    try {
      const r = await api.post<{ embedded: number }>(`/workspaces/${wsId}/memory/reindex`, {});
      toasts.info(`Re-embedded ${r.embedded} ${r.embedded === 1 ? 'memory' : 'memories'}`);
    } catch (e) {
      toasts.error('Reindex failed', e instanceof Error ? e.message : String(e));
    } finally {
      this.embedderBusy = false;
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

  // -- Vault v2: code repos ------------------------------------------------

  /** List indexed code repositories for the current workspace. */
  async loadRepos(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId) return;
    this.reposLoading = true;
    try {
      this.repos = await api.get<CodeRepo[]>(`/workspaces/${wsId}/vault/repos`);
    } finally {
      this.reposLoading = false;
    }
  }

  /** Index (or re-index) a repo by absolute path; returns the resulting counts. */
  async indexRepo(root: string, name?: string): Promise<IndexResult | null> {
    const wsId = ws.currentId;
    if (!wsId || !root.trim()) return null;
    this.indexing = true;
    try {
      const body: IndexRepoReq = { root: root.trim() };
      if (name && name.trim()) body.name = name.trim();
      const r = await api.post<IndexResult>(`/workspaces/${wsId}/vault/repos/index`, body);
      this.lastIndex = r;
      toasts.success('Indexed', `${r.files} files · ${r.symbols} symbols · ${r.edges} edges`);
      await this.loadRepos();
      return r;
    } catch (e) {
      toasts.error('Index failed', e instanceof Error ? e.message : String(e));
      return null;
    } finally {
      this.indexing = false;
    }
  }

  // -- Vault v2: symbols ---------------------------------------------------

  /** Search code symbols (name/kind/file/signature), optionally scoped to a repo. */
  async searchSymbols(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId) return;
    this.symbolsLoading = true;
    try {
      const p = new URLSearchParams();
      if (this.symbolQuery.trim()) p.set('q', this.symbolQuery.trim());
      if (this.symbolRepoId) p.set('repo_id', this.symbolRepoId);
      p.set('limit', '200');
      this.symbols = await api.get<CodeSymbol[]>(`/workspaces/${wsId}/vault/symbols?${p.toString()}`);
    } finally {
      this.symbolsLoading = false;
    }
  }

  // -- Vault v2: full graph ------------------------------------------------

  /** Load the unified knowledge+code graph (optionally scoped to a repo). */
  async loadFullGraph(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId) return;
    this.fullGraphLoading = true;
    try {
      const q = this.graphRepoId ? `?repo_id=${encodeURIComponent(this.graphRepoId)}` : '';
      this.fullGraph = await api.get<FullGraph>(`/workspaces/${wsId}/vault/fullgraph${q}`);
    } finally {
      this.fullGraphLoading = false;
    }
  }

  /** Jump to the Graph tab scoped to one repo and (re)load it. */
  async openRepoGraph(repoId: string): Promise<void> {
    this.graphRepoId = repoId;
    this.tab = 'graph';
    await this.loadFullGraph();
  }

  // -- Vault v2: remote backends ------------------------------------------

  /** List the workspace's remote-backend configs (Qdrant / SurrealDB / Ollama). */
  async loadBackends(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId) return;
    this.backendsLoading = true;
    try {
      this.backends = await api.get<VaultBackend[]>(`/workspaces/${wsId}/vault/backends`);
    } finally {
      this.backendsLoading = false;
    }
  }

  /** Create/update a backend config (secret is stored in the Keychain server-side). */
  async saveBackend(kind: string, req: VaultBackendReq): Promise<VaultBackend | null> {
    const wsId = ws.currentId;
    if (!wsId) return null;
    try {
      const updated = await api.put<VaultBackend>(`/workspaces/${wsId}/vault/backends/${kind}`, req);
      this.backends = this.backends.some((b) => b.kind === kind)
        ? this.backends.map((b) => (b.kind === kind ? updated : b))
        : [...this.backends, updated];
      toasts.success('Saved', `${kind} backend updated`);
      return updated;
    } catch (e) {
      toasts.error('Save failed', e instanceof Error ? e.message : String(e));
      return null;
    }
  }

  /** Ping a backend; reflects the result in the matching card's status. */
  async testBackend(kind: string): Promise<VaultHealth | null> {
    const wsId = ws.currentId;
    if (!wsId) return null;
    try {
      const h = await api.post<VaultHealth>(`/workspaces/${wsId}/vault/backends/${kind}/health`, {});
      this.backends = this.backends.map((b) =>
        b.kind === kind ? { ...b, status: h.status, message: h.message } : b,
      );
      if (h.status === 'ok') toasts.success('Healthy', `${kind} reachable`);
      else toasts.error('Unreachable', h.message ?? `${kind} did not respond`);
      return h;
    } catch (e) {
      toasts.error('Health check failed', e instanceof Error ? e.message : String(e));
      return null;
    }
  }

  /** Fetch the install plan (method + steps) for a backend — shown before installing. */
  async planInstall(kind: string): Promise<VaultInstallPlan | null> {
    const wsId = ws.currentId;
    if (!wsId) return null;
    try {
      return await api.post<VaultInstallPlan>(`/workspaces/${wsId}/vault/backends/${kind}/install/plan`, {});
    } catch (e) {
      toasts.error('Could not plan install', e instanceof Error ? e.message : String(e));
      return null;
    }
  }

  /** Run a backend install (deliberate, confirmed action); refreshes the list. */
  async installBackend(kind: string): Promise<VaultInstallResult | null> {
    const wsId = ws.currentId;
    if (!wsId) return null;
    try {
      const r = await api.post<VaultInstallResult>(`/workspaces/${wsId}/vault/backends/${kind}/install`, {});
      if (r.ok) toasts.success('Installed', kind);
      else toasts.error('Install failed', kind);
      await this.loadBackends();
      return r;
    } catch (e) {
      toasts.error('Install failed', e instanceof Error ? e.message : String(e));
      return null;
    }
  }

  // -- Vault v2: repo brain + docs ----------------------------------------

  /** Assemble the Repo Brain for a focus string (markdown + the reasons used). */
  async runBrain(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId || !this.brainFocus.trim()) return;
    this.brainBusy = true;
    try {
      this.brain = await api.post<RepoBrain>(`/workspaces/${wsId}/vault/brain`, {
        focus: this.brainFocus.trim(),
      });
    } catch (e) {
      toasts.error('Brain failed', e instanceof Error ? e.message : String(e));
    } finally {
      this.brainBusy = false;
    }
  }

  /** Add a knowledge doc (optionally linked to a repo + documented symbols). */
  async addDoc(req: VaultDocReq): Promise<Memory | null> {
    const wsId = ws.currentId;
    if (!wsId) return null;
    try {
      const m = await api.post<Memory>(`/workspaces/${wsId}/vault/docs`, req);
      toasts.success('Doc added', m.title);
      await this.load();
      return m;
    } catch (e) {
      toasts.error('Add doc failed', e instanceof Error ? e.message : String(e));
      return null;
    }
  }

  /** Replace one item in `items` in-place (after a state/patch mutation). */
  _replaceItem(updated: Memory): void {
    this.items = this.items.map((m) => (m.id === updated.id ? updated : m));
  }
}

export const vault = new VaultStore();
