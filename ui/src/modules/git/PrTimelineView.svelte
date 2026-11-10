<script lang="ts">
  // The PR Timeline tab: opened → pushes → comments → approvals → merged,
  // assembled by prTimeline.ts from the detail + commits the PR view loaded.
  import type { PrCommit, PrDetail } from '../../lib/api/types';
  import Icon, { type IconName } from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { renderMarkdown } from '../../lib/md';
  import { plural } from '../../lib/plural';
  import { rel } from '../../lib/stores/now.svelte';
  import { buildTimeline, type TimelineEvent } from './prTimeline';

  interface Props {
    pr: PrDetail;
    commits: PrCommit[] | null;
    commitsError: string | null;
    /** An inline comment's file link: go to the Code tab. */
    onopencode: () => void;
    /** A commit sha: show that commit's changes. */
    onpickcommit: (sha: string) => void;
  }
  let { pr, commits, commitsError, onopencode, onpickcommit }: Props = $props();

  const events = $derived(buildTimeline(pr, commits));
  function icon(ev: TimelineEvent): IconName {
    switch (ev.kind) {
      case 'opened': return 'pr';
      case 'commits': return 'commit';
      case 'comment': return 'comment';
      case 'approved': return 'check';
      case 'closed': return ev.state === 'merged' ? 'merge' : 'x';
    }
  }
</script>

<section class="tlw" aria-label="Timeline">
  {#if commits === null && !commitsError}
    <LoadState what="the timeline" loading empty />
  {:else}
    {#if commitsError}<p class="dim none">Commits couldn’t load ({commitsError}); showing the rest.</p>{/if}
    <ol class="tl">
      {#each events as ev, i (i)}
        <li class="item k-{ev.kind}">
          <span class="dot" aria-hidden="true"><Icon name={icon(ev)} size={12} /></span>
          <div class="body">
            <div class="line">
              {#if ev.who}<strong>{ev.who}</strong>{/if}
              {#if ev.kind === 'opened'} opened this pull request
              {:else if ev.kind === 'commits'} pushed {plural(ev.commits.length, 'commit')}
              {:else if ev.kind === 'comment'} commented{#if ev.comment.path}{' '}on <button class="link mono" onclick={onopencode}>{ev.comment.path.split('/').pop()}{ev.comment.line != null ? `:${ev.comment.line}` : ''}</button>{/if}{#if ev.replies > 0}<span class="dim">{' · '}{plural(ev.replies, 'reply', 'replies')}</span>{/if}{#if ev.comment.resolved}<span class="chip ok chip-gap">Resolved</span>{/if}
              {:else if ev.kind === 'approved'} approved these changes
              {:else if ev.kind === 'closed'}{ev.state === 'merged' ? 'Merged' : 'Closed without merging'}
              {/if}
              <span class="dim at" title={new Date(ev.at).toLocaleString()}>{rel(ev.at)}</span>
            </div>
            {#if ev.kind === 'commits'}
              <ul class="commits">
                {#each ev.commits as c (c.sha)}
                  <li>
                    <button class="link mono sha" onclick={() => onpickcommit(c.sha)} title="Show this commit’s changes">{c.short_sha}</button>
                    <span class="ell">{c.subject}</span>
                  </li>
                {/each}
              </ul>
            {:else if ev.kind === 'comment'}
              <!-- renderMarkdown escapes input before transforming -->
              <div class="md-body comment">{@html renderMarkdown(ev.comment.body)}</div>
            {/if}
          </div>
        </li>
      {:else}
        <li class="dim none">No activity yet.</li>
      {/each}
    </ol>
  {/if}
</section>

<style>
  .none { font-size: var(--fs-s); margin: 0 0 8px; }
  .tl { list-style: none; margin: 0; padding: 0; max-width: 820px; position: relative; }
  .tl::before {
    content: '';
    position: absolute;
    inset-block: 8px;
    inset-inline-start: 11px;
    border-inline-start: 1px solid var(--separator);
  }
  .item { position: relative; display: flex; gap: 12px; padding-block: 8px; }
  .dot {
    position: relative;
    flex: none;
    width: 24px;
    height: 24px;
    border-radius: 50%;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: var(--surface-2);
    border: 1px solid var(--border);
    color: var(--text-dim);
  }
  .k-approved .dot, .k-opened .dot { color: var(--success); background: var(--success-soft); }
  .k-closed .dot { color: var(--accent-text); background: var(--accent-soft); }
  .body { min-width: 0; flex: 1; padding-block-start: 2px; }
  .line { font-size: var(--fs-s); overflow-wrap: anywhere; }
  .at { margin-inline-start: 6px; font-size: var(--fs-xs); }
  .chip-gap { margin-inline-start: 6px; }
  .commits { list-style: none; margin: 6px 0 0; padding: 0; display: flex; flex-direction: column; gap: 4px; font-size: var(--fs-s); }
  .commits li { display: flex; gap: 10px; min-width: 0; }
  .sha { flex: none; font-size: var(--fs-xs); }
  .ell { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .comment {
    margin-top: 6px;
    padding: 8px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    font-size: var(--fs-s);
  }
  .link { background: none; border: none; padding: 0; color: var(--accent-text); cursor: pointer; font: inherit; }
  .link:hover { text-decoration: underline; }
</style>
