<script lang="ts">
  // GitKraken-style WIP panel: shown in the graph's RIGHT detail pane when the
  // WIP row is selected. Unstaged / Staged file trees (per-file + per-folder
  // stage toggles, discard), a per-file working diff, and the commit composer.
  // Replaces the old separate "Changes" tab — staging now lives on the graph.
  import { api } from '../../lib/api/client';
  import type {
    CommitInfo,
    DiffResp,
    DraftCommitMessageResp,
    FileChange,
    RepoStatusResp,
  } from '../../lib/api/types';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import DiffViewer from './DiffViewer.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';

  interface Props {
    repoId: string;
    status: RepoStatusResp;
    onstatus: (s: RepoStatusResp) => void;
    /** Called after a successful commit so the graph can reload its log. */
    oncommitted: () => void;
    onclose: () => void;
  }
  let { repoId, status, onstatus, oncommitted, onclose }: Props = $props();

  const unstaged = $derived(status.changes.filter((c) => !c.staged));
  const staged = $derived(status.changes.filter((c) => c.staged));

  // ── Commit composer (Summary + Description, joined "subject\n\nbody") ──────
  let subject = $state('');
  let body = $state('');
  let amend = $state(false);
  let committing = $state(false);
  let drafting = $state(false);

  // The draft endpoint reads the staged diff (falling back to the full working
  // diff) — neither includes untracked files, so require something draftable.
  const draftable = $derived(status.changes.some((c) => c.staged || c.kind !== 'untracked'));

  // ── Per-file diff (working tree, server-side scoped) ────────────────────────
  let selectedPath = $state<string | null>(null);
  let diff = $state<DiffResp | null>(null);
  let diffLoading = $state(false);

  $effect(() => {
    const path = selectedPath;
    if (path === null) {
      diff = null;
      return;
    }
    diffLoading = true;
    void api
      .get<DiffResp>(`/repos/${repoId}/diff?target=working&path=${encodeURIComponent(path)}`)
      .then((d) => (diff = d))
      .catch(() => (diff = { files: [] }))
      .finally(() => (diffLoading = false));
  });

  // A staged/unstaged move can remove the selected file from the tree entirely
  // (e.g. discard); drop the diff so it doesn't show a stale file.
  $effect(() => {
    if (selectedPath !== null && !status.changes.some((c) => c.path === selectedPath)) {
      selectedPath = null;
    }
  });

  const kindBadge: Record<FileChange['kind'], string> = {
    modified: 'M',
    added: 'A',
    deleted: 'D',
    renamed: 'R',
    untracked: 'U',
    conflicted: '!',
  };

  async function stagePaths(paths: string[], stage: boolean): Promise<void> {
    if (paths.length === 0) return;
    try {
      const s = await api.post<RepoStatusResp>(
        `/repos/${repoId}/${stage ? 'stage' : 'unstage'}`,
        { paths },
      );
      onstatus(s);
    } catch (e) {
      toasts.error('Operation failed', e instanceof Error ? e.message : String(e));
    }
  }

  async function discardPaths(paths: string[], label: string): Promise<void> {
    if (paths.length === 0) return;
    const ok = await confirmer.ask(
      `Discard changes to ${label}? This reverts ${paths.length === 1 ? 'it' : 'them'} to the last commit (new files are deleted) and cannot be undone.`,
      { title: 'Discard changes', confirmLabel: 'Discard' },
    );
    if (!ok) return;
    try {
      const s = await api.post<RepoStatusResp>(`/repos/${repoId}/discard`, { paths });
      onstatus(s);
      toasts.info(`Discarded ${paths.length} file${paths.length === 1 ? '' : 's'}`);
    } catch (e) {
      toasts.error('Discard failed', e instanceof Error ? e.message : String(e));
    }
  }

  function fileMenu(e: MouseEvent, c: FileChange): void {
    e.preventDefault();
    ctxMenu.show(e, [
      {
        label: c.staged ? 'Unstage' : 'Stage',
        action: () => void stagePaths([c.path], !c.staged),
      },
      { separator: true },
      {
        label: 'Discard',
        icon: 'trash',
        danger: true,
        action: () => void discardPaths([c.path], c.path),
      },
    ]);
  }

  // ── Folder tree (shared shape with the old Changes tab) ────────────────────
  type TFile = { type: 'file'; name: string; change: FileChange };
  type TFolder = {
    type: 'folder';
    name: string;
    path: string;
    children: TNode[];
    files: FileChange[];
  };
  type TNode = TFile | TFolder;

  function sortLevel(nodes: TNode[]): void {
    nodes.sort((a, b) => {
      if (a.type !== b.type) return a.type === 'folder' ? -1 : 1;
      return a.name.localeCompare(b.name);
    });
    for (const n of nodes) if (n.type === 'folder') sortLevel(n.children);
  }

  function buildTree(changes: FileChange[]): TNode[] {
    const rootChildren: TNode[] = [];
    const folders = new Map<string, TFolder>();
    for (const c of changes) {
      const parts = c.path.split('/');
      let children = rootChildren;
      let prefix = '';
      for (let i = 0; i < parts.length - 1; i++) {
        prefix = prefix ? `${prefix}/${parts[i]}` : parts[i];
        let folder = folders.get(prefix);
        if (!folder) {
          folder = { type: 'folder', name: parts[i], path: prefix, children: [], files: [] };
          folders.set(prefix, folder);
          children.push(folder);
        }
        folder.files.push(c);
        children = folder.children;
      }
      children.push({ type: 'file', name: parts[parts.length - 1], change: c });
    }
    sortLevel(rootChildren);
    return rootChildren;
  }

  const unstagedTree = $derived.by(() => buildTree(unstaged));
  const stagedTree = $derived.by(() => buildTree(staged));

  // Folders collapse by "section:path" so the same folder can fold independently
  // in the Unstaged and Staged trees.
  let collapsed = $state<Set<string>>(new Set());
  function toggleFolder(key: string): void {
    const next = new Set(collapsed);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    collapsed = next;
  }

  // Section collapse (GitKraken keeps both open; folding is still handy).
  let unstagedOpen = $state(true);
  let stagedOpen = $state(true);

  // Amend prefill: ticking Amend with an empty subject pulls HEAD's message in.
  $effect(() => {
    if (!amend || subject.trim() !== '') return;
    void api
      .get<CommitInfo[]>(`/repos/${repoId}/log?limit=1&skip=0`)
      .then((commits) => {
        if (amend && subject.trim() === '' && commits[0]) subject = commits[0].subject;
      })
      .catch(() => {});
  });

  async function draftMessage(): Promise<void> {
    if (drafting || committing) return;
    drafting = true;
    try {
      const d = await api.post<DraftCommitMessageResp>(
        `/repos/${repoId}/draft-commit-message`,
        {},
      );
      const text = d.message.trim();
      const nl = text.indexOf('\n');
      if (nl === -1) {
        subject = text;
        body = '';
      } else {
        subject = text.slice(0, nl).trim();
        body = text.slice(nl + 1).replace(/^\s*\n/, '').trimEnd();
      }
      toasts.info(
        'Draft ready',
        d.from_staged
          ? 'From staged changes — review the summary & description.'
          : 'From working changes (nothing staged) — review & edit.',
      );
    } catch (e) {
      toasts.error('Draft failed', e instanceof Error ? e.message : String(e));
    } finally {
      drafting = false;
    }
  }

  async function commit(): Promise<void> {
    if (committing) return;
    committing = true;
    try {
      const message = subject.trim() + (body.trim() ? `\n\n${body.trim()}` : '');
      const r = await api.post<{ sha: string }>(`/repos/${repoId}/commit`, {
        message,
        amend,
      });
      toasts.success('Committed', r.sha.slice(0, 8));
      subject = '';
      body = '';
      amend = false;
      const s = await api.get<RepoStatusResp>(`/repos/${repoId}/status`);
      onstatus(s);
      selectedPath = null;
      oncommitted();
    } catch (e) {
      toasts.error('Commit failed', e instanceof Error ? e.message : String(e));
    } finally {
      committing = false;
    }
  }
</script>

{#snippet fileRow(file: TFile, depth: number, section: 'unstaged' | 'staged')}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="wp-file"
    class:selected={selectedPath === file.change.path}
    style="padding-inline-start:{8 + depth * 14}px"
    oncontextmenu={(e) => fileMenu(e, file.change)}
  >
    <input
      type="checkbox"
      checked={section === 'staged'}
      onchange={() => stagePaths([file.change.path], section === 'unstaged')}
      title={section === 'staged' ? 'Unstage' : 'Stage'}
      aria-label="{section === 'staged' ? 'Unstage' : 'Stage'} {file.change.path}"
    />
    <button
      class="wp-name"
      onclick={() => (selectedPath = selectedPath === file.change.path ? null : file.change.path)}
      title={file.change.path}
    >
      <span class="kind k-{file.change.kind}">{kindBadge[file.change.kind]}</span>
      <span class="mono wp-fname">{file.name}</span>
    </button>
    <button
      class="wp-discard"
      title="Discard changes to this file"
      aria-label="Discard {file.change.path}"
      onclick={() => void discardPaths([file.change.path], file.change.path)}
    >
      <Icon name="trash" size={12} />
    </button>
  </div>
{/snippet}

{#snippet treeNode(node: TNode, depth: number, section: 'unstaged' | 'staged')}
  {#if node.type === 'folder'}
    {@const key = `${section}:${node.path}`}
    <div class="wp-folder" style="padding-inline-start:{8 + depth * 14}px">
      <button
        class="wp-fold-toggle"
        onclick={() => toggleFolder(key)}
        aria-label="Toggle folder"
        aria-expanded={!collapsed.has(key)}
      >
        <Icon name={collapsed.has(key) ? 'chevronRight' : 'chevronDown'} size={12} />
      </button>
      <button
        class="wp-fold-name"
        onclick={() =>
          stagePaths(node.files.map((f) => f.path), section === 'unstaged')}
        title="{section === 'staged' ? 'Unstage' : 'Stage'} {node.path}/ ({node.files.length})"
      >
        <Icon name="folder" size={12} />
        <span class="wp-fold-label">{node.name}</span>
        <span class="wp-fold-count">{node.files.length}</span>
      </button>
    </div>
    {#if !collapsed.has(key)}
      {#each node.children as child (child.type === 'folder' ? `d:${child.path}` : `f:${child.change.path}`)}
        {@render treeNode(child, depth + 1, section)}
      {/each}
    {/if}
  {:else}
    {@render fileRow(node, depth, section)}
  {/if}
{/snippet}

<div class="wip-panel">
  <div class="wp-head">
    <span class="wp-title mono">// WIP</span>
    <span class="wp-count">{status.changes.length} file{status.changes.length === 1 ? '' : 's'} changed</span>
    <span class="grow"></span>
    <button class="wp-close" onclick={onclose} title="Close" aria-label="Close WIP panel">✕</button>
  </div>

  <div class="wp-scroll" class:has-diff={selectedPath !== null}>
    <!-- Unstaged -->
    <div class="wp-section">
      <button class="wp-sec-head" onclick={() => (unstagedOpen = !unstagedOpen)} aria-expanded={unstagedOpen}>
        <Icon name={unstagedOpen ? 'chevronDown' : 'chevronRight'} size={12} />
        <span>Unstaged Files</span>
        <span class="wp-sec-count">{unstaged.length}</span>
        <span class="grow"></span>
        {#if unstaged.length > 0}
          <span
            class="wp-sec-action"
            role="button"
            tabindex="-1"
            onclick={(e) => {
              e.stopPropagation();
              void stagePaths(unstaged.map((c) => c.path), true);
            }}
            onkeydown={(e) => {
              if (e.key === 'Enter' || e.key === ' ') {
                e.stopPropagation();
                void stagePaths(unstaged.map((c) => c.path), true);
              }
            }}
          >Stage all</span>
        {/if}
      </button>
      {#if unstagedOpen}
        <div class="wp-list">
          {#each unstagedTree as node (node.type === 'folder' ? `d:${node.path}` : `f:${node.change.path}`)}
            {@render treeNode(node, 0, 'unstaged')}
          {:else}
            <div class="dim wp-empty">Nothing unstaged.</div>
          {/each}
        </div>
      {/if}
    </div>

    <!-- Staged -->
    <div class="wp-section">
      <button class="wp-sec-head" onclick={() => (stagedOpen = !stagedOpen)} aria-expanded={stagedOpen}>
        <Icon name={stagedOpen ? 'chevronDown' : 'chevronRight'} size={12} />
        <span>Staged Files</span>
        <span class="wp-sec-count">{staged.length}</span>
        <span class="grow"></span>
        {#if staged.length > 0}
          <span
            class="wp-sec-action"
            role="button"
            tabindex="-1"
            onclick={(e) => {
              e.stopPropagation();
              void stagePaths(staged.map((c) => c.path), false);
            }}
            onkeydown={(e) => {
              if (e.key === 'Enter' || e.key === ' ') {
                e.stopPropagation();
                void stagePaths(staged.map((c) => c.path), false);
              }
            }}
          >Unstage all</span>
        {/if}
      </button>
      {#if stagedOpen}
        <div class="wp-list">
          {#each stagedTree as node (node.type === 'folder' ? `d:${node.path}` : `f:${node.change.path}`)}
            {@render treeNode(node, 0, 'staged')}
          {:else}
            <div class="dim wp-empty">Nothing staged yet.</div>
          {/each}
        </div>
      {/if}
    </div>

  </div>

  <!-- Per-file diff: a SIBLING of the tree scroller, not its tail — appended
       inside wp-scroll it sat below every tree row, so on a large changeset
       (hundreds of files) selecting a file loaded the diff far off-screen and
       looked like nothing happened. As its own flex region it is visible the
       instant a file is clicked, with its own scrollbar. -->
  {#if selectedPath !== null}
    <div class="wp-diff">
      <div class="wp-diff-head">
        <Icon name="file" size={12} />
        <span class="mono wp-diff-path" title={selectedPath}>{selectedPath}</span>
        <span class="grow"></span>
        <button class="wp-close" onclick={() => (selectedPath = null)} title="Close diff" aria-label="Close diff">✕</button>
      </div>
      <div class="wp-diff-body">
        {#if diffLoading && !diff}
          <div style="padding: 10px"><Skeleton rows={5} height={20} /></div>
        {:else if diff && diff.files.length > 0}
          <DiffViewer {diff} />
        {:else}
          <div class="dim wp-empty">No textual diff for this file.</div>
        {/if}
      </div>
    </div>
  {/if}

  <!-- Commit composer -->
  <div class="wp-composer">
    <div class="msg-box">
      <input
        class="input subject-input"
        bind:value={subject}
        placeholder="Summary"
        spellcheck="false"
      />
      <button
        class="btn small ghost draft-btn"
        disabled={drafting || committing || !draftable}
        onclick={draftMessage}
        title={draftable
          ? 'Draft a summary + description from your staged changes'
          : 'Stage a file first — drafting can’t see untracked files'}
      >
        {#if drafting}
          <span class="spinner-xs"></span>Drafting…
        {:else}
          <Icon name="zap" size={11} /> Draft
        {/if}
      </button>
    </div>
    <textarea
      class="input body-input"
      rows="2"
      bind:value={body}
      placeholder="Description (optional)"
      spellcheck="false"
    ></textarea>
    <div class="row">
      <label class="checkbox-row">
        <input type="checkbox" bind:checked={amend} />
        Amend
      </label>
      <span class="grow"></span>
      <button
        class="btn primary"
        disabled={committing || drafting || (subject.trim() === '' && !amend) || (staged.length === 0 && !amend)}
        onclick={commit}
      >
        {committing ? 'Committing…' : `Commit${staged.length > 0 ? ` (${staged.length})` : ''}`}
      </button>
    </div>
  </div>
</div>

<style>
  .wip-panel {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .wp-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 12px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .wp-title {
    font-size: 13px;
    font-weight: 700;
    color: var(--accent);
  }
  .wp-count {
    font-size: 11px;
    color: var(--text-dim);
  }
  .wp-close {
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    font-size: 12px;
    padding: 2px 6px;
    border-radius: 4px;
    flex-shrink: 0;
  }
  .wp-close:hover {
    color: var(--text);
    background: var(--surface-2);
  }
  .grow {
    flex: 1;
  }

  .wp-scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
  }
  /* With a diff open the trees yield the pane: capped (but still scrollable)
     so the diff region below is always on-screen. */
  .wp-scroll.has-diff {
    flex: 0 1 auto;
    max-height: 45%;
  }
  .wp-section {
    border-bottom: 1px solid var(--border);
  }
  .wp-sec-head {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 7px 10px;
    border: none;
    background: var(--surface-2);
    color: var(--text);
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.03em;
    cursor: pointer;
    text-align: start;
  }
  .wp-sec-count {
    font-size: 10px;
    font-weight: 700;
    min-width: 16px;
    padding: 0 5px;
    border-radius: 999px;
    background: var(--surface);
    color: var(--text-dim);
    text-align: center;
  }
  .wp-sec-action {
    font-size: 10.5px;
    font-weight: 600;
    color: var(--accent);
    padding: 2px 7px;
    border-radius: var(--radius-s);
  }
  .wp-sec-action:hover {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .wp-list {
    padding: 4px 2px;
  }
  .wp-empty {
    padding: 6px 12px;
    font-size: 11.5px;
  }
  .wp-file {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    border-radius: var(--radius-s);
    padding-inline-end: 6px;
  }
  .wp-file.selected {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .wp-name {
    display: flex;
    align-items: center;
    gap: 7px;
    flex: 1;
    min-width: 0;
    height: 26px;
    border: none;
    background: transparent;
    cursor: pointer;
    color: var(--text);
    text-align: start;
  }
  .wp-fname {
    font-size: 11.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .wp-discard {
    flex-shrink: 0;
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    padding: 3px 5px;
    border-radius: 4px;
    line-height: 1;
    opacity: 0;
    transition: opacity 100ms ease-out;
  }
  .wp-file:hover .wp-discard,
  .wp-file.selected .wp-discard {
    opacity: 1;
  }
  .wp-discard:hover {
    color: var(--status-exited);
    background: color-mix(in srgb, var(--status-exited) 12%, transparent);
  }
  .wp-folder {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    height: 26px;
    padding-inline-end: 6px;
  }
  .wp-folder:hover {
    background: color-mix(in srgb, var(--accent) 7%, transparent);
  }
  .wp-fold-toggle {
    display: grid;
    place-items: center;
    width: 16px;
    height: 16px;
    flex-shrink: 0;
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
  }
  .wp-fold-name {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 1;
    min-width: 0;
    height: 26px;
    border: none;
    background: transparent;
    cursor: pointer;
    color: var(--text);
    text-align: start;
  }
  .wp-fold-label {
    font-size: 12px;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .wp-fold-count {
    font-size: 10px;
    color: var(--text-dim);
    background: var(--surface-2);
    border-radius: 999px;
    padding: 0 6px;
    line-height: 15px;
    flex-shrink: 0;
  }

  .kind {
    width: 15px;
    height: 15px;
    border-radius: 3px;
    font-size: 9.5px;
    font-weight: 700;
    display: grid;
    place-items: center;
    flex-shrink: 0;
  }
  .k-modified {
    background: var(--status-warn-soft);
    color: var(--status-warn);
  }
  .k-added,
  .k-untracked {
    background: color-mix(in srgb, var(--status-working) 22%, transparent);
    color: var(--status-working);
  }
  .k-deleted {
    background: color-mix(in srgb, var(--status-exited) 22%, transparent);
    color: var(--status-exited);
  }
  .k-renamed {
    background: color-mix(in srgb, var(--accent) 22%, transparent);
    color: var(--accent);
  }
  .k-conflicted {
    background: color-mix(in srgb, var(--status-exited) 35%, transparent);
    color: var(--status-exited);
  }

  .wp-diff {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--border);
  }
  .wp-diff-head {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    background: var(--surface-2);
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .wp-diff-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
  }
  .wp-diff-path {
    font-size: 11px;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }

  .wp-composer {
    border-top: 1px solid var(--border);
    padding: 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    flex-shrink: 0;
  }
  .msg-box {
    position: relative;
  }
  .subject-input {
    width: 100%;
    height: 30px;
    padding-inline-end: 78px;
    font-weight: 600;
  }
  .body-input {
    width: 100%;
    resize: vertical;
  }
  .draft-btn {
    position: absolute;
    top: 4px;
    inset-inline-end: 6px;
  }
  .spinner-xs {
    display: inline-block;
    width: 9px;
    height: 9px;
    border: 1.5px solid currentColor;
    border-top-color: transparent;
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
    vertical-align: middle;
    margin-inline-end: 4px;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .checkbox-row {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--text-dim);
    cursor: pointer;
  }
  .dim {
    color: var(--text-dim);
  }
  .mono {
    font-family: var(--font-mono);
  }

  /* ≤1024px (the graph stacks as an accordion): the panel renders full-width
     under the mobile diff header; give touch targets real height. */
  @media (max-width: 1024px) {
    .wp-name,
    .wp-fold-name,
    .wp-folder {
      height: 34px;
    }
    .wp-fname,
    .wp-fold-label {
      font-size: 13px;
    }
    .wp-file input[type='checkbox'] {
      width: 17px;
      height: 17px;
    }
    .wp-discard {
      opacity: 1;
      padding: 6px 7px;
    }
    .subject-input {
      font-size: 16px;
      height: 38px;
    }
    .body-input {
      font-size: 16px;
      line-height: 1.45;
    }
    .wp-composer .btn.primary {
      height: 36px;
      padding: 0 16px;
      font-size: 14px;
    }
  }
</style>
