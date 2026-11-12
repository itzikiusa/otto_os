<script lang="ts">
  // The PR Summary's Conversation: general comment threads (GitHub review
  // summaries among them, with their verdict chip) and the new-comment
  // composer. The draft is the parent's (bound), so its leave-guard sees it.
  // Edit / delete / reactions come down from PrDetail via pr-comment-ctx.
  import type { PrComment } from '../../lib/api/types';
  import { scrollBehavior } from '../../lib/motion';
  import CommentThread from './CommentThread.svelte';

  interface Props {
    comments: PrComment[];
    number: number;
    repoLabel: string;
    providerName: string;
    /** `PrDetail.reactions_supported` — false: say so once, quietly. */
    reactionsSupported: boolean | undefined;
    draft: string;
    posting: boolean;
    onpost: () => void;
    onreply: (parentId: string, body: string) => Promise<void>;
    onresolve: (threadId: string, resolved: boolean) => Promise<void>;
  }
  let { comments, number, repoLabel, providerName, reactionsSupported, draft = $bindable(), posting, onpost, onreply, onresolve }: Props = $props();
  const noReactions = $derived(
    reactionsSupported === false ? `${providerName === 'Bitbucket' ? 'Bitbucket Cloud' : providerName} has no reactions` : null,
  );

  function scrollIntoViewOnFocus(e: FocusEvent): void {
    const el = e.currentTarget as HTMLElement;
    setTimeout(() => el.scrollIntoView({ block: 'center', behavior: scrollBehavior() }), 250);
  }
</script>

<section class="prd-comments">
  <h3 class="prd-h">Conversation <span class="dim">{comments.length}</span></h3>
  {#if noReactions}<p class="dim prd-note" data-testid="pr-no-reactions">{noReactions}</p>{/if}
  {#each comments as c (c.id)}
    <div class="card prd-thread">
      <CommentThread comment={c} {onreply} {onresolve} />
    </div>
  {:else}
    <p class="dim prd-none">No comments yet.</p>
  {/each}
  <div class="new-comment card">
    <textarea dir="auto" class="input" rows="3" bind:value={draft} disabled={posting} aria-label="New comment" placeholder="Leave a comment…" onfocus={scrollIntoViewOnFocus}></textarea>
    <div class="row prd-compose-foot">
      <span class="hint dim pc-hint">Posts to PR #{number} on {repoLabel} · visible to the author and reviewers</span>
      <button class="btn small" disabled={posting || draft.trim() === ''} onclick={onpost}>
        {posting ? 'Posting…' : 'Post comment'}
      </button>
    </div>
  </div>
</section>

<style>
  .prd-h {
    margin: 0 0 8px;
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--text-dim);
  }
  .prd-note { margin: 0 0 8px; font-size: var(--fs-xs); }
  .prd-thread { padding: 4px 14px 8px; margin-bottom: 8px; }
  .prd-none { font-size: var(--fs-s); margin: 0 0 8px; }
  .new-comment { padding: 12px; }
  .new-comment textarea { width: 100%; }
  .prd-compose-foot { justify-content: flex-end; align-items: center; flex-wrap: wrap; gap: 8px; margin-top: 8px; }
  .pc-hint { flex: 1 1 220px; min-width: 0; font-size: var(--fs-xs); }
  @media (max-width: 1024px) {
    .new-comment textarea { font-size: var(--fs-l); }
  }
</style>
