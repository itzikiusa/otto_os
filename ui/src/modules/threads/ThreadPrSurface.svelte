<script lang="ts">
  // Right-panel "Pull request" surface: the open PR from the thread's branch —
  // title, state, checks, branches — with Open in Git and Open on the forge.
  import type { Repo, PrDetail } from '../../lib/api/types';
  import { api, isAbortError } from '../../lib/api/client';
  import { loadErrorText } from '../../lib/loadError';
  import Markdown from '../agents/conversation/Markdown.svelte'; // ui-guards: allow — shared chat markdown
  import LoadState from '../../lib/components/LoadState.svelte';
  import { reviewCommentContext, type ComposerContext } from './runtime/composerContext';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import { openExternal } from '../../lib/external';
  import { router } from '../../lib/router.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import { threadPrs } from './threadPrs.svelte';
  import { forgeLabel } from './threadsHelpers';

  interface Props {
    repo: Repo | null;
    branch: string | null;
    threadId?: string | null;
    oncontext?: (context: ComposerContext, threadId: string) => void;
  }
  let { repo, branch, threadId = null, oncontext }: Props = $props();

  const pr = $derived(threadPrs.prFor(repo, branch));
  const checked = $derived(threadPrs.checked(repo, branch));
  const forge = $derived(forgeLabel(repo));
  let detail = $state.raw<PrDetail | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let retry = $state(0);
  $effect(() => {
    const rid = repo?.id;
    const number = pr?.number;
    // Re-fetch on thread changes even when two threads share a branch. A late
    // response from the previous owner must never become selectable here.
    void threadId;
    void retry;
    detail = null;
    error = null;
    loading = false;
    if (!rid || !number) return;
    const ctl = new AbortController();
    loading = true;
    void api.get<PrDetail>(`/repos/${rid}/prs/${number}`, ctl.signal)
      .then((result) => { if (!ctl.signal.aborted) detail = result; })
      .catch((e) => { if (!ctl.signal.aborted && !isAbortError(e)) error = loadErrorText(e); })
      .finally(() => { if (!ctl.signal.aborted) loading = false; });
    return () => ctl.abort();
  });
</script>

<div class="tps" data-testid="threads-pr-surface">
  {#if !checked}
    <div class="tps-pad"><Skeleton rows={4} height={22} label="the pull request" /></div>
  {:else if !pr || !repo}
    <EmptyState
      variant="panel"
      icon="pr"
      title="No open pull request"
      body={branch ? `Nothing open from ${branch}${forge ? ` on ${forge}` : ''}.` : 'This thread has no branch.'}
    />
  {:else}
    <div class="tps-pad">
      <p class="tps-num mono">#{pr.number} · {pr.state}{pr.draft ? ' · draft' : ''}</p>
      <h3 class="tps-title" dir="auto">{pr.title}</h3>
      <dl class="tps-meta">
        <dt>Branches</dt>
        <dd class="mono" dir="ltr">{pr.source_branch} → {pr.target_branch}</dd>
        <dt>Author</dt>
        <dd>{pr.author}</dd>
        {#if pr.ci_status}
          <dt>Checks</dt>
          <dd>{pr.ci_status}</dd>
        {/if}
        <dt>Updated</dt>
        <dd>{rel(pr.updated_at)}</dd>
      </dl>
      <div class="tps-actions">
        <button type="button" class="btn small" onclick={() => router.go(`git/${encodeURIComponent(repo!.id)}/pr/${pr!.number}`)}>
          <Icon name="pr" size={12} /> Open in Git
        </button>
        <button type="button" class="btn small ghost" onclick={() => void openExternal(pr!.url)}>
          <Icon name="external" size={12} /> {forge ?? 'Forge'}
        </button>
        <button
          type="button"
          class="icon-btn"
          aria-label="Refresh pull request"
          title="Refresh"
          onclick={() => { threadPrs.forget(repo, branch); retry++; }}
        >
          <Icon name="refresh" size={12} />
        </button>
      </div>
      <section class="tps-comments" aria-label="Review conversations">
        <LoadState what="review conversations" variant="panel" {loading} {error} empty={!detail?.comments.length} onretry={() => retry++}>
          {#snippet emptyView()}<p class="tps-num">No review conversations yet.</p>{/snippet}
          {#each detail?.comments ?? [] as comment (comment.id)}
            <article class="tps-comment">
              <div class="tps-comment-head"><strong>{comment.author}</strong>
                {#if oncontext && threadId}
                  <button type="button" class="btn small" data-testid="pr-comment-context-{comment.id}"
                    onclick={() => oncontext?.(reviewCommentContext(repo!.id, pr!.url, comment), threadId!)}>Add to chat</button>
                {/if}
              </div>
              {#if comment.path}<p class="tps-num mono">{comment.path}{comment.line != null ? `:${comment.line}` : ''}{comment.side === 'old' ? ' (old)' : ''}</p>{/if}
              {#if comment.outdated || comment.resolved}<p class="tps-num">{comment.outdated ? 'Outdated' : ''}{comment.resolved ? ' Resolved' : ''}</p>{/if}
              <Markdown md={comment.body} />
              {#each comment.replies as reply (reply.id)}
                <div class="tps-reply"><strong>{reply.author}</strong><Markdown md={reply.body} /></div>
              {/each}
            </article>
          {/each}
        </LoadState>
      </section>
    </div>
  {/if}
</div>

<style>
  .tps {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .tps-pad {
    padding: 12px;
  }
  .tps-num {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .tps-title {
    margin: 4px 0 10px;
    font-size: var(--fs-l);
    font-weight: 500;
  }
  .tps-meta {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 4px 12px;
    margin: 0 0 12px;
    font-size: var(--fs-s);
  }
  .tps-meta dt {
    color: var(--text-dim);
  }
  .tps-meta dd {
    margin: 0;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .tps-comments { margin-block-start: 16px; }
  .tps-comment { padding-block: 10px; border-block-start: 1px solid var(--border); overflow-wrap: anywhere; }
  .tps-comment-head { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  .tps-reply { border-inline-start: 2px solid var(--border); padding-inline-start: 10px; margin-block-start: 8px; }
  .tps-actions {
    flex-wrap: wrap;
    display: flex;
    align-items: center;
    gap: 6px;
  }
</style>
