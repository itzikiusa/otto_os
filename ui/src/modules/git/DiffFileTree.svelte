<script lang="ts">
  // The changed-files tree of a PR — one component, two hosts: the diff's own
  // navigator rail (DiffNav) and the PR workspace's Files panel under the PR
  // list (PrFilesPanel). Directories are collapsible (single-child chains
  // compressed, see buildNavTree) and show how much of them is viewed; a file
  // row is the Viewed checkbox beside a real button that jumps to the file.
  // Rows are windowed (ListWindow, against the nearest scrolling ancestor), so
  // a 10k-file PR mounts only what is on screen, and ⌘F still sees every row.
  import type { FileDiff } from '../../lib/api/types';
  import Icon from '../../lib/components/Icon.svelte';
  import { plural } from '../../lib/plural';
  import { ListWindow } from '../../lib/listWindow.svelte';
  import { fileStat } from './diff-model';
  import { fileStatus, type NavDir } from './diff-pr';

  interface Props {
    tree: NavDir;
    /** Files to show (null = all). Dirs with nothing shown beneath are hidden. */
    matchSet: ReadonlySet<string> | null;
    viewed: ReadonlySet<string>;
    commentCounts: ReadonlyMap<string, number>;
    /** The file the reader is on: highlighted, and kept in view while the
     *  pointer is not over the tree. */
    current?: string | null;
    /** Status glyph (added / modified / …) before the name. */
    showStatus?: boolean;
    onjump: (path: string) => void;
    ontoggleviewed: (path: string) => void;
  }
  let { tree, matchSet, viewed, commentCounts, current = null, showStatus = false, onjump, ontoggleviewed }: Props = $props();

  let collapsed: Record<string, boolean> = $state({});
  function toggleDir(path: string): void {
    collapsed = { ...collapsed, [path]: !collapsed[path] };
  }

  /** Per dir: files shown beneath it and how many of those are viewed. */
  const dirStats = $derived.by(() => {
    const m = matchSet;
    const out = new Map<string, { total: number; viewed: number }>();
    const walk = (d: NavDir): { total: number; viewed: number } => {
      const s = { total: 0, viewed: 0 };
      for (const f of d.files) {
        if (m !== null && !m.has(f.path)) continue;
        s.total++;
        if (viewed.has(f.path)) s.viewed++;
      }
      for (const sub of d.dirs) {
        const c = walk(sub);
        s.total += c.total;
        s.viewed += c.viewed;
      }
      out.set(d.path, s);
      return s;
    };
    walk(tree);
    return out;
  });

  // The tree as the flat list of VISIBLE rows (folded dirs and filtered-out
  // files skipped). Dir and file rows share one fixed pitch (CSS) so the
  // window math is exact.
  type Row =
    | { kind: 'dir'; d: NavDir; depth: number; key: string }
    | { kind: 'file'; f: FileDiff; depth: number; key: string };
  const rows = $derived.by(() => {
    const out: Row[] = [];
    const m = matchSet;
    const stats = dirStats;
    const shut = collapsed;
    const walk = (d: NavDir, depth: number): void => {
      for (const sub of d.dirs) {
        if (m !== null && !stats.get(sub.path)?.total) continue;
        out.push({ kind: 'dir', d: sub, depth, key: `d:${sub.path}` });
        if (!shut[sub.path]) walk(sub, depth + 1);
      }
      for (const f of d.files) {
        if (m !== null && !m.has(f.path)) continue;
        out.push({ kind: 'file', f, depth, key: `f:${f.path}` });
      }
    };
    walk(tree, 0);
    return out;
  });
  const win = new ListWindow('.nav-file, .nav-dir-row', { min: 200, overscan: 15 });
  const range = $derived(win.range(rows.length));
  $effect(() => win.findRows(() => null, () => rows, (r) => (r.kind === 'dir' ? r.d.label : (r.f.path.split('/').pop() ?? ''))));

  // Follow the reader — never while the pointer is over the tree (rows would
  // move under the cursor).
  let hovering = false;
  function trackHover(el: HTMLElement): () => void {
    const on = (): void => { hovering = true; };
    const off = (): void => { hovering = false; };
    el.addEventListener('pointerenter', on);
    el.addEventListener('pointerleave', off);
    return () => {
      el.removeEventListener('pointerenter', on);
      el.removeEventListener('pointerleave', off);
    };
  }
  $effect(() => {
    const cur = current;
    if (!cur || hovering) return;
    const i = rows.findIndex((r) => r.kind === 'file' && r.f.path === cur);
    if (i >= 0) win.reveal(i);
  });

  function baseName(path: string): string {
    return path.split('/').pop() ?? path;
  }
</script>

{#snippet fileRow(file: FileDiff, depth: number)}
  {@const stats = fileStat(file)}
  {@const cCount = commentCounts.get(file.path) ?? 0}
  {@const isViewed = viewed.has(file.path)}
  {@const st = showStatus ? fileStatus(file) : null}
  <!-- A plain box holding TWO controls side by side — the "viewed" checkbox and
       a real button that jumps to the file — so nothing interactive nests
       inside a role=button. -->
  <div
    class="nav-file"
    class:nav-file-viewed={isViewed}
    class:nav-file-current={current === file.path}
    style="padding-inline-start: {8 + depth * 12}px"
    title={file.path}
  >
    <span class="nav-viewed-cb">
      <input
        type="checkbox"
        checked={isViewed}
        title={isViewed ? 'Viewed — untick to reopen' : 'Mark as viewed'}
        onchange={() => ontoggleviewed(file.path)}
        aria-label="Mark {file.path} as viewed"
      />
    </span>
    <button class="nav-file-open" onclick={() => onjump(file.path)} aria-label="Go to {file.path}" aria-current={current === file.path ? 'true' : undefined}>
      {#if st}<span class="nav-st st-{st.kind}" title={st.label} aria-hidden="true">{st.glyph}</span>{/if}
      <span class="nav-file-path"><span class="nav-base">{baseName(file.path)}</span></span>
      {#if cCount > 0}
        <span class="nav-comment-badge" title={plural(cCount, 'comment')}><Icon name="comment" size={12} />{cCount}</span>
      {/if}
      <span class="nav-file-stats">
        <span class="add">+{stats.add}</span>
        <span class="del">−{stats.del}</span>
      </span>
    </button>
  </div>
{/snippet}

{#snippet dirRow(sub: NavDir, depth: number)}
  {@const s = dirStats.get(sub.path)}
  <button
    class="nav-dir-row"
    class:nav-dir-done={!!s && s.total > 0 && s.viewed === s.total}
    style="padding-inline-start: {8 + depth * 12}px"
    aria-expanded={!collapsed[sub.path]}
    onclick={() => toggleDir(sub.path)}
    title="{sub.path}{s ? ` · ${s.viewed}/${s.total} viewed` : ''}"
  >
    <span class="nav-dir-chevron">
      <Icon name={collapsed[sub.path] ? 'chevronRight' : 'chevronDown'} size={12} />
    </span>
    <Icon name="folder" size={12} />
    <span class="nav-dir-label">{sub.label}</span>
    {#if s && s.viewed > 0}
      <span class="nav-dir-count">{#if s.viewed === s.total}<Icon name="check" size={12} />{:else}{s.viewed}/{s.total}{/if}</span>
    {/if}
  </button>
{/snippet}

<div class="nav-rows" {@attach win.attach} {@attach trackHover}>
  {#if range.top > 0}<div style="height:{range.top}px" aria-hidden="true"></div>{/if}
  {#each rows.slice(range.start, range.end) as r (r.key)}
    {#if r.kind === 'dir'}
      {@render dirRow(r.d, r.depth)}
    {:else}
      {@render fileRow(r.f, r.depth)}
    {/if}
  {/each}
  {#if range.bottom > 0}<div style="height:{range.bottom}px" aria-hidden="true"></div>{/if}
</div>

<style>
  /* One fixed pitch for dir and file rows: the tree is windowed (ListWindow
     measures it from a rendered row), so every row must be the same height. */
  .nav-dir-row,
  .nav-file {
    height: 26px;
    box-sizing: border-box;
    overflow: hidden;
  }
  .nav-dir-row {
    display: flex;
    align-items: center;
    gap: 4px;
    padding-block: 2px;
    padding-inline-end: 8px;
    inline-size: 100%;
    border: 0;
    background: transparent;
    font: inherit;
    font-size: var(--fs-xs);
    text-align: start;
    cursor: pointer;
    color: var(--text-dim);
    min-width: 0;
    white-space: nowrap;
  }
  .nav-dir-row:hover {
    background: var(--hover);
    color: var(--text);
  }
  .nav-dir-chevron {
    display: flex;
    align-items: center;
    flex-shrink: 0;
  }
  .nav-dir-label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    font-weight: 600;
  }
  .nav-dir-count {
    flex: none;
    display: inline-flex;
    align-items: center;
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
    font-weight: 400;
  }
  .nav-dir-done .nav-dir-count { color: var(--success); }
  .nav-file {
    display: flex;
    align-items: center;
    gap: 4px;
    padding-block: 4px;
    padding-inline-end: 8px;
    font-size: var(--fs-xs);
    color: var(--text);
    border-inline-start: 2px solid transparent;
    transition: background var(--dur-fast);
    min-width: 0;
  }
  .nav-file:hover {
    background: var(--hover);
  }
  .nav-file.nav-file-viewed {
    opacity: 0.45;
  }
  .nav-file.nav-file-current {
    opacity: 1;
    background: var(--accent-soft);
    border-inline-start-color: var(--accent-solid);
  }
  /* The file-jump button fills the row beside the checkbox. */
  .nav-file-open {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 4px;
    block-size: 100%;
    padding: 0;
    border: 0;
    background: transparent;
    color: inherit;
    font: inherit;
    font-size: var(--fs-xs);
    text-align: start;
    cursor: pointer;
  }
  .nav-viewed-cb {
    flex-shrink: 0;
    display: flex;
    align-items: center;
  }
  .nav-viewed-cb input[type='checkbox'] {
    width: 12px;
    height: 12px;
    cursor: pointer;
    accent-color: var(--accent);
  }
  .nav-st { flex: none; width: 10px; text-align: center; font-family: var(--font-mono); color: var(--text-dim); }
  .nav-st.st-added { color: var(--success); }
  .nav-st.st-deleted { color: var(--danger); }
  .nav-st.st-renamed { color: var(--info); }
  .nav-file-path {
    flex: 1;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    min-width: 0;
    line-height: 1.3;
  }
  .nav-base {
    font-weight: 600;
    font-size: var(--fs-xs);
  }
  .nav-file-stats {
    display: flex;
    gap: 4px;
    flex-shrink: 0;
    font-size: var(--fs-xs);
  }
  .nav-comment-badge {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    gap: 2px;
    font-size: var(--fs-xs);
    color: var(--accent-text);
    white-space: nowrap;
  }
  .add {
    color: var(--success);
    font-weight: 600;
    font-size: var(--fs-xs);
  }
  .del {
    color: var(--danger);
    font-weight: 600;
    font-size: var(--fs-xs);
  }

  @media (max-width: 1024px) {
    .nav-file { font-size: var(--fs-m); padding-block: 8px; height: 38px; }
    .nav-base { font-size: var(--fs-m); }
    .nav-dir-row { font-size: var(--fs-m); padding-block: 6px; height: 38px; }
    .nav-file-stats { font-size: var(--fs-s); }
    .add, .del { font-size: var(--fs-m); }
    /* "Mark as viewed" checkbox: 12px is below the touch minimum. */
    .nav-viewed-cb input[type='checkbox'] { width: 18px; height: 18px; }
  }
</style>
