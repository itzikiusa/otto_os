<script lang="ts">
  // The git graph's right pane (arch-1-09: split out of GraphView.svelte): the
  // WIP staging panel when the working-tree row is selected, otherwise the
  // selected commit's header (sha, refs, subject, author, the "on <branch>"
  // hint) over its summary-first diff. On a phone it is the accordion's last
  // section; GraphView owns the selection (graphSelection.svelte.ts).
  import type { CommitInfo, RepoStatusResp } from '../../../lib/api/types';
  import { rel } from '../../../lib/stores/now.svelte';
  import { git } from '../../../lib/stores/git.svelte';
  import { forgeLabel, prStepPossible } from '../../../lib/gitNextStep';
  import { router } from '../../../lib/router.svelte';
  import LazyMount from '../../../lib/components/LazyMount.svelte';
  import { lazyComponent } from '../../../lib/lazy-component.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import WipPanel from '../WipPanel.svelte';
  import GraphCommitDiff from '../graph-commit-diff.svelte';
  import RefChip from './RefChip.svelte';
  import type { RefChip as Chip } from '../ref-menus';

  interface Props {
    repoId: string;
    status: RepoStatusResp;
    onstatus: (s: RepoStatusResp) => void;
    onresolve?: () => void;
    /** Desktop: the pane takes the rest of the width when open. */
    open: boolean;
    isMobile: boolean;
    wipSelected: boolean;
    selectedSha: string | null;
    selectedCommit: CommitInfo | null;
    /** The selected commit's ref chips (GraphView's `chipsFor`). */
    chips: Chip[];
    stashLabel: (sha: string) => string;
    /** The branch the highlighted spine resolves to, if any. */
    selectedBranch: string | null;
    diffSha: string | null;
    diffHeld: boolean;
    onresume: () => void;
    onfiletools: (e: MouseEvent, path: string) => void;
    fileCount?: number | null;
    onclose: () => void;
    oncommitted: (sha: string) => void;
  }
  let {
    repoId,
    status,
    onstatus,
    onresolve,
    open,
    isMobile,
    wipSelected,
    selectedSha,
    selectedCommit,
    chips,
    stashLabel,
    selectedBranch,
    diffSha,
    diffHeld,
    onresume,
    onfiletools,
    fileCount = $bindable(null),
    onclose,
    oncommitted,
  }: Props = $props();

  const repo = $derived(git.allRepos.find((r) => r.id === repoId) ?? git.repos.find((r) => r.id === repoId) ?? null);
  // A pushed, clean HEAD: the commit card's next step (Create / Open pull
  // request) above its detail. Push and Publish are the header's verbs.
  const CommitCardLazy = lazyComponent(() => import('../CommitCard.svelte'));
  const showStep = $derived(!!repo && chips.some((c) => c.current) && prStepPossible({
    changes: status.changes.length, branch: status.branch, upstream: status.upstream, ahead: status.ahead,
    remote: !!repo.remote_url, forge: !!forgeLabel(repo), prNumber: null,
  }));

  /** The shared relative-time label (content.md §7), compact for dense rows. */
  function fmtDate(iso: string): string {
    return rel(iso, { compact: true });
  }
  function fmtDateTitle(iso: string): string {
    const d = new Date(iso);
    return Number.isNaN(d.getTime()) ? '' : d.toLocaleString();
  }
</script>

<div
  class="detail-panel"
  class:detail-visible={open && !isMobile}
  class:mob-visible={isMobile && open}
  class:mob-hidden={isMobile && !open}
>
  {#if wipSelected}
    <!-- Every change counts as included, the message drafts itself, the first
         file's diff opens beside the list (desktop). -->
    <!-- A phone closes it from the section header above (one ✕). -->
    <WipPanel {repoId} {status} {onstatus} {oncommitted} onclose={isMobile ? undefined : onclose} {onresolve} remote={!!repo?.remote_url} selectFirst={!isMobile} autoDraft includeAll />
  {:else if selectedSha === null}
    <div class="detail-empty">
      <span class="detail-empty-label">COMMIT</span>
      <span class="dim detail-empty-hint">select a commit</span>
    </div>
  {:else if selectedCommit !== null}
    {#if showStep && repo}
      <LazyMount lazy={CommitCardLazy} what="the next step" quiet props={{ repo, layout: 'step', onopenpr: (n: number) => router.go(`git/${repoId}/pr/${n}`) }} />
    {/if}
    <div class="detail-header">
      <div class="detail-header-main">
        <!-- One line that never wraps: sha, the refs (each ellipsizes its own
             label, the group shrinks first), then close pinned at the end. -->
        <div class="detail-title-row">
          <span class="mono detail-sha">{selectedCommit.short_sha}</span>
          <span class="detail-refs">
            {#each chips as chip}
              <RefChip
                {chip}
                label={chip.kind === 'stash' ? stashLabel(selectedCommit.sha) : chip.label}
                title={chip.worktree && !chip.current
                  ? `Worktree · ${chip.label}`
                  : chip.current
                    ? `Checked out · ${chip.label}`
                    : chip.label}
              />
            {/each}
          </span>
          {#if !isMobile}
            <button class="icon-btn detail-close" onclick={onclose} title="Close commit detail" aria-label="Close commit detail">
              <Icon name="x" size={14} />
            </button>
          {/if}
        </div>
        <div class="detail-subject">{selectedCommit.subject}</div>
        <div class="detail-meta">
          <span class="detail-author">{selectedCommit.author}</span>
          <span class="dim detail-dot">·</span>
          <span class="dim detail-date" title={fmtDateTitle(selectedCommit.date)}>{fmtDate(selectedCommit.date)}</span>
          {#if selectedBranch && !chips.some((c) => c.label === selectedBranch)}
            <span class="on-branch-hint" title="On branch {selectedBranch}">
              <Icon name="branch" size={12} /><span class="chip-label">{selectedBranch}</span>
            </span>
          {/if}
        </div>
      </div>
    </div>

    <!-- Diff area: summary-first, per-file lazy (see graph-commit-diff). -->
    <GraphCommitDiff {repoId} sha={diffSha} held={diffHeld} {onresume} {onfiletools} bind:fileCount />
  {/if}
</div>

<style>
  .detail-panel {
    width: 0;
    min-width: 0;
    flex-shrink: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    border-inline-start: 1px solid var(--border);
    background: var(--surface);
  }
  .detail-panel.detail-visible {
    /* Fill all remaining width after refs + commit-list so the page isn't
       half-empty when a commit is open. */
    flex: 1 1 0;
    width: auto;
    min-width: 360px;
    overflow: hidden;
  }
  .detail-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    gap: 6px;
  }
  .detail-empty-label {
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: .06em;
    color: var(--text-dim);
    text-transform: uppercase;
  }
  .detail-empty-hint {
    font-size: var(--fs-s);
  }
  .detail-header {
    flex-shrink: 0;
    padding: 10px 12px 8px;
    border-bottom: 1px solid var(--border);
    background: var(--surface-2);
  }
  .detail-header-main {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .detail-title-row {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .detail-sha {
    flex-shrink: 0;
    font-size: var(--fs-xs);
    color: var(--accent-text);
    font-weight: 600;
    letter-spacing: .06em;
  }
  .detail-refs {
    display: flex;
    align-items: center;
    gap: 4px;
    flex: 1;
    min-width: 0;
    overflow: hidden;
  }
  .detail-refs :global(.ref-chip) {
    flex-shrink: 1;
    min-width: 0;
  }
  .detail-subject {
    font-size: var(--fs-m);
    font-weight: 600;
    color: var(--text);
    line-height: 1.35;
    word-break: break-word;
  }
  .detail-meta {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    font-size: var(--fs-xs);
  }
  .detail-author {
    color: var(--text-dim);
    font-size: var(--fs-xs);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .detail-dot {
    font-size: var(--fs-xs);
    flex-shrink: 0;
  }
  .detail-date {
    font-size: var(--fs-xs);
    flex-shrink: 0;
  }
  /* "on <branch>" hint in the open commit's detail header. */
  .on-branch-hint {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    min-width: 0;
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
    padding: 1px 8px;
    border-radius: 999px;
    background: var(--surface-2);
    white-space: nowrap;
  }
  .chip-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  /* An .icon-btn; only its placement is local. */
  .detail-close {
    margin-inline-start: auto;
    flex-shrink: 0;
  }
  .dim {
    color: var(--text-dim);
  }
  .mono {
    font-family: var(--font-mono);
  }

  /* A narrow desktop pane (a 1100 px window with the right panel open) has no
     room for refs + list + detail: while the detail is open the refs step
     aside and the list and the detail share the width. */
  :global(.graphview) {
    container: graph / inline-size;
    position: relative;
  }
  :global(.refs-peek) {
    display: none;
  }
  @container graph (max-width: 900px) {
    /* …and peek over the list from the branch button (the refs header's spot). */
    :global(.graphview:has(> .detail-visible) > .refs-peek) {
      display: inline-flex;
      position: absolute;
      inset-block-start: 2px;
      inset-inline-start: 4px;
      z-index: 7;
    }
    :global(.graphview.peek:has(> .detail-visible) > .refs-panel) {
      display: block;
      position: absolute;
      inset-block: 0;
      inset-inline-start: 0;
      width: min(260px, 70%);
      z-index: 6;
      background: var(--surface);
      box-shadow: var(--glass-shadow);
    }
    :global(.graphview.peek:has(> .detail-visible) > .refs-peek) {
      color: var(--accent-text);
    }
    :global(.graphview:has(> .detail-visible) > .refs-panel),
    :global(.graphview:has(> .detail-visible) > .refs-resizer) {
      display: none;
    }
    :global(.graphview:has(> .detail-visible) > .graph-panel) {
      flex-basis: 45% !important;
      min-width: 0;
    }
    .detail-panel.detail-visible {
      min-width: 0;
    }
  }

  /* ── Mobile + tablet (≤1024px): the accordion's diff section ── */
  @media (max-width: 1024px) {
    /* Diff/detail pane: the prominent section once a commit is open. */
    .detail-panel.mob-visible,
    .detail-panel.mob-hidden {
      width: 100%;
      border-inline-start: none;
    }
    .detail-panel.mob-visible {
      flex: 1 1 auto;
      min-width: 0;
      min-height: 50vh;
      overflow: hidden;
      display: flex;
    }
    /* No commit selected → drop the empty diff pane so the list fills the view. */
    .detail-panel.mob-hidden {
      display: none !important;
    }
    .mob-visible .detail-subject { font-size: var(--fs-l); }
    .mob-visible .detail-sha { font-size: var(--fs-s); }
    .mob-visible .detail-author,
    .mob-visible .detail-date { font-size: var(--fs-s); }
  }
</style>
