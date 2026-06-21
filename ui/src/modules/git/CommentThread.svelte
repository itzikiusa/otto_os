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

<div class="cmt" style="margin-left: {Math.min(depth, 4) * 18}px">
  <div class="cmt-head">
    <span class="cmt-avatar">{comment.author.slice(0, 1).toUpperCase()}</span>
    <span class="cmt-author">{comment.author}</span>
    {#if comment.path && comment.line !== null && depth === 0}
      <span class="chip mono">{comment.path}:{comment.line}</span>
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
    margin: 4px 0 4px 25px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-width: 480px;
  }
</style>
