<script lang="ts">
  // The body of History's Detail pane (arch-1-09: split out of GraphView.svelte):
  // the WIP staging panel when the working-tree row is selected, otherwise the
  // selected commit's header (sha, refs, subject, author, the "on <branch>"
  // hint) over its summary-first diff. GraphView owns the pane (DetailPane:
  // its title, Close and width) and the selection (graphSelection.svelte.ts).
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
    /** The pane is a sheet (a phone, a narrow tablet): the WIP list opens no diff by itself. */
    compact: boolean;
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
    oncommitted: (sha: string) => void;
  }
  let {
    repoId,
    status,
    onstatus,
    onresolve,
    compact,
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

<div class="detail-panel">
  {#if wipSelected}
    <!-- Every change counts as included, the message drafts itself, the first
         file's diff opens beside the list (an inline pane). The pane's Close
         closes it. -->
    <WipPanel {repoId} {status} {onstatus} {oncommitted} {onresolve} remote={!!repo?.remote_url} selectFirst={!compact} autoDraft includeAll />
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
        <!-- One line that never wraps: sha, then the refs (each ellipsizes its
             own label, the group shrinks first). -->
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
    <GraphCommitDiff {repoId} sha={diffSha} held={diffHeld} {onresume} {onfiletools} />
  {/if}
</div>

<style>
  .detail-panel {
    flex: 1;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    background: var(--surface);
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
  .dim {
    color: var(--text-dim);
  }
  .mono {
    font-family: var(--font-mono);
  }

</style>
