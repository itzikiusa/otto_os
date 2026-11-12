<script lang="ts">
  import { announceShown } from '../../lib/announce.svelte';
  import { plural } from '../../lib/plural';
  // Merge confirmation for a hosted PR. Nothing merges until the user clicks
  // Merge here — and the modal first shows everything that argues against it:
  // per-check CI, approvals, mergeability, open blocker findings, and the two
  // facts only the local checkout knows (unpushed commits, branch freshness).
  // Blocking reasons disable Merge until "Merge anyway" is ticked.
  import { onDestroy, untrack } from 'svelte';
  import { api, ApiError } from '../../lib/api/client';
  import type { MergeStrategy, PrChecksResp, PrCommit, PrDetail, PrReadiness } from '../../lib/api/types';
  import { router } from '../../lib/router.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { openExternal } from '../../lib/external';
  import Modal from '../../lib/components/Modal.svelte';
  import Icon, { type IconName } from '../../lib/components/Icon.svelte';
  import { pinnedHead } from './pr-cache';
  import { unseenHead, type UnseenHead } from './pr-merge-gate';
  import { mergeStrategies } from './pr-merge-strategy';
  import { mergeCheckReasons } from './pr-merge-checks';
  import PrMergeUnseen from './PrMergeUnseen.svelte';

  interface Props {
    repoId: string;
    number: number;
    pr: PrDetail;
    onclose: () => void;
    onmerged: () => void;
    /** The PR moved under the modal (a pinned merge 409'd): the parent should
     *  re-read its own (now stale) copy of the PR. */
    onstale?: () => void;
    /** Pre-selected strategy (the PR header's ⋯ "Squash and merge" etc.). */
    initialStrategy?: MergeStrategy;
    /** The head the PR's diff view was loaded at — what the user actually read
     *  ('' / omitted = no diff loaded, nothing to compare). */
    reviewedHead?: string | null;
    /** Close the dialog and reload the PR + its diff ("Reload the diff"). */
    onreload?: () => void;
    /** The repo's forge (`github` | `bitbucket` | `gitlab`): strategy labels
     *  say what that forge actually does. */
    provider?: string | null;
  }
  let { repoId, number, pr, onclose, onmerged, onstale, initialStrategy, reviewedHead = null, onreload, provider = null }: Props = $props();

  let disposed = false;
  onDestroy(() => { disposed = true; });

  let strategy: MergeStrategy = $state(untrack(() => initialStrategy) ?? 'merge');
  let deleteSource = $state(false);
  let mergeAnyway = $state(false);
  let merging = $state(false);
  let error = $state<string | null>(null);

  // Both probes are best-effort, but a failed probe is NOT "all clear": the
  // row reads "Unavailable" with a Retry, and Merge needs "Merge anyway" until
  // CI / readiness could actually be verified.
  let checks = $state<PrChecksResp | null>(null);
  let checksLoading = $state(true);
  let checksError = $state(false);
  let readiness = $state<PrReadiness | null>(null);
  let readinessLoading = $state(true);
  let readinessError = $state(false);
  // Bumped by Retry to re-run the probes.
  let probeTick = $state(0);

  // The head the merge is PINNED to, read by the modal itself alongside the
  // probes (S15-305). `pr` is the parent's copy — possibly a cache hit from
  // minutes ago — so pinning `pr.head_sha` 409'd forever after a push while
  // the CI row showed the live head's checks.
  let liveHead = $state<string | null>(null);
  let headLoading = $state(true);
  $effect(() => {
    const id = repoId;
    const n = number;
    void probeTick;
    headLoading = true;
    void api
      .get<PrDetail>(`/repos/${id}/prs/${n}`)
      .then((r) => (liveHead = r.head_sha ?? null))
      // Unreadable: fall back to the parent's head — the daemon still refuses
      // a merge whose head moved, so this can only fail closed.
      .catch(() => (liveHead = null))
      .finally(() => (headLoading = false));
  });
  const pinSha = $derived(pinnedHead(liveHead, pr.head_sha ?? null));

  // A push after the diff was read (not just during this dialog): Merge waits
  // for "including changes I have not reviewed". The commit list is read only
  // then, to say how many commits you haven't seen. `reviewed` is fixed at
  // open: a 409 makes the parent reload its diff in the background, which
  // must not count as having read it.
  const reviewed = untrack(() => reviewedHead);
  let unseenCommits = $state.raw<PrCommit[] | null>(null);
  let ackUnseen = $state(false);
  const unseen: UnseenHead = $derived(headLoading ? { moved: false, count: null } : unseenHead(pinSha, reviewed, unseenCommits));
  $effect(() => {
    if (!unseen.moved || unseenCommits) return;
    const id = repoId;
    const n = number;
    void api.get<PrCommit[]>(`/repos/${id}/prs/${n}/commits`).then((c) => { if (!disposed) unseenCommits = c; }).catch(() => {});
  });

  $effect(() => {
    const id = repoId;
    const n = number;
    void probeTick;
    checksLoading = true;
    checksError = false;
    void api
      .get<PrChecksResp>(`/repos/${id}/prs/${n}/checks`)
      .then((r) => (checks = r))
      .catch(() => {
        checks = null;
        checksError = true;
      })
      .finally(() => (checksLoading = false));
  });

  $effect(() => {
    const id = repoId;
    const n = number;
    void probeTick;
    readinessLoading = true;
    readinessError = false;
    void api
      .get<PrReadiness>(`/repos/${id}/prs/${n}/readiness`)
      .then((r) => (readiness = r))
      .catch(() => {
        readiness = null;
        readinessError = true;
      })
      .finally(() => (readinessLoading = false));
  });

  const blockers = $derived(readiness?.review?.unresolved_blocker_count ?? 0);
  const mergeable = $derived(readiness?.mergeable ?? pr.mergeable ?? null);

  // What argues against merging. Empty ⇒ Merge is enabled outright.
  const reasons = $derived(
    [
      checks?.ci.state === 'failure' ? 'CI failing' : null,
      mergeable === false ? 'Not mergeable' : null,
      blockers > 0 ? plural(blockers, 'blocker finding') : null,
      checksError || readinessError ? 'Unable to verify CI / readiness' : null,
      // The forge's own merge checks + open tasks (CI ones are the row above).
      ...mergeCheckReasons(pr),
    ].filter((r): r is string => r !== null),
  );
  const loading = $derived(checksLoading || readinessLoading || headLoading);
  const canMerge = $derived(!merging && !loading && (reasons.length === 0 || mergeAnyway) && (!unseen.moved || ackUnseen));

  const FRESHNESS: Record<string, string> = {
    fresh: 'Up to date with the target branch',
    behind: 'Behind the target branch',
    unknown: 'Unknown (branch not checked out locally)',
  };

  function checkIcon(state: string): { icon: IconName; cls: string } {
    if (state === 'success') return { icon: 'check', cls: 'ok' };
    if (state === 'failure') return { icon: 'x', cls: 'bad' };
    if (state === 'skipped' || state === 'neutral') return { icon: 'dot', cls: 'dim' };
    // Anything else is still pending / in progress — waiting on CI.
    return { icon: 'clock', cls: 'warn' };
  }

  /** Persist the Review tab and route to the PR — PrDetail honours the stored
   *  tab when this modal closes, so the blockers link lands on the findings. */
  function openReview(): void {
    try {
      localStorage.setItem(`otto_pr_tab_${repoId}_${number}`, 'review');
    } catch {
      /* private mode — the route still moves */
    }
    router.go(`git/${repoId}/pr/${number}`);
    onclose();
  }

  async function doMerge(): Promise<void> {
    if (!canMerge) return;
    const mergedNumber = number;
    merging = true;
    error = null;
    try {
      // Pin the merge to the head these checks/readiness rows describe: a push
      // while the modal is open must 409, not merge unreviewed commits.
      await api.post(`/repos/${repoId}/prs/${number}/merge`, {
        strategy,
        delete_source_branch: deleteSource,
        ...(pinSha ? { expected_head_sha: pinSha } : {}),
      });
      toasts.success('PR merged', `#${mergedNumber}`);
      if (!disposed) onmerged();
    } catch (e) {
      // Keep the provider's own words — "Required status check failing" and
      // friends are the actionable text.
      const moved = e instanceof ApiError && e.status === 409 && /head|changed|sha/i.test(e.message);
      const detail = moved
        ? 'The pull request changed since these checks ran (new commits were pushed). The checks below were re-read for the new head — review them, then merge again.'
        : e instanceof ApiError || e instanceof Error
          ? e.message
          : String(e);
      if (disposed) toasts.error(`Couldn’t merge PR #${mergedNumber}`, detail);
      else error = detail;
      // Recover instead of dead-ending: re-read the head + probes here, and let
      // the parent refresh its stale copy. Merge stays a fresh, explicit click.
      if (moved && !disposed) {
        mergeAnyway = false;
        ackUnseen = false;
        unseenCommits = null;
        probeTick++;
        onstale?.();
      }
    } finally {
      merging = false;
    }
  }
</script>

<Modal title="Merge pull request" width={520} {onclose}>
  <div class="body">
    <div class="prline">
      <span class="dim">#{number}</span>
      <span class="ptitle">{pr.title}</span>
    </div>
    <div class="flow mono dim">
      {pr.source_branch} <span class="dir-arrow">→</span> {pr.target_branch}
      {#if pinSha}<span title="The merge is pinned to this head commit">· at {pinSha.slice(0, 7)}</span>{/if}
    </div>

    {#if unseen.moved}
      <PrMergeUnseen {unseen} bind:ack={ackUnseen} disabled={merging} onreload={() => (onreload ?? onclose)()} />
    {/if}

    <!-- readiness rows -->
    <div class="rows">
      <div class="row-item">
        <span class="rlabel">CI</span>
        <span class="rvalue">
          {#if checksLoading}
            <span class="spinner" role="img" aria-label="Loading checks" title="Loading checks"></span>
          {:else if checksError || checks === null}
            <span class="warn">Unavailable</span>
            <button class="btn small" onclick={() => probeTick++}>Retry</button>
          {:else if checks.checks.length === 0}
            <span class="dim">No checks reported</span>
          {:else}
            <span class="checks">
              {#each checks.checks as c, i (i)}
                {@const ic = checkIcon(c.state)}
                {#if c.url}
                  <button
                    class="check link {ic.cls}"
                    title="{c.name} — {c.state}"
                    onclick={() => void openExternal(c.url)}
                  >
                    <span class="glyph"><Icon name={ic.icon} size={12} /></span>{c.name}
                  </button>
                {:else}
                  <span class="check {ic.cls}" title="{c.name} — {c.state}">
                    <span class="glyph"><Icon name={ic.icon} size={12} /></span>{c.name}
                  </span>
                {/if}
              {/each}
            </span>
          {/if}
        </span>
      </div>

      <div class="row-item">
        <span class="rlabel">Approvals</span>
        <span class="rvalue">
          {#if readinessLoading}<span class="spinner" role="img" aria-label="Loading approvals" title="Loading approvals"></span>
          {:else if readinessError || readiness === null}
            <span class="warn">Unavailable</span>
            <button class="btn small" onclick={() => probeTick++}>Retry</button>
          {:else}{readiness.approvals}{/if}
        </span>
      </div>

      <div class="row-item">
        <span class="rlabel">Mergeable</span>
        <span class="rvalue">
          {#if mergeable === true}<span class="ok">Yes</span>
          {:else if mergeable === false}<span class="bad">No — resolve the conflicts first</span>
          {:else}<span class="dim">Unknown</span>{/if}
        </span>
      </div>

      <div class="row-item">
        <span class="rlabel">Open blockers</span>
        <span class="rvalue">
          {#if readinessLoading}<span class="spinner" role="img" aria-label="Loading blockers" title="Loading blockers"></span>
          {:else if readiness?.review}
            <span class:bad={blockers > 0}>{blockers}</span>
            <span class="dim">of {readiness.review.unresolved_total} unresolved</span>
            <button class="linkbtn" onclick={openReview}>Open review</button>
          {:else if readinessError}
            <span class="warn">Unavailable</span>
          {:else}
            <span class="dim">No review run</span>
          {/if}
        </span>
      </div>

      {#if readiness && readiness.unpushed !== null}
        <div class="row-item">
          <span class="rlabel">Unpushed local commits</span>
          <span class="rvalue" class:bad={readiness.unpushed > 0}>{readiness.unpushed}</span>
        </div>
      {/if}

      {#if readiness}
        <div class="row-item">
          <span class="rlabel">Freshness</span>
          <span class="rvalue" class:bad={readiness.branch_freshness === 'behind'}>
            {FRESHNESS[readiness.branch_freshness] ?? readiness.branch_freshness}
          </span>
        </div>
      {/if}
    </div>

    <!-- merge options -->
    <div class="opts">
      <label class="opt">
        <span class="rlabel">Strategy</span>
        <select class="input" bind:value={strategy} style="width: 140px">
          {#each mergeStrategies(provider) as o (o.value)}<option value={o.value} title={o.hint}>{o.label}</option>{/each}
        </select>
      </label>
      <label class="check-opt">
        <input type="checkbox" bind:checked={deleteSource} />
        <span>Delete source branch after merge</span>
      </label>
      {#if reasons.length > 0}
        <label class="check-opt anyway">
          <input type="checkbox" bind:checked={mergeAnyway} />
          <span>Merge anyway</span>
        </label>
      {/if}
    </div>

    {#if error}
      <div class="err" use:announceShown={{ text: `Couldn’t merge. ${error}`, assertive: true }}><Icon name="warning" size={14} /><span>Couldn’t merge. {error}</span></div>
    {/if}
    <p class="who dim">
      Merges <span class="mono">{pr.source_branch}</span> into <span class="mono">{pr.target_branch}</span> on the provider.
      Everyone with access to the repository sees it, and it can’t be undone from Otto.
    </p>
  </div>

  {#snippet footer()}
    <div class="foot">
      {#if reasons.length > 0}
        <span class="reasons" class:muted={mergeAnyway}>{reasons.join(' · ')}</span>
      {/if}
      <span class="grow"></span>
      <button class="btn ghost" onclick={onclose} disabled={merging}>Cancel</button>
      <button class="btn primary" onclick={doMerge} disabled={!canMerge}>
        <Icon name="merge" size={12} />
        {merging ? 'Merging…' : 'Merge pull request'}
      </button>
    </div>
  {/snippet}
</Modal>

<style>
  .who {
    margin: 10px 0 0;
    font-size: var(--fs-xs);
  }
  .body {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .prline {
    display: flex;
    align-items: baseline;
    gap: 6px;
    font-size: var(--fs-m);
  }
  .ptitle {
    font-weight: 600;
    overflow-wrap: anywhere;
  }
  .flow {
    font-size: var(--fs-xs);
  }
  /* source→target separator mirrors in place under RTL. */
  .dir-arrow {
    display: inline-block;
  }
  :global([dir='rtl']) .dir-arrow {
    transform: scaleX(-1);
  }
  .rows {
    display: flex;
    flex-direction: column;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
  }
  .row-item {
    display: flex;
    align-items: baseline;
    gap: 10px;
    padding: 6px 10px;
    font-size: var(--fs-s);
  }
  .row-item + .row-item {
    border-top: 1px solid var(--border);
  }
  .rlabel {
    flex: 0 0 168px;
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .rvalue {
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    gap: 6px;
    min-width: 0;
  }
  .checks {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 10px;
  }
  .check {
    display: inline-flex;
    align-items: baseline;
    gap: 4px;
    font-size: var(--fs-xs);
    overflow-wrap: anywhere;
  }
  .check.link {
    background: none;
    border: none;
    padding: 0;
    color: inherit;
    cursor: pointer;
    text-decoration: underline;
    text-underline-offset: 2px;
  }
  .glyph {
    display: inline-flex;
    margin-inline-end: 2px;
  }
  .ok {
    color: var(--success);
  }
  .bad {
    color: var(--danger);
  }
  .warn {
    color: var(--warning);
  }
  .dim {
    color: var(--text-dim);
  }
  .linkbtn {
    background: none;
    border: none;
    padding: 0;
    color: var(--accent-text);
    cursor: pointer;
    font-size: var(--fs-xs);
    text-decoration: underline;
    text-underline-offset: 2px;
  }
  .opts {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .opt {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .check-opt {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .check-opt input {
    accent-color: var(--accent);
  }
  .anyway {
    color: var(--danger);
  }
  .err {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border-radius: var(--radius-m);
    background: var(--danger-soft);
    color: var(--danger);
    font-size: var(--fs-xs);
    line-height: 1.45;
  }
  .foot {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    min-width: 0;
  }
  .grow {
    flex: 1;
  }
  .reasons {
    font-size: var(--fs-xs);
    color: var(--danger);
    overflow-wrap: anywhere;
  }
  .reasons.muted {
    color: var(--text-dim);
  }
  .mono {
    font-family: var(--font-mono);
  }

  /* ── Mobile + tablet (≤1024px): the label column stops eating the value and
     the footer stacks so the reason text never squeezes the buttons out. */
  @media (max-width: 1024px) {
    .row-item {
      flex-direction: column;
      gap: 2px;
    }
    .rlabel {
      flex: none;
    }
    .check-opt input {
      width: 18px;
      height: 18px;
    }
    .foot {
      flex-wrap: wrap;
    }
    .reasons {
      flex: 1 0 100%;
    }
  }
</style>
