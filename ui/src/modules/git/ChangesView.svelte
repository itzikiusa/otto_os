<script lang="ts">
  // Working-tree changes: stage checkboxes, per-file diff, commit composer.
  import { api } from '../../lib/api/client';
  import type {
    DiffResp,
    DraftCommitMessageResp,
    FileChange,
    RepoStatusResp,
  } from '../../lib/api/types';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import DiffViewer from './DiffViewer.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';

  interface Props {
    repoId: string;
    status: RepoStatusResp;
    onstatus: (s: RepoStatusResp) => void;
  }
  let { repoId, status, onstatus }: Props = $props();

  let selectedPath: string | null = $state(null);
  let diff: DiffResp | null = $state(null);
  let diffLoading = $state(false);
  let message = $state('');
  let amend = $state(false);
  let committing = $state(false);
  let drafting = $state(false);

  // Mouse multi-select for batch actions. Plain click selects one (and shows
  // its diff); ⌘/Ctrl-click toggles; Shift-click selects a range. The selection
  // toolbar + right-click menu then act on all selected files.
  let selected = $state<Set<string>>(new Set());
  let lastClicked: string | null = null;

  function selectFile(c: FileChange, e: MouseEvent): void {
    const path = c.path;
    if (e.metaKey || e.ctrlKey) {
      const next = new Set(selected);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      selected = next;
    } else if (e.shiftKey && lastClicked) {
      const paths = status.changes.map((x) => x.path);
      const a = paths.indexOf(lastClicked);
      const b = paths.indexOf(path);
      if (a >= 0 && b >= 0) {
        const [lo, hi] = a <= b ? [a, b] : [b, a];
        const next = new Set(selected);
        for (let i = lo; i <= hi; i++) next.add(paths[i]);
        selected = next;
      }
    } else {
      selected = new Set([path]);
    }
    lastClicked = path;
    selectedPath = path;
  }

  function clearSelection(): void {
    selected = new Set();
  }

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

  // Right-click a file → act on the whole multi-selection if the row is part of
  // it, otherwise just that row.
  function rowMenu(e: MouseEvent, c: FileChange): void {
    const inSelection = selected.has(c.path) && selected.size > 1;
    const targets = inSelection ? [...selected] : [c.path];
    if (!inSelection) {
      selected = new Set([c.path]);
      selectedPath = c.path;
      lastClicked = c.path;
    }
    const n = targets.length;
    const sfx = n > 1 ? ` (${n})` : '';
    const label = n > 1 ? `${n} files` : c.path;
    ctxMenu.show(e, [
      { label: `Stage${sfx}`, action: () => void stagePaths(targets, true) },
      { label: `Unstage${sfx}`, action: () => void stagePaths(targets, false) },
      { separator: true },
      { label: `Discard${sfx}`, icon: 'trash', danger: true, action: () => void discardPaths(targets, label) },
    ]);
  }

  const kindBadge: Record<FileChange['kind'], string> = {
    modified: 'M',
    added: 'A',
    deleted: 'D',
    renamed: 'R',
    untracked: 'U',
    conflicted: '!',
  };

  const stagedCount = $derived(status.changes.filter((c) => c.staged).length);

  // The draft endpoint reads the staged diff (falling back to the full working
  // diff) — neither includes untracked files. So a tree of only-untracked files
  // gives the drafter nothing to work from: enable Draft only when at least one
  // change is staged or is a tracked modification.
  const draftable = $derived(
    status.changes.some((c) => c.staged || c.kind !== 'untracked'),
  );

  $effect(() => {
    // (re)load the diff whenever a file is selected
    const path = selectedPath;
    if (path === null) {
      void loadDiff('working');
      return;
    }
    void loadDiff('working', path);
  });

  async function loadDiff(target: string, focusPath?: string): Promise<void> {
    diffLoading = true;
    try {
      const d = await api.get<DiffResp>(`/repos/${repoId}/diff?target=${encodeURIComponent(target)}`);
      diff = focusPath ? { files: d.files.filter((f) => f.path === focusPath) } : d;
    } catch {
      diff = { files: [] };
    } finally {
      diffLoading = false;
    }
  }

  async function toggleStage(c: FileChange): Promise<void> {
    const op = c.staged ? 'unstage' : 'stage';
    try {
      const s = await api.post<RepoStatusResp>(`/repos/${repoId}/${op}`, { paths: [c.path] });
      onstatus(s);
    } catch (e) {
      toasts.error(`${op} failed`, e instanceof Error ? e.message : String(e));
    }
  }

  async function stageAll(stage: boolean): Promise<void> {
    const paths = status.changes.filter((c) => c.staged !== stage).map((c) => c.path);
    if (paths.length === 0) return;
    try {
      const s = await api.post<RepoStatusResp>(`/repos/${repoId}/${stage ? 'stage' : 'unstage'}`, {
        paths,
      });
      onstatus(s);
    } catch (e) {
      toasts.error('Operation failed', e instanceof Error ? e.message : String(e));
    }
  }

  // Discard (revert) working-tree + staged changes. Destructive → confirm.
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
      if (selectedPath !== null && paths.includes(selectedPath)) {
        selectedPath = null;
        void loadDiff('working');
      }
      toasts.info(`Discarded ${paths.length} file${paths.length === 1 ? '' : 's'}`);
    } catch (e) {
      toasts.error('Discard failed', e instanceof Error ? e.message : String(e));
    }
  }

  function discardOne(c: FileChange): void {
    void discardPaths([c.path], c.path);
  }

  function discardAll(): void {
    void discardPaths(
      status.changes.map((c) => c.path),
      `all ${status.changes.length} changed file${status.changes.length === 1 ? '' : 's'}`,
    );
  }

  // Ask an agent to draft a Conventional Commits message from the staged diff
  // (falls back to the working diff when nothing is staged). Fills the box; the
  // user reviews/edits before committing.
  async function draftMessage(): Promise<void> {
    if (drafting || committing) return;
    drafting = true;
    try {
      const d = await api.post<DraftCommitMessageResp>(
        `/repos/${repoId}/draft-commit-message`,
        {},
      );
      message = d.message;
      toasts.info(
        'Draft ready',
        d.from_staged ? 'From staged changes — review and edit.' : 'From working changes (nothing staged) — review and edit.',
      );
    } catch (e) {
      toasts.error('Draft failed', e instanceof Error ? e.message : String(e));
    } finally {
      drafting = false;
    }
  }

  // ── Mobile (phone) accordion ──────────────────────────────────────────────
  // Stack the file list + composer over the diff as collapsible, independently-
  // scrollable sections. Selecting a file collapses the list so the diff gets
  // the screen; the composer is reachable from its own section header.
  let isMobile = $state(false);
  $effect(() => {
    const mq = window.matchMedia('(max-width: 1024px)');
    const sync = () => (isMobile = mq.matches);
    sync();
    mq.addEventListener('change', sync);
    return () => mq.removeEventListener('change', sync);
  });
  let secFilesOpen = $state(true);

  function selectFileMobile(c: FileChange, e: MouseEvent): void {
    selectFile(c, e);
    if (isMobile && !(e.metaKey || e.ctrlKey || e.shiftKey)) secFilesOpen = false;
  }

  async function commit(): Promise<void> {
    if (committing) return;
    committing = true;
    try {
      const r = await api.post<{ sha: string }>(`/repos/${repoId}/commit`, {
        message,
        amend,
      });
      toasts.success('Committed', r.sha.slice(0, 8));
      message = '';
      amend = false;
      const s = await api.get<RepoStatusResp>(`/repos/${repoId}/status`);
      onstatus(s);
      selectedPath = null;
      void loadDiff('working');
    } catch (e) {
      toasts.error('Commit failed', e instanceof Error ? e.message : String(e));
    } finally {
      committing = false;
    }
  }
</script>

<div class="changes" class:mobile={isMobile}>
  {#if isMobile}
    <button
      class="mob-sec-head"
      onclick={() => (secFilesOpen = !secFilesOpen)}
      aria-expanded={secFilesOpen}
    >
      <Icon name={secFilesOpen ? 'chevronDown' : 'chevronRight'} size={13} />
      <Icon name="file" size={13} />
      <span>Changed files</span>
      <span class="grow"></span>
      <span class="mob-sec-count">{status.changes.length}</span>
    </button>
  {/if}
  <div class="changes-side" class:mob-files-collapsed={isMobile && !secFilesOpen}>
    <div class="cs-head">
      <span class="dim">{status.changes.length} changed · {stagedCount} staged</span>
      <span class="grow"></span>
      <button class="btn small ghost" onclick={() => stageAll(true)}>Stage all</button>
      <button class="btn small ghost" onclick={() => stageAll(false)}>Unstage</button>
      {#if status.changes.length > 0}
        <button class="btn small ghost danger" onclick={discardAll} title="Discard all changes">Discard all</button>
      {/if}
    </div>

    {#if selected.size > 0}
      <div class="cs-selbar">
        <span class="dim">{selected.size} selected</span>
        <span class="grow"></span>
        <button class="btn small ghost" onclick={() => stagePaths([...selected], true)}>Stage</button>
        <button class="btn small ghost" onclick={() => stagePaths([...selected], false)}>Unstage</button>
        <button
          class="btn small ghost danger"
          onclick={() => discardPaths([...selected], `${selected.size} selected file${selected.size === 1 ? '' : 's'}`)}
        >Discard</button>
        <button class="btn small ghost" onclick={clearSelection} title="Clear selection">✕</button>
      </div>
    {/if}

    <div class="cs-list">
      <button class="cs-file" class:selected={selectedPath === null} onclick={() => { selectedPath = null; clearSelection(); }}>
        <span class="grow">All changes</span>
      </button>
      {#each status.changes as c (c.path)}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
          class="cs-file"
          class:selected={selectedPath === c.path}
          class:multi={selected.has(c.path)}
          oncontextmenu={(e) => rowMenu(e, c)}
        >
          <input
            type="checkbox"
            checked={c.staged}
            onchange={() => toggleStage(c)}
            title={c.staged ? 'Unstage' : 'Stage'}
          />
          <button class="cs-name" onclick={(e) => selectFileMobile(c, e)} title={c.path}>
            <span class="kind k-{c.kind}">{kindBadge[c.kind]}</span>
            <span class="mono cs-path">{c.path}</span>
          </button>
          <button
            class="cs-discard"
            title="Discard changes to this file"
            aria-label="Discard {c.path}"
            onclick={() => discardOne(c)}
          >
            <Icon name="trash" size={12} />
          </button>
        </div>
      {:else}
        <div class="dim" style="padding: 14px 10px; font-size: 12px">Working tree clean.</div>
      {/each}
    </div>

    <div class="composer">
      <div class="msg-box">
        <textarea
          class="input"
          rows="3"
          bind:value={message}
          placeholder="Commit message"
          spellcheck="false"
        ></textarea>
        <button
          class="btn small ghost draft-btn"
          disabled={drafting || committing || !draftable}
          onclick={draftMessage}
          title={draftable
            ? 'Draft a commit message from your staged changes'
            : 'Stage a file first — drafting can’t see untracked files'}
        >
          {#if drafting}
            <span class="spinner-xs"></span>Drafting…
          {:else}
            <Icon name="zap" size={11} /> Draft
          {/if}
        </button>
      </div>
      <div class="row">
        <label class="checkbox-row">
          <input type="checkbox" bind:checked={amend} />
          Amend
        </label>
        <span class="grow"></span>
        <button
          class="btn primary"
          disabled={committing || drafting || (message.trim() === '' && !amend) || (stagedCount === 0 && !amend)}
          onclick={commit}
        >
          {committing ? 'Committing…' : `Commit${stagedCount > 0 ? ` (${stagedCount})` : ''}`}
        </button>
      </div>
    </div>
  </div>

  {#if isMobile}
    <button
      class="mob-sec-head mob-diff-head"
      onclick={() => (secFilesOpen = true)}
      aria-expanded="true"
    >
      <Icon name="file" size={13} />
      <span class="mob-diff-title">{selectedPath === null ? 'All changes' : selectedPath.split('/').pop()}</span>
      <span class="grow"></span>
      <span class="mob-back">↑ Files</span>
    </button>
  {/if}
  <div class="changes-diff">
    {#if diffLoading && !diff}
      <Skeleton rows={6} height={28} />
    {:else if diff && diff.files.length > 0}
      <DiffViewer {diff} />
    {:else}
      <EmptyState icon="branch" title="Nothing to diff" body="Select a changed file on the left, or make some changes." />
    {/if}
  </div>
</div>

<style>
  .changes {
    display: flex;
    height: 100%;
    min-height: 0;
  }
  .changes-side {
    width: 300px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    border-inline-end: 1px solid var(--border);
  }
  .cs-head {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 8px 10px;
    font-size: 11px;
    border-bottom: 1px solid var(--border);
  }
  .cs-list {
    flex: 1;
    overflow-y: auto;
    padding: 6px;
  }
  .cs-file {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    border-radius: var(--radius-s);
    padding: 0 6px;
    border: none;
    background: transparent;
    text-align: start;
    cursor: pointer;
  }
  .cs-file.selected {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .cs-file.multi {
    background: color-mix(in srgb, var(--accent) 24%, transparent);
  }
  .cs-selbar {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 5px 10px;
    font-size: 11px;
    background: color-mix(in srgb, var(--accent) 10%, transparent);
    border-bottom: 1px solid var(--border);
  }
  .cs-name {
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
  .cs-path {
    font-size: 11.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: rtl;
    text-align: left;
  }
  .cs-discard {
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
  .cs-file:hover .cs-discard,
  .cs-file.selected .cs-discard {
    opacity: 1;
  }
  .cs-discard:hover {
    color: var(--danger, #e5534b);
    background: color-mix(in srgb, var(--danger, #e5534b) 12%, transparent);
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
    background: color-mix(in srgb, #febc2e 25%, transparent);
    color: #b8860b;
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
  .composer {
    border-top: 1px solid var(--border);
    padding: 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .msg-box {
    position: relative;
  }
  .msg-box textarea {
    width: 100%;
    padding-inline-end: 78px;
  }
  .draft-btn {
    position: absolute;
    top: 6px;
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
  .changes-diff {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    padding: 12px 14px;
  }

  /* ── Mobile accordion section headers ── */
  .mob-sec-head {
    display: none;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 11px 14px;
    border: none;
    border-bottom: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
    font-size: 14px;
    font-weight: 600;
    cursor: pointer;
    text-align: start;
    flex-shrink: 0;
    -webkit-tap-highlight-color: transparent;
  }
  .mob-sec-head:active { background: color-mix(in srgb, var(--accent) 10%, var(--surface-2)); }
  .mob-sec-count {
    font-size: 11px;
    font-weight: 700;
    padding: 1px 7px;
    border-radius: 999px;
    background: var(--surface);
    color: var(--text-dim);
  }
  .mob-diff-head { background: color-mix(in srgb, var(--accent) 12%, var(--surface-2)); }
  .mob-diff-title {
    font-size: 13px;
    color: var(--accent);
    font-weight: 700;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .mob-back { font-size: 12px; color: var(--text-dim); flex-shrink: 0; }

  /* ── Mobile + tablet (≤1024px): stack files+composer over the diff ── */
  @media (max-width: 1024px) {
    .changes.mobile { flex-direction: column; overflow-y: auto; -webkit-overflow-scrolling: touch; }
    .mobile .mob-sec-head { display: flex; }
    .mobile .changes-side {
      width: 100%;
      flex: 0 0 auto;
      border-inline-end: none;
      border-bottom: 1px solid var(--border);
    }
    /* The file list scrolls within a capped height; the composer stays visible
       below it. When collapsed, only the list hides — head + composer remain. */
    .mobile .cs-list { max-height: 40vh; }
    .mobile .changes-side.mob-files-collapsed .cs-list { display: none; }
    .mobile .changes-diff {
      min-width: 0;
      width: 100%;
      flex: 1 1 auto;
      min-height: 45vh;
    }
    /* Bigger touch targets + legible text. */
    .mobile .cs-head { font-size: 13px; gap: 6px; flex-wrap: wrap; }
    .mobile .cs-name { height: 34px; }
    .mobile .cs-path { font-size: 13px; }
    .mobile .cs-file input[type='checkbox'] { width: 17px; height: 17px; }
    .mobile .composer textarea { font-size: 14px; }
    /* Discard button always visible on touch (no hover). */
    .mobile .cs-discard { opacity: 1; }
  }
</style>
