// Vault module store — the docs home. One class instance holds the vault
// list + selection, the lazy file tree, the open note (edit/read), panels,
// and status polling (only while the page is visible).

import {
  createVault,
  createVaultFolder,
  deleteVault,
  deleteVaultNote,
  listDocsRuns,
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
import { authedBlobUrl } from '../../lib/api/client';
import { assetPath } from '../../lib/api/vault';
import { ws } from '../../lib/stores/workspace.svelte';
import { toasts } from '../../lib/toast.svelte';

export type LeftMode = 'files' | 'search' | 'tags';
export type CenterMode = 'note' | 'graph' | 'empty' | 'docs-agents' | 'file';

/** One open page in the vault center pane (persisted per vault). */
export interface VaultTab {
  kind: 'note' | 'file';
  path: string;
}

/** Text files bigger than this render a "too large" notice, not the body. */
const MAX_TEXT_FILE = 2_000_000;

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
const TABS_KEY = 'otto_vault_tabs';

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

  // Open tabs — multiple notes/files at once, persisted per vault (survives
  // module switches AND app restarts; restored in select()).
  tabs = $state<VaultTab[]>([]);
  activeTab = $state(-1);

  // Non-markdown file viewer (centerMode 'file').
  filePath = $state<string | null>(null);
  fileText = $state<string | null>(null);
  fileBlobUrl = $state<string | null>(null);
  fileLoading = $state(false);
  fileError = $state('');

  // Search / tags panels.
  searchQuery = $state('');
  searchHits = $state<VaultSearchHit[]>([]);
  searching = $state(false);
  tags = $state<VaultTagCount[]>([]);

  // Quick switcher.
  switcherOpen = $state(false);

  // Docs agents — the SELECTED run (live or history). The view owns the 1.5s
  // poll timer; the run lives here so switching to a note or the graph and
  // back doesn't lose it. `docsRuns` is the server-persisted list (docs +
  // refine, newest-first) — the durable source the view refetches on mount,
  // which is what makes runs survive tab switches AND app restarts.
  docsRun = $state<VaultDocsRun | null>(null);
  docsRuns = $state<VaultDocsRun[]>([]);
  /** Target-folder prefill for the docs-agent form ("Docs agent here"). */
  docsAgentsDir = $state('');
  /** One-shot prompt/skills prefill for the docs-agent form ("Review + fix
   *  docs" on a folder, "Send to agent to fix" on a findings run). The view
   *  consumes it on open. */
  docsAgentsPrefill = $state<{ prompt: string; skills: string[] } | null>(null);
  /** One-shot "Review + fix" request for a note: NoteView auto-opens the
   *  refine drawer and the drawer auto-sends this prompt for the given path. */
  pendingRefine = $state<{ path: string; prompt: string } | null>(null);

  /** Runs still moving — drives the topbar "agents running" chip. */
  get activeDocsRuns(): VaultDocsRun[] {
    return this.docsRuns.filter((r) =>
      ['running', 'summarizing', 'reviewing', 'revising'].includes(r.state),
    );
  }

  /** Provider label for a `_drafts/docs-run-<run8>/agent-<n>` folder row —
   *  the tree shows WHICH agent wrote each drafts folder. */
  draftAgentLabel(path: string): string | null {
    const m = /^_drafts\/docs-run-([A-Za-z0-9]+)\/agent-(\d+)$/.exec(path);
    if (!m) return null;
    const run = this.docsRuns.find((r) => r.id.startsWith(m[1]));
    const a = run?.agents[Number(m[2]) - 1];
    if (!a) return null;
    return a.model ? `${a.provider} · ${a.model}` : a.provider;
  }

  /** Refresh the persisted runs list (newest-first; live runs overlaid). */
  async refreshDocsRuns(): Promise<void> {
    if (!this.current) return;
    const vaultId = this.current.id;
    const wasActive = new Set(this.activeDocsRuns.map((r) => r.id));
    try {
      const runs = await listDocsRuns(this.wsId, vaultId);
      // Async guard: drop the result if the vault changed under us.
      if (this.current?.id !== vaultId) return;
      this.docsRuns = runs;
      // A run just finished (possibly launched outside this UI, e.g. over
      // MCP) — its drafts were consolidated/trashed, so refresh the tree.
      const finished = runs.some(
        (r) => wasActive.has(r.id) && r.state !== 'running' && r.state !== 'summarizing',
      );
      if (finished) {
        void this.refreshTree();
        void this.refreshStatus();
      }
    } catch {
      /* transient — the view's poll retries */
    }
  }

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
      // Vaults are GLOBAL (the ws in the URL is auth context only) — the
      // last-vault choice and per-vault view keys are ws-independent too.
      const lastId = Number(
        localStorage.getItem(LAST_VAULT_KEY) ??
          localStorage.getItem(`${LAST_VAULT_KEY}:${this.wsId}`) ??
          0,
      );
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
    // Re-entering the SAME vault (module switch / workspace reload): keep the
    // whole view — open tabs, note, center mode — and just refresh the data.
    // Resetting here is what used to lose the user's page on every tab switch.
    if (v && this.current?.id === v.id && this.roots.length > 0) {
      this.current = v;
      void this.refreshStatus();
      void this.refreshDocsRuns();
      return;
    }
    this.current = v;
    this.note = null;
    this.notePath = null;
    this.backlinks = [];
    this.okfReport = null;
    this.roots = [];
    this.docsRun = null;
    this.docsRuns = [];
    this.docsAgentsDir = '';
    this.tabs = [];
    this.activeTab = -1;
    this.clearFileView();
    this.centerMode = 'empty';
    if (!v) return;
    localStorage.setItem(LAST_VAULT_KEY, String(v.id));
    this.editing = localStorage.getItem(`${VIEW_MODE_KEY}:${v.id}`) === 'edit';
    await Promise.all([this.loadRoot(), this.refreshStatus(), this.refreshDocsRuns()]);
    void this.loadTags();
    await this.restoreView();
  }

  startPolling(): void {
    this.stopPolling();
    this.pollTimer = setInterval(() => {
      if (this.current && !document.hidden) {
        void this.refreshStatus();
        // Keep the runs list fresh even when the Docs agent view isn't
        // mounted — the topbar chip is the always-visible signal that
        // agents are writing into this vault right now.
        void this.refreshDocsRuns();
      }
    }, 5000);
  }

  stopPolling(): void {
    if (this.pollTimer) clearInterval(this.pollTimer);
    this.pollTimer = null;
  }

  async refreshStatus(): Promise<void> {
    if (!this.current) return;
    try {
      const prev = this.status;
      this.status = await vaultStatus(this.wsId, this.current.id);
      // Refresh the visible tree when a scan just finished — OR when the
      // note/link counts moved without us ever OBSERVING a 'scanning' state:
      // agents writing over MCP trigger fast server-side scans that complete
      // between two polls, which used to leave the tree stale until a manual
      // refresh.
      const scanFinished = prev?.scan_state === 'scanning' && this.status.scan_state === 'idle';
      const countsMoved =
        prev != null &&
        (prev.notes !== this.status.notes ||
          prev.links !== this.status.links ||
          prev.unresolved !== this.status.unresolved);
      if (scanFinished || countsMoved) {
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

  private treeRefreshing = false;

  /** Refresh every loaded level (after create/rename/delete or a scan).
   *  Overlap-guarded: during an agent write burst the 5s status poll can
   *  request refreshes faster than a deep tree walks — drop, don't stack
   *  (the next poll refreshes again anyway). */
  async refreshTree(): Promise<void> {
    if (!this.current || this.treeRefreshing) return;
    this.treeRefreshing = true;
    try {
      await this.refreshTreeInner();
    } finally {
      this.treeRefreshing = false;
    }
  }

  private async refreshTreeInner(): Promise<void> {
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

  async open(path: string, opts: { edit?: boolean; newTab?: boolean } = {}): Promise<void> {
    if (!this.current) return;
    this.claimTab({ kind: 'note', path }, opts.newTab);
    await this.openNoteInPlace(path, opts);
    this.persistView();
  }

  /** Load a note into the center pane WITHOUT touching tab bookkeeping. */
  private async openNoteInPlace(path: string, opts: { edit?: boolean } = {}): Promise<void> {
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

  // -- tabs (persisted view) -----------------------------------------------------

  /** Activate an existing tab for `t`, replace the active tab, or push a new
   *  one. `newTab` forces a fresh tab (⌘/middle-click). */
  private claimTab(t: VaultTab, newTab = false): void {
    const existing = this.tabs.findIndex((x) => x.kind === t.kind && x.path === t.path);
    if (existing >= 0) {
      this.activeTab = existing;
      return;
    }
    if (newTab || this.activeTab < 0 || this.tabs.length === 0) {
      this.tabs = [...this.tabs, t];
      this.activeTab = this.tabs.length - 1;
    } else {
      const tabs = [...this.tabs];
      tabs[this.activeTab] = t;
      this.tabs = tabs;
    }
  }

  async activateTab(i: number): Promise<void> {
    const t = this.tabs[i];
    if (!t) return;
    this.activeTab = i;
    if (t.kind === 'note') await this.openNoteInPlace(t.path);
    else await this.loadFile(t.path);
    this.persistView();
  }

  async closeTab(i: number): Promise<void> {
    const t = this.tabs[i];
    if (!t) return;
    const wasActive = i === this.activeTab;
    this.tabs = this.tabs.filter((_, x) => x !== i);
    if (this.activeTab > i) this.activeTab -= 1;
    if (wasActive) {
      if (this.tabs.length > 0) {
        await this.activateTab(Math.min(i, this.tabs.length - 1));
        return; // activateTab persisted
      }
      this.activeTab = -1;
      this.note = null;
      this.notePath = null;
      this.clearFileView();
      if (this.centerMode === 'note' || this.centerMode === 'file') this.centerMode = 'empty';
    }
    this.persistView();
  }

  private viewKey(): string {
    return `${TABS_KEY}:${this.current?.id ?? 0}`;
  }

  /** Persist tabs + active tab + center mode — the "come back to my page"
   *  contract, including across full app restarts. */
  persistView(): void {
    if (!this.current) return;
    localStorage.setItem(
      this.viewKey(),
      JSON.stringify({ tabs: this.tabs, active: this.activeTab, mode: this.centerMode }),
    );
  }

  private async restoreView(): Promise<void> {
    if (!this.current) return;
    let saved: { tabs?: unknown; active?: unknown; mode?: unknown } = {};
    try {
      saved = JSON.parse(localStorage.getItem(this.viewKey()) ?? '{}');
    } catch {
      /* corrupt blob — start clean */
    }
    const tabs = (Array.isArray(saved.tabs) ? saved.tabs : []).filter(
      (t): t is VaultTab =>
        !!t &&
        typeof t === 'object' &&
        ((t as VaultTab).kind === 'note' || (t as VaultTab).kind === 'file') &&
        typeof (t as VaultTab).path === 'string',
    );
    this.tabs = tabs;
    const active = typeof saved.active === 'number' ? saved.active : tabs.length - 1;
    this.activeTab = Math.min(Math.max(active, -1), tabs.length - 1);
    if (saved.mode === 'graph') {
      this.centerMode = 'graph';
    } else if (saved.mode === 'docs-agents') {
      this.centerMode = 'docs-agents';
    } else if (this.activeTab >= 0) {
      await this.activateTab(this.activeTab);
    }
  }

  // -- non-markdown file viewer ---------------------------------------------------

  async openFile(path: string, opts: { newTab?: boolean } = {}): Promise<void> {
    if (!this.current) return;
    // Leaving a dirty note for a file view must not lose the edit.
    if (this.dirty && this.notePath && !this.conflict) await this.saveNow();
    this.claimTab({ kind: 'file', path }, opts.newTab);
    await this.loadFile(path);
    this.persistView();
  }

  private clearFileView(): void {
    if (this.fileBlobUrl) URL.revokeObjectURL(this.fileBlobUrl);
    this.filePath = null;
    this.fileText = null;
    this.fileBlobUrl = null;
    this.fileError = '';
    this.fileLoading = false;
  }

  private async loadFile(path: string): Promise<void> {
    if (!this.current) return;
    const vaultId = this.current.id;
    this.clearFileView();
    this.filePath = path;
    this.fileLoading = true;
    this.centerMode = 'file';
    try {
      const url = await authedBlobUrl(assetPath(this.wsId, vaultId, path));
      // Async guards: user may have switched files/vaults while fetching.
      if (this.filePath !== path || this.current?.id !== vaultId) {
        URL.revokeObjectURL(url);
        return;
      }
      this.fileBlobUrl = url;
      if (!/\.(png|jpe?g|gif|webp|svg|avif|bmp|ico|pdf)$/i.test(path)) {
        const blob = await (await fetch(url)).blob();
        if (this.filePath !== path) return;
        if (blob.size > MAX_TEXT_FILE) {
          this.fileError = `File is too large to display (${(blob.size / 1024 / 1024).toFixed(1)} MB)`;
        } else {
          this.fileText = await blob.text();
        }
      }
    } catch (e) {
      if (this.filePath === path) this.fileError = msg(e);
    } finally {
      if (this.filePath === path) this.fileLoading = false;
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
      // Keep open tabs pointing at the moved path (file or whole folder).
      this.tabs = this.tabs.map((t) =>
        t.path === from
          ? { ...t, path: to }
          : t.path.startsWith(`${from}/`)
            ? { ...t, path: to + t.path.slice(from.length) }
            : t,
      );
      this.persistView();
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
      const i = this.tabs.findIndex((t) => t.path === path || t.path.startsWith(`${path}/`));
      if (i >= 0) await this.closeTab(i);
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
    this.persistView();
  }

  /** "Review + fix docs" on a bundle folder → docs-agent form prefilled with a
   *  review-and-repair prompt scoped to that folder. */
  reviewFixBundle(dir: string): void {
    this.docsAgentsPrefill = {
      prompt:
        `REVIEW AND FIX the existing documentation bundle \`${dir}/\` in this vault — ` +
        `repair in place, do NOT rewrite it from scratch.\n\n` +
        `1. Read the bundle's index.md, coverage.md and overview.md; locate the source ` +
        `repository from overview.md's frontmatter (repo:/commit:).\n` +
        `2. Verify the docs against the CURRENT code: every claim, citation (file:line), ` +
        `example, link, table, diagram and OpenAPI operation.\n` +
        `3. FIX in place: wrong or stale claims; placeholder/meta examples (replace with ` +
        `REAL bodies from the code — actual field names, plausible values); missing ` +
        `flow-note sections (trigger, numbered steps naming each store as engine + ` +
        `table/collection, request/response examples, failure/retry, one diagram); broken ` +
        `mermaid/d2 fences; dangling links (forward links into not-yet-scanned dependency ` +
        `bundles are FINE — leave those); coverage rows that no longer match reality.\n` +
        `4. Run the OKF validator and the staged audit script; resolve every finding.\n\n` +
        `Finish with a one-line summary of what was fixed.`,
      skills: ['vault-repo-docs'],
    };
    this.openDocsAgents(dir);
  }

  /** "Send to agent…" on a folder — free-form instruction scoped to the dir
   *  (e.g. "split the flows into groups with proper indexing"). */
  sendDirToAgent(dir: string, instruction: string): void {
    this.docsAgentsPrefill = {
      prompt:
        `Work on the folder \`${dir}/\` of this vault. Read its index and notes first; ` +
        `keep every note's content source-backed (never drop citations, examples or ` +
        `diagrams while reorganizing), and keep all links — inbound and outbound — ` +
        `resolving when you move or rename notes (update index.md files accordingly).\n\n` +
        `Instruction:\n${instruction}\n\n` +
        `Finish with a one-line summary of what changed.`,
      skills: [],
    };
    this.openDocsAgents(dir);
  }

  /** Multi-select → one agent request over the group (canned review+fix or a
   *  free-form instruction). */
  sendGroupToAgent(paths: string[], instruction: string | null): void {
    const list = paths.map((p) => `- \`${p}\``).join('\n');
    const task = instruction
      ? `Instruction:\n${instruction}`
      : `REVIEW AND FIX each of these notes in place, treating them as ONE coherent set: ` +
        `verify every claim, citation (file:line), example and link against the CURRENT ` +
        `source code; replace placeholder/meta examples with REAL bodies from the code; ` +
        `fill missing required sections; fix broken mermaid/d2 fences; align terminology ` +
        `and cross-links BETWEEN these notes.`;
    this.docsAgentsPrefill = {
      prompt:
        `Work on this specific GROUP of notes in the vault — these and ONLY these:\n` +
        `${list}\n\n${task}\n\n` +
        `Finish with a one-line summary per note changed.`,
      skills: ['vault-repo-docs'],
    };
    this.openDocsAgents();
  }

  /** "Review + fix" on a single note → open it and auto-send a refine turn. */
  reviewFixNote(path: string): void {
    this.pendingRefine = {
      path,
      prompt:
        `Review this note against the source repository it documents, then FIX it in place: ` +
        `verify every claim, citation (file:line), example and link against the CURRENT code ` +
        `and correct anything wrong or stale; replace placeholder/meta examples with REAL ` +
        `bodies lifted from the code (actual field names, plausible values); fill missing ` +
        `required sections (flow notes: trigger, numbered steps naming each store as engine + ` +
        `table/collection, request/response examples, failure/retry, one diagram); verify ` +
        `every mermaid/d2 fence parses and fix or simplify broken ones; keep it dense. ` +
        `Reply with a one-line summary of what you fixed.`,
    };
    void this.open(path);
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
