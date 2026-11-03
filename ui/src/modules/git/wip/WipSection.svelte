<script lang="ts">
  // One section of the WIP panel (Unstaged / Untracked / Staged; split out of
  // WipPanel.svelte): a sticky source-list header with the count and the
  // section's bulk actions, then its rows WINDOWED against the panel's shared
  // `.wp-scroll` scroller. A row is a stage checkbox (checked = staged), a
  // name button (opens the diff) with the status letter, the file name and —
  // in the list view — its dimmed directory, `+/−` line counts, and a
  // discard button revealed on hover / focus / selection. The panel owns
  // every mutation; this component only renders and reports.
  import type { Attachment } from 'svelte/attachments';
  import type { Snippet } from 'svelte';
  import type { FileChange } from '../../../lib/api/types';
  import type { ListWindow } from '../../../lib/listWindow.svelte';
  import { rowMenu } from '../../../lib/rowMenu';
  import { plural } from '../../../lib/plural';
  import Icon from '../../../lib/components/Icon.svelte';
  import Badge from '../../../lib/components/Badge.svelte';
  import { KIND_LETTER, KIND_WORD, type LineCounts, type Row, type TFile, type TFolder, type WipSection } from './wipTree';

  type Origin = { section: WipSection; key: string };
  interface Props {
    section: WipSection;
    title: string;
    /** Files in the section (the header count; folders excluded). */
    count: number;
    rows: Row[];
    lw: ListWindow;
    open: boolean;
    /** Shown when the section has no rows. */
    empty: string;
    /** Folder rows (tree view) vs a flat list with dimmed directories. */
    tree: boolean;
    selected: (path: string) => boolean;
    partial: Set<string>;
    counts: LineCounts | null;
    collapsed: (node: TFolder) => boolean;
    ontoggle: (node: TFolder) => void;
    onstage: (paths: string[], origin?: Origin) => void;
    ondiscard: (paths: string[], label: string, origin?: Origin) => void;
    onselect: (path: string) => void;
    onfilemenu: (e: MouseEvent, c: FileChange) => void;
    onfoldermenu: (e: MouseEvent, node: TFolder) => void;
    /** Paths the header's bulk actions apply to. */
    all: string[];
    diffId: string;
    /** Extra content under the rows (the untracked-cap note). */
    footer?: Snippet;
  }
  let {
    section, title, count, rows, lw, open = $bindable(), empty, tree, selected, partial, counts,
    collapsed, ontoggle, onstage, ondiscard, onselect, onfilemenu, onfoldermenu, all, diffId, footer,
  }: Props = $props();

  const isStaged = $derived(section === 'staged');
  const verb = $derived(isStaged ? 'Unstage' : 'Stage');
  /** The discard wording per section: untracked files are DELETED, the
   *  Unstaged list keeps staged hunks, the Staged list reverts to HEAD. */
  const discardWord = $derived(
    section === 'untracked' ? 'Delete' : section === 'unstaged' ? 'Discard unstaged changes to' : 'Discard changes to',
  );
  const bulkDiscard = $derived(
    section === 'untracked'
      ? 'Delete all untracked files…'
      : section === 'unstaged'
        ? 'Discard all unstaged changes…'
        : 'Discard all staged changes…',
  );
  const bulkLabel = $derived(
    section === 'untracked' ? 'all untracked files' : section === 'unstaged' ? 'all unstaged files' : 'all staged files',
  );

  /** The right-click menu without a `oncontextmenu` attribute on a div
   *  (`use:rowMenu` is the row's keyboard and touch way into it). */
  const contextMenu = (handler: (e: MouseEvent) => void): Attachment<HTMLElement> => (el) => {
    el.addEventListener('contextmenu', handler);
    return () => el.removeEventListener('contextmenu', handler);
  };

  const fmt = (n: number): string => n.toLocaleString();
</script>

{#snippet kindBadge(kind: FileChange['kind'])}
  <span class="wp-kind k-{kind}" title={KIND_WORD[kind]} aria-hidden="true">{KIND_LETTER[kind]}</span>
  <span class="sr-only">{KIND_WORD[kind]}</span>
{/snippet}

{#snippet fileRow(file: TFile, depth: number)}
  {@const path = file.change.path}
  {@const key = `f:${path}`}
  {@const sel = selected(path)}
  {@const stat = counts?.get(path)}
  <div use:rowMenu {@attach contextMenu((e) => onfilemenu(e, file.change))}
    class="wp-file"
    class:selected={sel}
    style:--depth={depth}
  >
    <input
      type="checkbox"
      class="wp-check"
      checked={isStaged}
      data-row-key={key}
      onchange={() => onstage([path], { section, key })}
      title={verb}
      aria-label="{verb} {path}"
    />
    <button
      class="wp-name"
      data-wp-nav={key}
      onclick={() => onselect(path)}
      title={file.change.orig_path ? `${file.change.orig_path} → ${path}` : path}
      aria-expanded={sel}
      aria-controls={sel ? diffId : undefined}
    >
      {@render kindBadge(file.change.kind)}
      <span class="wp-fname" dir="ltr">{file.name}</span>
      {#if !tree && file.dir}<span class="wp-dir" dir="ltr">{file.dir}</span>{:else}<span class="wp-fill"></span>{/if}
      {#if partial.has(path)}
        <!-- Same path in both trees: some hunks staged, some not. -->
        <Badge tone="accent" variant="outline" label="Partial" title="Partially staged" testid="wip-partial" />
      {/if}
      {#if stat && (stat.added || stat.deleted)}
        <span class="wp-stat" title="{fmt(stat.added ?? 0)} added, {fmt(stat.deleted ?? 0)} deleted">
          {#if stat.added}<span class="wp-add">+{fmt(stat.added)}</span>{/if}
          {#if stat.deleted}<span class="wp-del">−{fmt(stat.deleted)}</span>{/if}
        </span>
      {:else if stat && stat.added == null}
        <span class="wp-stat wp-bin">binary</span>
      {/if}
    </button>
    <button
      class="wp-discard"
      title="{discardWord} this file…"
      aria-label="{section === 'untracked' ? 'Delete' : 'Discard'} {path}"
      onclick={() => ondiscard([path], path, { section, key })}
    >
      <Icon name="trash" size={12} />
    </button>
  </div>
{/snippet}

{#snippet folderRow(node: TFolder, depth: number)}
  {@const key = `d:${node.path}`}
  {@const shut = collapsed(node)}
  {@const n = plural(node.files.length, 'file')}
  <!-- The checkbox stages/unstages EVERY file under the folder (recursively —
       node.files carries the whole subtree) in one call; the name only folds,
       a name click must never mutate the index. -->
  <div use:rowMenu {@attach contextMenu((e) => onfoldermenu(e, node))} class="wp-folder" style:--depth={depth}>
    <input
      type="checkbox"
      class="wp-check"
      checked={isStaged}
      data-row-key={key}
      onchange={() => onstage(node.files.map((f) => f.path), { section, key })}
      title="{verb} {node.path}/ ({n})"
      aria-label="{verb} folder {node.path}"
    />
    <button class="wp-fold-name" data-wp-nav={key} onclick={() => ontoggle(node)} title="{node.path}/ ({n})" aria-expanded={!shut}>
      <Icon name={shut ? 'chevronRight' : 'chevronDown'} size={12} />
      <Icon name="folder" size={12} />
      <span class="wp-fold-label" dir="ltr">{node.name}</span>
      <span class="wp-fold-count">{fmt(node.files.length)}</span>
    </button>
    <button
      class="wp-discard"
      title="{discardWord} every file under {node.path}/…"
      aria-label="{section === 'untracked' ? 'Delete' : 'Discard'} folder {node.path}"
      onclick={() => ondiscard(node.files.map((f) => f.path), `${node.path}/ (${n})`, { section, key })}
    >
      <Icon name="trash" size={12} />
    </button>
  </div>
{/snippet}

<div class="wp-section" role="group" data-wp-section={section} aria-label={title}>
  <div class="wp-sec-head">
    <button type="button" class="wp-sec-toggle" data-wp-head={section} onclick={() => (open = !open)} aria-expanded={open}>
      <Icon name={open ? 'chevronDown' : 'chevronRight'} size={12} />
      <span class="wp-sec-title">{title}</span>
      <span class="wp-sec-count">{fmt(count)}</span>
    </button>
    {#if all.length > 0}
      <button
        type="button"
        class="icon-btn wp-sec-btn"
        title="{verb} {bulkLabel}"
        aria-label="{verb} {bulkLabel}"
        onclick={() => onstage(all)}
      >
        <Icon name={isStaged ? 'minus' : 'plus'} size={14} />
      </button>
      <button
        type="button"
        class="icon-btn wp-sec-btn wp-sec-danger"
        title={bulkDiscard}
        aria-label={bulkDiscard}
        onclick={() => ondiscard(all, bulkLabel)}
      >
        <Icon name="trash" size={14} />
      </button>
    {/if}
  </div>
  {#if open}
    {@const win = lw.range(rows.length)}
    {#if rows.length === 0}
      <div class="wp-empty">{empty}</div>
    {/if}
    <div class="wp-rows" data-section={section} {@attach lw.attach}>
      {#if win.top > 0}<div style="height:{win.top}px" aria-hidden="true"></div>{/if}
      {#each rows.slice(win.start, win.end) as r (r.key)}
        {#if r.node.type === 'folder'}
          {@render folderRow(r.node, r.depth)}
        {:else}
          {@render fileRow(r.node, r.depth)}
        {/if}
      {/each}
      {#if win.bottom > 0}<div style="height:{win.bottom}px" aria-hidden="true"></div>{/if}
    </div>
    {@render footer?.()}
  {/if}
</div>

<style>
  .wp-section {
    --wp-row-h: 28px;
    padding-block-end: 4px;
  }
  /* Source-list header: sticks to the top of the shared scroller while its
     own rows scroll beneath it, so the section you are in is always named. */
  .wp-sec-head {
    position: sticky;
    top: 0;
    z-index: var(--z-sticky);
    display: flex;
    align-items: center;
    gap: 2px;
    min-height: 30px;
    padding-inline: 6px 4px;
    background: var(--surface);
    border-block-end: 1px solid var(--separator);
  }
  .wp-sec-toggle {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    height: 26px;
    padding-inline: 4px;
    border: none;
    border-radius: var(--radius-s);
    background: none;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-s);
    font-weight: 600;
    cursor: pointer;
    text-align: start;
  }
  .wp-sec-toggle:hover {
    color: var(--text);
  }
  .wp-sec-title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .wp-sec-count {
    flex-shrink: 0;
    min-width: 18px;
    padding: 0 6px;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text-dim);
    font-size: var(--fs-xs);
    font-weight: 500;
    line-height: 16px;
    text-align: center;
    font-variant-numeric: tabular-nums;
  }
  .wp-sec-btn {
    flex-shrink: 0;
    color: var(--text-dim);
  }
  .wp-sec-danger:hover {
    color: var(--danger);
  }
  .wp-empty {
    padding: 8px 14px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .wp-rows {
    padding-inline: 4px;
    padding-block-start: 2px;
  }

  /* ── Rows (uniform height: the window measures one row's pitch) ───────── */
  .wp-file,
  .wp-folder {
    display: flex;
    align-items: center;
    gap: 6px;
    height: var(--wp-row-h);
    padding-inline: calc(8px + var(--depth, 0) * 14px) 4px;
    border-radius: var(--radius-s);
  }
  .wp-file:hover,
  .wp-folder:hover {
    background: var(--hover);
  }
  .wp-file.selected {
    background: var(--accent-soft);
  }
  .wp-check {
    flex-shrink: 0;
    margin: 0;
  }
  .wp-name,
  .wp-fold-name {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 1;
    min-width: 0;
    height: 100%;
    padding: 0;
    border: none;
    background: transparent;
    color: var(--text);
    font: inherit;
    cursor: pointer;
    text-align: start;
  }
  .wp-name:focus-visible,
  .wp-fold-name:focus-visible {
    outline: 2px solid var(--accent-line-strong);
    outline-offset: -2px;
    border-radius: var(--radius-s);
  }
  /* Filename first and never squeezed out by its directory: the directory
     takes what is left and ellipsizes first. */
  .wp-fname {
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-m);
    unicode-bidi: isolate;
  }
  .wp-dir {
    flex: 1 1 0;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-s);
    color: var(--text-dim);
    unicode-bidi: isolate;
  }
  .wp-fill {
    flex: 1 1 0;
  }
  .wp-stat {
    flex-shrink: 0;
    display: inline-flex;
    gap: 4px;
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
  }
  .wp-add {
    color: var(--success);
  }
  .wp-del {
    color: var(--danger);
  }
  .wp-bin {
    color: var(--text-dim);
    font-family: inherit;
  }

  /* Status letter: a soft-tinted square, tone by kind (meaning, not decoration;
     the word is in the tooltip and read out by the sr-only label). */
  .wp-kind {
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
  .k-added,
  .k-untracked {
    background: var(--success-soft);
    color: var(--success);
  }
  .k-deleted,
  .k-conflicted {
    background: var(--danger-soft);
    color: var(--danger);
  }
  .k-renamed {
    background: var(--info-soft);
    color: var(--info);
  }

  .wp-discard {
    flex-shrink: 0;
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    padding: 0;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    opacity: 0;
    transition: opacity var(--dur-fast) ease-out;
  }
  .wp-file:hover .wp-discard,
  .wp-file:focus-within .wp-discard,
  .wp-file.selected .wp-discard,
  .wp-folder:hover .wp-discard,
  .wp-folder:focus-within .wp-discard {
    opacity: 1;
  }
  .wp-discard:hover {
    color: var(--danger);
    background: var(--danger-soft);
  }
  /* No hover on touch: the action can't stay hidden behind it. */
  @media (hover: none) {
    .wp-discard {
      opacity: 1;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .wp-discard {
      transition: none;
    }
  }

  .wp-fold-label {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-m);
    font-weight: 500;
  }
  .wp-fold-name :global(svg) {
    flex-shrink: 0;
    color: var(--text-dim);
  }
  .wp-fold-count {
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

  /* ≤1024px (the graph stacks as an accordion): touch-height rows. */
  @media (max-width: 1024px) {
    .wp-section {
      --wp-row-h: 36px;
    }
    .wp-check {
      width: 17px;
      height: 17px;
    }
    .wp-discard {
      opacity: 1;
      width: 32px;
      height: 32px;
    }
  }
  /* A narrow pane drops the directory (the tooltip keeps the full path) and
     the line counts before the filename ever ellipsizes. */
  @container wip (max-width: 340px) {
    .wp-dir,
    .wp-stat {
      display: none;
    }
  }
</style>
