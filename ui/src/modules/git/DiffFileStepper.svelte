<script lang="ts">
  // The Code tab's file stepper on phones and tablets (≤1024 px), a row of
  // its own inside the pinned diff toolbar: ‹ previous file, "12 / 54" (opens
  // the file picker), › next file, Viewed for the file being read, and Next
  // unviewed. There the desktop Files panel is hidden and the tree rail sits
  // above the diff, so without this a 50-file PR could only be scrolled.
  // Everything comes from the PR files feed the viewer publishes
  // (prFilesBridge): the same files, Viewed marks and jump as the Files panel.
  import Icon from '../../lib/components/Icon.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import DiffFileTree from './DiffFileTree.svelte';
  import { buildNavTree, nextUnviewed } from './diff-pr';
  import { prFilesBridge } from './prFilesBridge.svelte';

  let { feedKey }: { feedKey: string } = $props();

  const feed = $derived(prFilesBridge.feed?.key === feedKey ? prFilesBridge.feed : null);
  const order = $derived((feed?.files ?? []).map((f) => f.path));
  /** Index of the file being read (-1 = above the first file / unknown). */
  const at = $derived(feed?.current ? order.indexOf(feed.current) : -1);
  const cur = $derived(at >= 0 ? order[at] : null);
  const isViewed = $derived(!!cur && !!feed?.viewed.has(cur));
  const unviewed = $derived(feed ? nextUnviewed(order, feed.viewed, feed.current) : null);

  function go(i: number): void {
    const p = order[i];
    if (feed && p !== undefined) void feed.jump(p);
  }

  // ── The picker: the PR's file tree in a sheet ────────────────────────────
  let picking = $state(false);
  let filter = $state('');
  const tree = $derived(picking ? buildNavTree(feed?.files ?? []) : null);
  const shown = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    return q ? new Set(order.filter((p) => p.toLowerCase().includes(q))) : null;
  });
  function pick(path: string): void {
    picking = false;
    filter = '';
    // After the sheet is gone, so the jump measures the diff, not the sheet.
    requestAnimationFrame(() => void feed?.jump(path));
  }
</script>

{#if feed && order.length > 0}
  <div class="dfs" role="group" aria-label="Files in this pull request">
    <button class="dfs-btn" disabled={at <= 0} aria-label="Previous file" title="Previous file" onclick={() => go(at - 1)}>
      <Icon name="chevronLeft" size={16} />
    </button>
    <button class="dfs-count" aria-haspopup="dialog" aria-label="File {at + 1 || '–'} of {order.length} — pick a file" title="Pick a file" onclick={() => (picking = true)}>
      <span class="dfs-num">{at >= 0 ? at + 1 : '–'} / {order.length}</span>
      {#if cur}<span class="dfs-name" dir="ltr">{cur.split('/').pop()}</span>{/if}
    </button>
    <button class="dfs-btn" disabled={at >= order.length - 1} aria-label="Next file" title="Next file" onclick={() => go(at + 1)}>
      <Icon name="chevronRight" size={16} />
    </button>
    <button class="dfs-btn dfs-viewed" disabled={!cur} aria-pressed={isViewed}
      aria-label={isViewed ? 'Viewed — untick to reopen' : 'Mark this file viewed'}
      title={isViewed ? 'Viewed — untick to reopen' : 'Mark this file viewed'}
      onclick={() => cur && feed.toggleViewed(cur)}>
      <Icon name="check" size={16} /><span>Viewed</span>
    </button>
    <button class="dfs-btn" disabled={!unviewed || unviewed === cur} aria-label="Next unviewed file" title="Next unviewed file"
      onclick={() => unviewed && void feed.jump(unviewed)}>
      <Icon name="arrowDown" size={16} />
    </button>
  </div>
{/if}

{#if picking && feed && tree}
  <Modal title="Files" height={640} onclose={() => { picking = false; filter = ''; }}>
    <input class="input dfs-filter" type="search" dir="ltr" placeholder="Filter files" aria-label="Filter files" bind:value={filter} />
    {#if shown?.size === 0}
      <p class="dfs-note">No file matches.</p>
    {:else}
      <DiffFileTree {tree} matchSet={shown} viewed={feed.viewed} commentCounts={feed.commentCounts} current={feed.current}
        showStatus onjump={pick} ontoggleviewed={(p) => feed.toggleViewed(p)} />
    {/if}
  </Modal>
{/if}

<style>
  /* Its own row of the (wrapping, pinned) toolbar. */
  .dfs {
    flex: 1 0 100%;
    display: flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
  }
  .dfs-btn,
  .dfs-count {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 4px;
    min-width: 44px;
    min-height: 44px;
    padding-inline: 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    color: var(--text);
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .dfs-btn:disabled {
    opacity: var(--disabled-opacity);
    cursor: default;
  }
  .dfs-btn:focus-visible,
  .dfs-count:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 1px;
  }
  .dfs-count {
    flex: 1 1 auto;
    min-width: 0;
    justify-content: flex-start;
  }
  .dfs-num {
    flex: none;
    font-variant-numeric: tabular-nums;
    font-weight: 600;
  }
  .dfs-name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-dim);
    font-family: var(--font-mono);
  }
  .dfs-viewed[aria-pressed='true'] {
    background: var(--success-soft);
    border-color: var(--success);
    color: var(--success);
  }
  .dfs-filter {
    width: 100%;
    height: 36px;
    margin-block-end: 8px;
    font-size: var(--fs-m);
  }
  /* Landscape phone: inline in the toolbar's single row (DiffPrToolbar). */
  @media (max-width: 1024px) and (max-height: 500px) {
    .dfs {
      flex: 0 0 auto;
    }
    .dfs-count {
      flex: none;
    }
    .dfs-name,
    .dfs-viewed span {
      display: none;
    }
  }
  .dfs-note {
    margin: 0;
    padding: 8px 4px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
</style>
