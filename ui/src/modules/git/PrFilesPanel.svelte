<script lang="ts">
  // The PR workspace's quick file access, under the PR list: every file of the
  // PR in front with its Viewed tick, +/− and comment count. A click jumps to
  // the file in the Code tab (bringing that tab to the front); the tick is the
  // same Viewed mark as the diff's (shared through prFilesBridge, persisted
  // per file content). Made for reading a big PR one file at a time: a
  // progress line, "Next unviewed", "Hide viewed", and the row of the file at
  // the top of the diff follows the reader. The tree is DiffFileTree, the same
  // one as the diff's own navigator rail.
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { plural } from '../../lib/plural';
  import DiffFileTree from './DiffFileTree.svelte';
  import { buildNavTree, nextUnviewed as nextUnviewedOf, type NavDir } from './diff-pr';
  import { prFilesBridge, prFilesKey } from './prFilesBridge.svelte';

  interface Props {
    repoId: string;
    number: number;
    collapsed: boolean;
    ontogglecollapsed: () => void;
  }
  let { repoId, number, collapsed, ontogglecollapsed }: Props = $props();

  const key = $derived(prFilesKey(repoId, number));
  const feed = $derived(prFilesBridge.feed?.key === key ? prFilesBridge.feed : null);

  const HIDE_KEY = 'otto_pr_files_hide_viewed';
  let hideViewed = $state((() => { try { return localStorage.getItem(HIDE_KEY) === '1'; } catch { return false; } })());
  function toggleHideViewed(): void {
    hideViewed = !hideViewed;
    try { localStorage.setItem(HIDE_KEY, hideViewed ? '1' : '0'); } catch { /* not persisted */ }
  }
  let filter = $state('');

  const files = $derived(feed?.files ?? []);
  const viewedCount = $derived(feed ? files.reduce((n, f) => n + (feed.viewed.has(f.path) ? 1 : 0), 0) : 0);
  const tree = $derived(buildNavTree(files));
  /** Files left by the filter and "Hide viewed" (null = every file). The
   *  current file stays while it is open, even once ticked. */
  const shown = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    if (!feed || (!q && !hideViewed)) return null;
    return new Set(files.filter((f) =>
      (!hideViewed || !feed.viewed.has(f.path) || f.path === feed.current)
      && (!q || f.path.toLowerCase().includes(q))).map((f) => f.path));
  });

  function open(path: string): void {
    void prFilesBridge.open(key, path);
  }
  /** The files in tree order (what "next" means to someone reading the tree). */
  const order = $derived.by(() => {
    const out: string[] = [];
    const walk = (d: NavDir): void => {
      for (const sub of d.dirs) walk(sub);
      for (const f of d.files) out.push(f.path);
    };
    walk(tree);
    return out;
  });
  /** The first unviewed file after the current one, in tree order (wrapping). */
  const nextUnviewed = $derived(feed ? nextUnviewedOf(order, feed.viewed, feed.current) : null);
  /** Tick the current file viewed and move on — the one-file-at-a-time loop. */
  function viewedAndNext(): void {
    if (!feed) return;
    const cur = feed.current;
    if (cur && !feed.viewed.has(cur)) feed.toggleViewed(cur);
    if (nextUnviewed && nextUnviewed !== cur) open(nextUnviewed);
  }

</script>

<section class="pfp" class:collapsed aria-label="Files in pull request #{number}">
  <div class="pfp-head">
    <button class="pfp-title" aria-expanded={!collapsed} title={collapsed ? 'Show the files' : 'Hide the files'} onclick={ontogglecollapsed}>
      <Icon name={collapsed ? 'chevronRight' : 'chevronDown'} size={12} />
      <span>Files</span>
    </button>
    {#if feed}
      <span class="pfp-count" title="{viewedCount} of {plural(files.length, 'file')} viewed">{viewedCount}/{files.length} viewed</span>
      <span class="pfp-progress" aria-hidden="true"><span style="inline-size:{files.length ? (viewedCount / files.length) * 100 : 0}%"></span></span>
      <span class="grow"></span>
      <button class="pfp-icon" aria-label="Mark viewed and go to the next unviewed file" title="Mark viewed · next unviewed file"
        disabled={!nextUnviewed} onclick={viewedAndNext}>
        <Icon name="check" size={13} />
      </button>
      <button class="pfp-icon" aria-label="Next unviewed file" title="Next unviewed file" disabled={!nextUnviewed}
        onclick={() => nextUnviewed && open(nextUnviewed)}>
        <Icon name="arrowDown" size={13} />
      </button>
      <button class="pfp-icon" aria-pressed={hideViewed} aria-label={hideViewed ? 'Show viewed files' : 'Hide viewed files'}
        title={hideViewed ? 'Show viewed files' : 'Hide viewed files'} onclick={toggleHideViewed}>
        <Icon name={hideViewed ? 'eyeOff' : 'eye'} size={13} />
      </button>
    {/if}
  </div>

  {#if !collapsed}
    {#if !feed}
      <LoadState what="the files" loading />
    {:else}
      {#if files.length > 8}
        <div class="pfp-filter">
          <input class="input pfp-filter-input" type="search" dir="ltr" placeholder="Filter files" aria-label="Filter files" bind:value={filter} />
        </div>
      {/if}
      <div class="pfp-list">
        {#if shown?.size === 0}
          <p class="pfp-note">{filter ? 'No file matches.' : 'Every file is viewed.'}</p>
        {:else}
          <DiffFileTree {tree} matchSet={shown} viewed={feed.viewed} commentCounts={feed.commentCounts} current={feed.current}
            showStatus onjump={open} ontoggleviewed={(p) => feed.toggleViewed(p)} />
        {/if}
      </div>
    {/if}
  {/if}
</section>

<style>
  .pfp {
    display: flex;
    flex-direction: column;
    min-height: 0;
    height: 100%;
    border-block-start: 1px solid var(--separator);
  }
  .pfp.collapsed { height: auto; }
  .pfp-head {
    flex: none;
    display: flex;
    align-items: center;
    gap: 6px;
    padding-block: 6px;
    padding-inline: 6px 8px;
    min-width: 0;
  }
  .pfp-title {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    background: none;
    border: none;
    color: var(--text);
    font-size: var(--fs-s);
    font-weight: 600;
    padding: 2px 4px;
    border-radius: var(--radius-s);
    cursor: pointer;
  }
  .pfp-title:hover { background: var(--hover); }
  .pfp-count {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .pfp-progress {
    flex: 0 1 60px;
    min-width: 24px;
    height: 4px;
    border-radius: 2px;
    background: var(--surface-3);
    overflow: hidden;
  }
  .pfp-progress > span { display: block; height: 100%; background: var(--success); }
  .grow { flex: 1; }
  .pfp-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex: none;
    width: 24px;
    height: 24px;
    background: none;
    border: none;
    color: var(--text-dim);
    border-radius: var(--radius-s);
    cursor: pointer;
  }
  .pfp-icon:hover:not(:disabled) { background: var(--hover); color: var(--text); }
  .pfp-icon:disabled { opacity: var(--disabled-opacity); cursor: default; }
  .pfp-icon[aria-pressed='true'] { color: var(--accent-text); }
  .pfp-filter { flex: none; padding: 0 8px 6px; }
  .pfp-filter-input { width: 100%; height: 26px; font-size: var(--fs-s); }
  .pfp-list { flex: 1; min-height: 0; overflow-y: auto; padding-block-end: 6px; }
  .pfp-note { margin: 0; padding: 8px 12px; font-size: var(--fs-s); color: var(--text-dim); }
  @media (max-width: 1024px) {
    .pfp-icon { width: 32px; height: 32px; }
  }
</style>
