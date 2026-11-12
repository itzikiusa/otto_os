<script lang="ts">
  import { announceShown } from '../../lib/announce.svelte';
  // One PR comment + its replies (recursive). Optional reply affordance; inside
  // a PR (pr-comment-ctx) also edit / delete your own comment and reactions.
  // A GitHub review summary wears its verdict chip and can only be replied to.
  import CommentThread from './CommentThread.svelte';
  import type { PrComment } from '../../lib/api/types';
  import { guardUnsaved } from '../../lib/leaveGuard';
  import { renderMarkdown } from '../../lib/md';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import Icon from '../../lib/components/Icon.svelte';
  import PrReactions from './PrReactions.svelte';
  import { prCommentActions } from './pr-comment-ctx';
  import { isReviewSummary, replyingTo, reviewChip } from './pr-comments';

  interface Props {
    comment: PrComment;
    onreply?: (parentId: string, body: string) => Promise<void>;
    /** Resolve/reopen the thread on the provider. Only offered on thread heads
     *  that carry a provider thread_id. */
    onresolve?: (threadId: string, resolved: boolean) => Promise<void>;
    depth?: number;
    /** The thread head (set on replies): names who a reply-to-a-reply answers. */
    root?: PrComment;
  }
  let { comment, onreply, onresolve, depth = 0, root }: Props = $props();

  const actions = prCommentActions();
  const summary = $derived(isReviewSummary(comment));
  const chip = $derived(comment.review_state ? reviewChip(comment.review_state) : null);
  const answers = $derived(root ? replyingTo(comment, root) : null);
  const canEdit = $derived(!!actions && comment.can_edit === true && !summary);
  const showReactions = $derived(!!actions && !summary && actions.reactionsSupported());

  // Edit in place: the body swaps for the composer, pre-filled.
  let editing = $state(false);
  let editText = $state('');
  let editBusy = $state(false);
  let editError = $state<string | null>(null);
  $effect(() => guardUnsaved(() => editing && editText !== comment.body, { what: 'comment edit' }));
  function ownMenu(e: MouseEvent): void {
    ctxMenu.show(e, [
      { label: 'Edit', icon: 'edit', action: () => { editText = comment.body; editError = null; editing = true; } },
      { label: 'Delete…', icon: 'trash', danger: true, action: () => void actions?.remove(comment) },
    ]);
  }
  async function saveEdit(): Promise<void> {
    if (!actions || editBusy || editText.trim() === '') return;
    editBusy = true;
    editError = null;
    try {
      await actions.edit(comment, editText.trim());
      editing = false;
    } catch (e) {
      editError = loadErrorText(e);
    } finally {
      editBusy = false;
    }
  }

  let replying = $state(false);
  let replyText = $state('');
  let busy = $state(false);
  let replyError = $state<string | null>(null);
  $effect(() => guardUnsaved(() => replying && replyText.trim() !== '', { what: 'comment reply' }));
  let resolveBusy = $state(false);

  // Thread status lives on the head comment; replies inherit the container.
  const isHead = $derived(depth === 0);
  const canResolve = $derived(isHead && onresolve != null && comment.thread_id != null);

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
    if (busy || !onreply || replyText.trim() === '') return;
    busy = true;
    replyError = null;
    try {
      await onreply(comment.id, replyText.trim());
      replying = false;
      replyText = '';
    } catch (e) {
      replyError = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  async function toggleResolved(): Promise<void> {
    if (!onresolve || comment.thread_id == null) return;
    resolveBusy = true;
    try {
      await onresolve(comment.thread_id, !comment.resolved);
    } finally {
      resolveBusy = false;
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
    {#if isHead && comment.resolved}
      <span class="chip ok cmt-status" title="This thread is resolved on the provider">Resolved</span>
    {/if}
    {#if chip}<span class="cmt-review tone-{chip.tone}" title="GitHub review">{chip.label}</span>{/if}
    <span class="cmt-date dim">{fmtDate(comment.created_at)}</span>
    {#if canEdit && !editing}
      <button class="btn small ghost cmt-more" aria-haspopup="menu" aria-label="Comment actions" title="Edit or delete your comment" onclick={ownMenu}>
        <Icon name="more" size={14} />
      </button>
    {/if}
  </div>
  {#if answers}<div class="cmt-answers dim" dir="auto">↪ replying to {answers}</div>{/if}
  {#if editing}
    <div class="cmt-actions">
      <textarea dir="auto" class="input" rows="3" bind:value={editText} disabled={editBusy} aria-label="Edit comment"></textarea>
      {#if editError}<p class="reply-error" use:announceShown={{ text: `Couldn’t save the comment. ${editError}`, assertive: true }}>Couldn’t save the comment. {editError}</p>{/if}
      <div class="row" style="justify-content: flex-end">
        <button class="btn small" disabled={editBusy} onclick={() => (editing = false)}>Cancel</button>
        <button class="btn small primary" disabled={editBusy || editText.trim() === ''} onclick={saveEdit}>{editBusy ? 'Saving…' : 'Save'}</button>
      </div>
    </div>
  {:else}
    <div class="cmt-body md-body">
      <!-- renderMarkdown escapes HTML before transforming -->
      {@html renderMarkdown(comment.body)}
    </div>
  {/if}
  {#if showReactions && actions}<div class="cmt-rx"><PrReactions {comment} {actions} /></div>{/if}
  {#if onreply || canResolve}
    <div class="cmt-actions">
      {#if replying}
        <textarea dir="auto" class="input" rows="2" bind:value={replyText} disabled={busy} aria-label="Reply" placeholder="Reply…"></textarea>
        {#if replyError}<p class="reply-error" use:announceShown={{ text: `Couldn’t post reply. ${replyError}`, assertive: true }}>Couldn’t post reply. {replyError} Try Reply again.</p>{/if}
        <div class="row" style="justify-content: flex-end">
          <button class="btn small" disabled={busy} onclick={() => (replying = false)}>Cancel</button>
          <button class="btn small primary" disabled={busy || replyText.trim() === ''} onclick={submitReply}>
            {busy ? 'Posting…' : 'Reply'}
          </button>
        </div>
      {:else}
        <div class="row cmt-action-row">
          {#if onreply}
            <button class="btn small ghost" onclick={() => (replying = true)}>Reply</button>
          {/if}
          {#if canResolve}
            <button
              class="btn small ghost"
              disabled={resolveBusy}
              title={comment.resolved ? 'Reopen this thread on the provider' : 'Mark this thread resolved on the provider'}
              onclick={toggleResolved}
            >{resolveBusy ? '…' : comment.resolved ? 'Reopen' : 'Resolve'}</button>
          {/if}
        </div>
      {/if}
    </div>
  {/if}
  {#each comment.replies as r (r.id)}
    <CommentThread comment={r} {onreply} depth={depth + 1} root={root ?? comment} />
  {/each}
</div>

<style>
  .reply-error { margin: 0; color: var(--danger); font-size: var(--fs-s); }
  .cmt {
    padding: 8px 0 2px;
  }
  .cmt-head {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 2px;
    flex-wrap: wrap;
  }
  .cmt-avatar {
    width: 18px;
    height: 18px;
    border-radius: 50%;
    background: var(--accent-soft-strong);
    color: var(--accent-text);
    font-size: var(--fs-xs);
    font-weight: 600;
    display: grid;
    place-items: center;
  }
  .cmt-author {
    font-size: var(--fs-s);
    font-weight: 600;
  }
  .cmt-date {
    font-size: var(--fs-xs);
  }
  .cmt-more { margin-inline-start: auto; padding-inline: 6px; }
  .cmt-review {
    padding: 0 8px;
    border-radius: 999px;
    font-size: var(--fs-xs);
    font-weight: 600;
  }
  .tone-success { color: var(--success); background: var(--success-soft); }
  .tone-warning { color: var(--warning); background: var(--warning-soft); }
  .tone-info { color: var(--info); background: var(--info-soft); }
  .tone-dim { color: var(--text-dim); background: var(--surface-2); }
  .cmt-answers { font-size: var(--fs-xs); margin-inline-start: 24px; }
  .cmt-rx { margin-inline-start: 24px; }
  @media (max-width: 1024px), (pointer: coarse) {
    .cmt-more { min-height: 44px; min-width: 44px; justify-content: center; }
  }
  .cmt-body {
    font-size: var(--fs-s);
    margin-inline-start: 24px;
  }
  .cmt-actions {
    margin: 4px 0;
    margin-inline-start: 24px;
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
    .cmt-body, .cmt-answers, .cmt-rx { margin-inline-start: 0; }
    .cmt-actions { margin-inline-start: 0; }
    .cmt-actions .btn { min-height: 38px; }
  }
</style>
