<script lang="ts">
  // One PR comment + its replies (recursive). Optional reply affordance.
  import CommentThread from './CommentThread.svelte';
  import type { PrComment } from '../../lib/api/types';
  import { renderMarkdown } from '../../lib/md';

  interface Props {
    comment: PrComment;
    onreply?: (parentId: string, body: string) => Promise<void>;
    depth?: number;
  }
  let { comment, onreply, depth = 0 }: Props = $props();

  let replying = $state(false);
  let replyText = $state('');
  let busy = $state(false);

  // On a phone the desktop 18px-per-level indent quickly eats the narrow width
  // for deep reply chains; halve it under ≤1024 so nested threads stay readable
  // without pushing content off-screen. Tracked via matchMedia (same convention
  // as the other git views) so desktop nesting is unchanged.
  let isMobile = $state(false);
  $effect(() => {
    const mq = window.matchMedia('(max-width: 1024px)');
    const sync = () => (isMobile = mq.matches);
    sync();
    mq.addEventListener('change', sync);
    return () => mq.removeEventListener('change', sync);
  });
  const indent = $derived(Math.min(depth, 4) * (isMobile ? 9 : 18));

  function fmtDate(iso: string): string {
    const d = new Date(iso);
    return d.toLocaleDateString([], { month: 'short', day: 'numeric' }) +
      ' ' +
      d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  }

  async function submitReply(): Promise<void> {
    if (!onreply || replyText.trim() === '') return;
    busy = true;
    try {
      await onreply(comment.id, replyText.trim());
      replying = false;
      replyText = '';
    } finally {
      busy = false;
    }
  }
</script>

<div class="cmt" style="margin-inline-start: {indent}px">
  <div class="cmt-head">
    <span class="cmt-avatar">{comment.author.slice(0, 1).toUpperCase()}</span>
    <span class="cmt-author">{comment.author}</span>
    {#if comment.path && comment.line !== null && depth === 0}
      <span class="chip mono cmt-loc">{comment.path}:{comment.line}</span>
    {/if}
    <span class="cmt-date dim">{fmtDate(comment.created_at)}</span>
  </div>
  <div class="cmt-body md-body">
    <!-- renderMarkdown escapes HTML before transforming -->
    {@html renderMarkdown(comment.body)}
  </div>
  {#if onreply}
    <div class="cmt-actions">
      {#if replying}
        <textarea class="input" rows="2" bind:value={replyText} placeholder="Reply…"></textarea>
        <div class="row" style="justify-content: flex-end">
          <button class="btn small" onclick={() => (replying = false)}>Cancel</button>
          <button class="btn small primary" disabled={busy || replyText.trim() === ''} onclick={submitReply}>
            {busy ? 'Posting…' : 'Reply'}
          </button>
        </div>
      {:else}
        <button class="btn small ghost" onclick={() => (replying = true)}>Reply</button>
      {/if}
    </div>
  {/if}
  {#each comment.replies as r (r.id)}
    <CommentThread comment={r} {onreply} depth={depth + 1} />
  {/each}
</div>

<style>
  .cmt {
    padding: 8px 0 2px;
  }
  .cmt-head {
    display: flex;
    align-items: center;
    gap: 7px;
    margin-bottom: 3px;
    flex-wrap: wrap;
  }
  .cmt-avatar {
    width: 18px;
    height: 18px;
    border-radius: 50%;
    background: color-mix(in srgb, var(--accent) 25%, transparent);
    color: var(--accent);
    font-size: 9.5px;
    font-weight: 700;
    display: grid;
    place-items: center;
  }
  .cmt-author {
    font-size: 12px;
    font-weight: 600;
  }
  .cmt-date {
    font-size: 10.5px;
  }
  .cmt-body {
    font-size: 12.5px;
    margin-inline-start: 25px;
  }
  .cmt-actions {
    margin: 4px 0;
    margin-inline-start: 25px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-width: 480px;
  }

  /* ── Mobile + tablet (≤1024px) ──────────────────────────────────────────────
     The location chip can hold a long "path:line"; let it shrink + ellipsis so
     it never forces the head row (or the page) wider than the viewport. The
     reply box + buttons go full-width and grow their touch height on a phone. */
  @media (max-width: 1024px) {
    .cmt-loc {
      display: inline-block;
      min-width: 0;
      max-width: 100%;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
      box-sizing: border-box;
    }
    .cmt-actions { max-width: 100%; }
    /* Markdown body can hold long unbroken tokens (URLs, paths) or wide images;
       break them and cap image width so a comment never pushes the page wider
       than the viewport. (Code blocks already side-scroll in their own box via
       the global .md-body pre rule.) */
    .cmt-body {
      overflow-wrap: anywhere;
      word-break: break-word;
    }
    .cmt-body :global(img) {
      max-width: 100%;
      height: auto;
    }
  }
  @media (max-width: 640px) {
    .cmt-body { margin-inline-start: 0; }
    .cmt-actions { margin-inline-start: 0; }
    .cmt-actions .btn { min-height: 38px; }
  }
</style>
