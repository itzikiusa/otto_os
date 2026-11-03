<script lang="ts">
  // The commit-detail pane's file navigator (GitKraken-style): "N files
  // +A −D", a Path / Tree switch (Tree by default, remembered per viewer),
  // Expand all / Collapse all, and the changed files as a folder tree with
  // per-folder counts or as sorted paths. Picking a file opens ONLY that
  // file's diff (graph-commit-diff.svelte); "View all files" switches back to
  // every diff stacked, and then this component is just the toolbar. Rows
  // reuse the WIP panel's model (wip/wipTree.ts) and are windowed, so a
  // 1,500-file commit mounts what a 15-file one does.
  import type { Attachment } from 'svelte/attachments';
  import type { FileChange, FileDiff } from '../../../lib/api/types';
  import { ListWindow } from '../../../lib/listWindow.svelte';
  import { plural } from '../../../lib/plural';
  import Icon from '../../../lib/components/Icon.svelte';
  import {
    AUTO_COLLAPSE_FILES, KIND_LETTER, KIND_WORD, buildTree, folderPaths, kindOfStatus, listRows, treeRows,
    type TFolder,
  } from '../wip/wipTree';

  interface Props {
    files: FileDiff[];
    stats: Map<string, { add: number; del: number }>;
    totals: { add: number; del: number };
    selected: string | null;
    onselect: (path: string) => void;
    view: 'tree' | 'path';
    /** Every diff stacked instead of one file at a time. */
    all: boolean;
    /** A large commit (files start collapsed in the stacked view). */
    large: boolean;
  }
  let { files, stats, totals, selected, onselect, view = $bindable(), all = $bindable(), large }: Props = $props();

  const changes: FileChange[] = $derived(
    files.map((f) => ({ path: f.path, orig_path: f.old_path, kind: kindOfStatus(f.status), staged: false, unstaged: false })),
  );
  const tree = $derived(buildTree(changes));
  let folds = $state.raw<Map<string, boolean>>(new Map());
  const collapsed = (n: TFolder): boolean => folds.get(n.path) ?? n.files.length > AUTO_COLLAPSE_FILES;
  function toggle(n: TFolder): void {
    folds = new Map(folds).set(n.path, !collapsed(n));
  }
  function setAll(shut: boolean): void {
    folds = new Map(folderPaths(tree).map((p) => [p, shut]));
  }
  const rows = $derived(view === 'tree' ? treeRows(tree, collapsed) : listRows(changes, true));

  const lw = new ListWindow('.cf-row');
  const win = $derived(lw.range(rows.length));
  $effect(() => lw.findRows(() => null, () => rows, (r) => r.node.name));
  $effect(() => {
    void rows.length;
    lw.refresh();
  });

  // Open on an item: the first file in the shown order when nothing (or a
  // file of another commit) is selected.
  $effect(() => {
    if (all || (selected !== null && files.some((f) => f.path === selected))) return;
    const first = rows.find((r) => r.node.type === 'file');
    if (first && first.node.type === 'file') onselect(first.node.change.path);
  });

  /** ↑/↓ (Home/End) move AND open; ←/→ fold a folder (mirrored in RTL). */
  const rowNav: Attachment<HTMLElement> = (el) => {
    const onKey = (e: KeyboardEvent): void => {
      if (e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return;
      const cur = (e.target as HTMLElement | null)?.closest?.<HTMLElement>('.cf-row');
      if (!cur) return;
      const list = Array.from(el.querySelectorAll<HTMLElement>('.cf-row'));
      const i = list.indexOf(cur);
      const rtl = getComputedStyle(el).direction === 'rtl';
      let next: HTMLElement | undefined;
      if (e.key === 'ArrowDown') next = list[i + 1];
      else if (e.key === 'ArrowUp') next = list[i - 1];
      else if (e.key === 'Home') next = list[0];
      else if (e.key === 'End') next = list.at(-1);
      else if ((e.key === 'ArrowRight' || e.key === 'ArrowLeft') && cur.dataset.folder !== undefined) {
        e.preventDefault();
        if (((e.key === 'ArrowRight') !== rtl) !== (cur.getAttribute('aria-expanded') === 'true')) cur.click();
        return;
      } else return;
      e.preventDefault();
      if (!next) return;
      next.focus();
      next.scrollIntoView({ block: 'nearest' });
      if (next.dataset.path) onselect(next.dataset.path);
    };
    el.addEventListener('keydown', onKey);
    return () => el.removeEventListener('keydown', onKey);
  };
  const fmt = (n: number): string => n.toLocaleString();
</script>

<div class="cf">
  <div class="cf-bar diff-summary-bar">
    <span class="cf-count">{plural(files.length, 'file')}</span>
    <span class="cf-add">+{fmt(totals.add)}</span>
    <span class="cf-del">−{fmt(totals.del)}</span>
    <span class="cf-grow"></span>
    <label class="cf-all" title={large ? 'Large commit — the stacked view starts collapsed' : 'Show every file’s diff stacked'}>
      <input type="checkbox" bind:checked={all} /> View all files
    </label>
  </div>
  {#if !all}
    <div class="cf-tools">
      <div class="segmented cf-view" role="group" aria-label="File view">
        <button class:active={view === 'path'} aria-pressed={view === 'path'} onclick={() => (view = 'path')}>Path</button>
        <button class:active={view === 'tree'} aria-pressed={view === 'tree'} onclick={() => (view = 'tree')}>Tree</button>
      </div>
      {#if view === 'tree' && rows.some((r) => r.node.type === 'folder')}
        <span class="cf-grow"></span>
        <button class="btn small ghost" onclick={() => setAll(false)}>Expand all</button>
        <button class="btn small ghost" onclick={() => setAll(true)}>Collapse all</button>
      {/if}
    </div>
    <div class="cf-list" {@attach rowNav}>
      <div {@attach lw.attach}>
        {#if win.top > 0}<div style="height:{win.top}px" aria-hidden="true"></div>{/if}
        {#each rows.slice(win.start, win.end) as r (r.key)}
          {#if r.node.type === 'folder'}
            {@const n = r.node}
            {@const shut = collapsed(n)}
            <button class="cf-row cf-folder" style:--depth={r.depth} data-folder onclick={() => toggle(n)} aria-expanded={!shut} title="{n.path}/ ({plural(n.files.length, 'file')})">
              <Icon name={shut ? 'chevronRight' : 'chevronDown'} size={12} />
              <span class="cf-name" dir="ltr">{n.name}</span>
              <span class="cf-n">{fmt(n.files.length)}</span>
            </button>
          {:else}
            {@const c = r.node.change}
            {@const st = stats.get(c.path)}
            <button
              class="cf-row"
              class:selected={selected === c.path}
              style:--depth={r.depth}
              data-path={c.path}
              aria-current={selected === c.path ? 'true' : undefined}
              onclick={() => onselect(c.path)}
              title={c.orig_path ? `${c.orig_path} → ${c.path}` : c.path}
            >
              <span class="cf-kind k-{c.kind}" aria-hidden="true">{KIND_LETTER[c.kind]}</span>
              <span class="sr-only">{KIND_WORD[c.kind]}</span>
              <span class="cf-name" dir="ltr">{r.node.name}</span>
              {#if view === 'path' && r.node.dir}<span class="cf-dir" dir="ltr">{r.node.dir}</span>{:else}<span class="cf-grow"></span>{/if}
              {#if st && st.add + st.del > 0}
                <span class="cf-stat">{#if st.add}<span class="cf-add">+{fmt(st.add)}</span>{/if}{#if st.del}<span class="cf-del">−{fmt(st.del)}</span>{/if}</span>
              {/if}
            </button>
          {/if}
        {/each}
        {#if win.bottom > 0}<div style="height:{win.bottom}px" aria-hidden="true"></div>{/if}
      </div>
    </div>
  {/if}
</div>

<style>
  .cf {
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
  }
  .cf-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 32px;
    padding-inline: 12px 8px;
    border-block-end: 1px solid var(--border);
    font-size: var(--fs-s);
  }
  .cf-count {
    color: var(--text-dim);
  }
  .cf-add,
  .cf-del {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
  }
  .cf-add {
    color: var(--success);
  }
  .cf-del {
    color: var(--danger);
  }
  .cf-grow {
    flex: 1;
  }
  .cf-all {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--text-dim);
    cursor: pointer;
    white-space: nowrap;
  }
  .cf-tools {
    display: flex;
    align-items: center;
    gap: 4px;
    padding-block: 6px 4px;
    padding-inline: 12px 8px;
  }
  .cf-view > button {
    height: 22px;
    padding: 0 10px;
    font-size: var(--fs-s);
  }
  /* Narrow: the list is capped so the opened file's diff below stays in
     reach; wide (the diff beside it): the list fills its column. */
  .cf-list {
    max-height: min(40vh, 360px);
    overflow-y: auto;
    overscroll-behavior: contain;
    padding: 2px 4px 6px;
  }
  @container cdiff (min-width: 760px) {
    .cf-list {
      flex: 1;
      max-height: none;
    }
  }
  .cf-row {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    height: 28px;
    padding-block: 0;
    padding-inline: calc(8px + var(--depth, 0) * 14px) 8px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font: inherit;
    text-align: start;
    cursor: pointer;
  }
  .cf-row:hover {
    background: var(--hover);
  }
  .cf-row.selected {
    background: var(--accent-soft);
  }
  .cf-row:focus-visible {
    outline: 2px solid var(--accent-line-strong);
    outline-offset: -2px;
  }
  .cf-folder :global(svg) {
    flex-shrink: 0;
    color: var(--text-dim);
  }
  .cf-name {
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-m);
    unicode-bidi: isolate;
  }
  .cf-folder .cf-name {
    font-weight: 500;
  }
  .cf-dir {
    flex: 1 1 0;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-s);
    color: var(--text-dim);
    unicode-bidi: isolate;
  }
  .cf-n {
    flex-shrink: 0;
    margin-inline-start: auto;
    padding: 0 6px;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text-dim);
    font-size: var(--fs-xs);
    line-height: 16px;
    font-variant-numeric: tabular-nums;
  }
  .cf-stat {
    flex-shrink: 0;
    display: inline-flex;
    gap: 4px;
  }
  .cf-kind {
    flex-shrink: 0;
    display: grid;
    place-items: center;
    width: 16px;
    height: 16px;
    border-radius: var(--radius-s);
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    font-weight: 600;
  }
  .k-modified {
    background: var(--warning-soft);
    color: var(--warning);
  }
  .k-added {
    background: var(--success-soft);
    color: var(--success);
  }
  .k-deleted {
    background: var(--danger-soft);
    color: var(--danger);
  }
  .k-renamed {
    background: var(--info-soft);
    color: var(--info);
  }
  @media (max-width: 1024px) {
    .cf-row {
      height: 36px;
    }
  }
</style>
