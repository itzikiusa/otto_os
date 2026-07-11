// Vault module store — the docs home. One class instance holds the vault
// list + selection, the lazy file tree, the open note (edit/read), panels,
// and status polling (only while the page is visible).

import {
  createVault,
  createVaultFolder,
  deleteVault,
  deleteVaultNote,
  listVaults,
  okfIndexes,
  okfValidate,
  patchVault,
  renameVaultPath,
  rescanVault,
  vaultBacklinks,
  vaultDir,
  vaultNote,
  vaultSearch,
  vaultStatus,
  vaultSwitcher,
  vaultTags,
  writeVaultNote,
} from '../../lib/api/vault';
import { ApiError } from '../../lib/api/client';
import type {
  OkfReport,
  Vault,
  VaultBacklink,
  VaultDirEntry,
  VaultDocsRun,
  VaultNote,
  VaultSearchHit,
  VaultStatus,
  VaultSwitchHit,
  VaultTagCount,
} from '../../lib/api/types';
import { ws } from '../../lib/stores/workspace.svelte';
import { toasts } from '../../lib/toast.svelte';

export type LeftMode = 'files' | 'search' | 'tags';
export type CenterMode = 'note' | 'graph' | 'empty' | 'docs-agents';

/** A row of the flattened, lazily-loaded file tree. */
export interface TreeNode {
  entry: VaultDirEntry;
  depth: number;
  open: boolean;
  loaded: boolean;
  loading: boolean;
  children: TreeNode[];
}

const LAST_VAULT_KEY = 'otto_vault_last';
const VIEW_MODE_KEY = 'otto_vault_view';

class VaultStore {
  vaults = $state<Vault[]>([]);
  current = $state<Vault | null>(null);
  status = $state<VaultStatus | null>(null);
  loading = $state(false);

  leftMode = $state<LeftMode>('files');
  centerMode = $state<CenterMode>('empty');

  // File tree (roots of the current vault).
  roots = $state<TreeNode[]>([]);

  // Open note.
  note = $state<VaultNote | null>(null);
  notePath = $state<string | null>(null);
  editing = $state(false);
  draft = $state('');
  dirty = $state(false);
  saving = $state(false);
  /** 409 conflict from autosave — the banner offers Reload / Overwrite. */
  conflict = $state(false);
  backlinks = $state<VaultBacklink[]>([]);

  // Search / tags panels.
  searchQuery = $state('');
  searchHits = $state<VaultSearchHit[]>([]);
  searching = $state(false);
  tags = $state<VaultTagCount[]>([]);

  // Quick switcher.
  switcherOpen = $state(false);

  // Docs agents — the current (or last) multi-writer documentation run. The
  // view owns the 1.5s poll timer; the run lives here so switching to a note
  // or the graph and back doesn't lose it.
  docsRun = $state<VaultDocsRun | null>(null);
  /** Target-folder prefill for the docs-agent form ("Docs agent here"). */
  docsAgentsDir = $state('');

  // OKF.
  okfReport = $state<OkfReport | null>(null);
  okfBusy = $state(false);

  private pollTimer: ReturnType<typeof setInterval> | null = null;
  private saveTimer: ReturnType<typeof setTimeout> | null = null;

  get wsId(): string {
    return ws.current?.id ?? '';
  }

  // -- lifecycle -------------------------------------------------------------

  async load(): Promise<void> {
    if (!this.wsId) return;
    this.loading = true;
    try {
      this.vaults = await listVaults(this.wsId);
      const lastId = Number(localStorage.getItem(`${LAST_VAULT_KEY}:${this.wsId}`) || 0);
      const pick = this.vaults.find((v) => v.id === lastId) ?? this.vaults[0] ?? null;
      if (pick) await this.select(pick.id);
      else {
        this.current = null;
        this.centerMode = 'empty';
      }
    } catch (e) {
      toasts.error(`Vault: ${msg(e)}`);
    } finally {
      this.loading = false;
    }
  }

  async select(id: number): Promise<void> {
    const v = this.vaults.find((x) => x.id === id) ?? null;
    this.current = v;
    this.note = null;
    this.notePath = null;
    this.backlinks = [];
    this.okfReport = null;
    this.roots = [];
    this.docsRun = null;
    this.docsAgentsDir = '';
    this.centerMode = 'empty';
    if (!v) return;
    localStorage.setItem(`${LAST_VAULT_KEY}:${this.wsId}`, String(v.id));
    this.editing = localStorage.getItem(`${VIEW_MODE_KEY}:${v.id}`) === 'edit';
    await Promise.all([this.loadRoot(), this.refreshStatus()]);
    void this.loadTags();
  }

  startPolling(): void {
    this.stopPolling();
    this.pollTimer = setInterval(() => {
      if (this.current && !document.hidden) void this.refreshStatus();
    }, 5000);
  }

  stopPolling(): void {
    if (this.pollTimer) clearInterval(this.pollTimer);
    this.pollTimer = null;
  }

  async refreshStatus(): Promise<void> {
    if (!this.current) return;
    try {
      const prev = this.status?.scan_state;
      this.status = await vaultStatus(this.wsId, this.current.id);
      // A scan just finished → refresh the visible tree + note backlinks.
      if (prev === 'scanning' && this.status.scan_state === 'idle') {
        void this.refreshTree();
        void this.loadTags();
        if (this.notePath) void this.reloadBacklinks();
      }
    } catch {
      /* transient — next poll retries */
    }
  }

  // -- vault management --------------------------------------------------------

  async create(name: string, rootPath: string | undefined, okf: boolean): Promise<void> {
    const v = await createVault(this.wsId, { name, root_path: rootPath, okf });
    this.vaults = [...this.vaults, v];
    await this.select(v.id);
    toasts.success(`Vault "${v.name}" registered`);
  }

  async unregister(id: number): Promise<void> {
    await deleteVault(this.wsId, id);
    this.vaults = this.vaults.filter((v) => v.id !== id);
    if (this.current?.id === id) await this.load();
    toasts.success('Vault unregistered (files untouched)');
  }

  async toggleOkf(): Promise<void> {
    if (!this.current) return;
    const v = await patchVault(this.wsId, this.current.id, { okf: !this.current.okf });
    this.vaults = this.vaults.map((x) => (x.id === v.id ? v : x));
    this.current = v;
  }

  async rescan(): Promise<void> {
    if (!this.current) return;
    try {
      this.status = await rescanVault(this.wsId, this.current.id);
      await this.refreshTree();
      toasts.success('Vault rescanned');
    } catch (e) {
      toasts.error(`Rescan: ${msg(e)}`);
    }
  }

  // -- file tree ----------------------------------------------------------------

  async loadRoot(): Promise<void> {
    if (!this.current) return;
    const listing = await vaultDir(this.wsId, this.current.id, '');
    this.roots = mergeLevel(this.roots, listing.entries, 0);
  }

  async toggleDir(node: TreeNode): Promise<void> {
    node.open = !node.open;
    if (node.open && !node.loaded && this.current) {
      node.loading = true;
      try {
        const listing = await vaultDir(this.wsId, this.current.id, node.entry.path);
        node.children = mergeLevel(node.children, listing.entries, node.depth + 1);
        node.loaded = true;
      } finally {
        node.loading = false;
      }
    }
  }

  collapseAll(): void {
    const visit = (nodes: TreeNode[]) => {
      for (const n of nodes) {
        n.open = false;
        visit(n.children);
      }
    };
    visit(this.roots);
  }

  /** Depth-first flatten of the open tree for the virtualized list. */
  flatTree(): TreeNode[] {
    const out: TreeNode[] = [];
    const visit = (nodes: TreeNode[]) => {
      for (const n of nodes) {
        out.push(n);
        if (n.entry.kind === 'dir' && n.open) visit(n.children);
      }
    };
    visit(this.roots);
    return out;
  }

  /** Refresh every loaded level (after create/rename/delete or a scan). */
  async refreshTree(): Promise<void> {
    if (!this.current) return;
    const id = this.current.id;
    const refresh = async (nodes: TreeNode[], path: string, depth: number): Promise<TreeNode[]> => {
      const listing = await vaultDir(this.wsId, id, path);
      const merged = mergeLevel(nodes, listing.entries, depth);
      for (const n of merged) {
        if (n.entry.kind === 'dir' && n.loaded) {
          n.children = await refresh(n.children, n.entry.path, depth + 1);
        }
      }
      return merged;
    };
    this.roots = await refresh(this.roots, '', 0);
  }

  // -- note open / edit / save -----------------------------------------------------

  async open(path: string, opts: { edit?: boolean } = {}): Promise<void> {
    if (!this.current) return;
    if (this.dirty && this.notePath && !this.conflict) await this.saveNow();
    try {
      const n = await vaultNote(this.wsId, this.current.id, path);
      this.note = n;
      this.notePath = path;
      this.draft = n.raw;
      this.dirty = false;
      this.conflict = false;
      this.centerMode = 'note';
      if (opts.edit !== undefined) this.setView(opts.edit);
      void this.reloadBacklinks();
    } catch (e) {
      toasts.error(`Open ${path}: ${msg(e)}`);
    }
  }

  async reloadBacklinks(): Promise<void> {
    if (!this.current || !this.notePath) return;
    try {
      this.backlinks = await vaultBacklinks(this.wsId, this.current.id, this.notePath);
    } catch {
      this.backlinks = [];
    }
  }

  setView(edit: boolean): void {
    this.editing = edit;
    if (this.current) {
      localStorage.setItem(`${VIEW_MODE_KEY}:${this.current.id}`, edit ? 'edit' : 'read');
    }
  }

  onDraftChange(content: string): void {
    this.draft = content;
    this.dirty = content !== (this.note?.raw ?? '');
    if (this.saveTimer) clearTimeout(this.saveTimer);
    if (this.dirty && !this.conflict) {
      this.saveTimer = setTimeout(() => void this.saveNow(), 800);
    }
  }

  async saveNow(overwrite = false): Promise<void> {
    if (!this.current || !this.notePath || !this.note || this.saving) return;
    if (!this.dirty && !overwrite) return;
    this.saving = true;
    const savedDraft = this.draft;
    try {
      await writeVaultNote(this.wsId, this.current.id, {
        path: this.notePath,
        content: savedDraft,
        if_hash: overwrite ? undefined : this.note.meta.hash,
      });
      // Refresh meta + outgoing from the index (the write ran a scan).
      const n = await vaultNote(this.wsId, this.current.id, this.notePath);
      this.note = n;
      this.dirty = this.draft !== savedDraft;
      this.conflict = false;
      void this.reloadBacklinks();
      void this.refreshStatus();
    } catch (e) {
      if (e instanceof ApiError && e.status === 409) {
        this.conflict = true;
      } else {
        toasts.error(`Save: ${msg(e)}`);
      }
    } finally {
      this.saving = false;
    }
  }

  /** Conflict banner: discard local edits and reload the disk version. */
  async conflictReload(): Promise<void> {
    if (this.notePath) {
      this.dirty = false;
      this.conflict = false;
      await this.open(this.notePath);
    }
  }

  /** Conflict banner: force-write the local draft over the disk version. */
  async conflictOverwrite(): Promise<void> {
    this.conflict = false;
    this.dirty = true;
    await this.saveNow(true);
  }

  // -- note operations ---------------------------------------------------------------

  async createNote(path: string, content: string): Promise<void> {
    if (!this.current) return;
    await writeVaultNote(this.wsId, this.current.id, { path, content, if_hash: '' });
    await this.refreshTree();
    await this.open(path, { edit: true });
  }

  async createFolder(path: string): Promise<void> {
    if (!this.current) return;
    await createVaultFolder(this.wsId, this.current.id, path);
    await this.refreshTree();
  }

  async rename(from: string, to: string): Promise<void> {
    if (!this.current) return;
    try {
      const r = await renameVaultPath(this.wsId, this.current.id, from, to);
      toasts.success(r.links_updated === 1 ? '1 link updated' : `${r.links_updated} links updated`);
      if (this.notePath === from) {
        await this.open(to);
      } else if (this.notePath?.startsWith(`${from}/`)) {
        await this.open(to + this.notePath.slice(from.length));
      }
      await this.refreshTree();
    } catch (e) {
      toasts.error(`Rename: ${msg(e)}`);
    }
  }

  async trash(path: string): Promise<void> {
    if (!this.current) return;
    try {
      await deleteVaultNote(this.wsId, this.current.id, path);
      toasts.success(`Moved to .trash: ${path}`);
      if (this.notePath === path) {
        this.note = null;
        this.notePath = null;
        this.centerMode = 'empty';
      }
      await this.refreshTree();
      void this.refreshStatus();
    } catch (e) {
      toasts.error(`Delete: ${msg(e)}`);
    }
  }

  // -- docs agents -------------------------------------------------------------------

  /** Open the docs-agent center view, optionally prefilled with a folder. */
  openDocsAgents(dir = ''): void {
    this.docsAgentsDir = dir;
    this.centerMode = 'docs-agents';
  }

  // -- search / tags / switcher ---------------------------------------------------------

  async runSearch(): Promise<void> {
    if (!this.current) return;
    const q = this.searchQuery.trim();
    if (!q) {
      this.searchHits = [];
      return;
    }
    this.searching = true;
    try {
      this.searchHits = await vaultSearch(this.wsId, this.current.id, { query: q, limit: 50 });
    } catch (e) {
      toasts.error(`Search: ${msg(e)}`);
    } finally {
      this.searching = false;
    }
  }

  searchTag(tag: string): void {
    this.leftMode = 'search';
    this.searchQuery = `tag:${tag}`;
    void this.runSearch();
  }

  async loadTags(): Promise<void> {
    if (!this.current) return;
    try {
      this.tags = await vaultTags(this.wsId, this.current.id);
    } catch {
      this.tags = [];
    }
  }

  async switcherQuery(q: string): Promise<VaultSwitchHit[]> {
    if (!this.current) return [];
    try {
      return await vaultSwitcher(this.wsId, this.current.id, q);
    } catch {
      return [];
    }
  }

  // -- OKF ---------------------------------------------------------------------------

  async validateOkf(): Promise<void> {
    if (!this.current) return;
    this.okfBusy = true;
    try {
      this.okfReport = await okfValidate(this.wsId, this.current.id);
    } catch (e) {
      toasts.error(`OKF validate: ${msg(e)}`);
    } finally {
      this.okfBusy = false;
    }
  }

  async generateIndexes(): Promise<void> {
    if (!this.current) return;
    this.okfBusy = true;
    try {
      const r = await okfIndexes(this.wsId, this.current.id);
      toasts.success(`${r.written} index.md files written`);
      await this.refreshTree();
      await this.validateOkf();
    } catch (e) {
      toasts.error(`OKF indexes: ${msg(e)}`);
    } finally {
      this.okfBusy = false;
    }
  }
}

/** Merge a fresh dir listing into existing nodes, keeping open/loaded state. */
function mergeLevel(prev: TreeNode[], entries: VaultDirEntry[], depth: number): TreeNode[] {
  const old = new Map(prev.map((n) => [n.entry.path, n]));
  return entries.map((e) => {
    const ex = old.get(e.path);
    if (ex) {
      ex.entry = e;
      ex.depth = depth;
      return ex;
    }
    return { entry: e, depth, open: false, loaded: false, loading: false, children: [] };
  });
}

function msg(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

export const vault = new VaultStore();
