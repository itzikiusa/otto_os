<script lang="ts">
  // The top of a finished PR review: the verdict line (verdict, blockers and
  // the one-line summary the daemon computes from the tracked findings) and
  // the merge-readiness card (CI, approvals, open findings, mergeable) from
  // GET /reviews/{id}/merge-readiness.
  import type { MergeReadiness, Review } from '../../lib/api/types';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { verdictLabel } from '../../lib/labels';
  import { plural } from '../../lib/plural';

  interface Props {
    review: Review;
    readiness: MergeReadiness | null;
    loading: boolean;
    error: string | null;
    onretry: () => void;
  }
  let { review, readiness, loading, error, onretry }: Props = $props();

  const blockers = $derived(review.blocker_count ?? 0);
  const ready = $derived(review.status === 'done' && blockers === 0);
  const CI_TEXT: Record<string, string> = { none: 'No CI checks', pending: 'CI pending', success: 'CI passed', failure: 'CI failed' };
</script>

{#if review.blocker_count != null || review.verdict != null}
  <div class="mg-gate" class:mg-ok={ready} class:mg-blocked={!ready} data-testid="review-merge-gate">
    {#if ready}
      <Icon name="check" size={13} /> Merge-ready — no critical or high findings open
    {:else}
      <Icon name="zap" size={13} /> {plural(blockers, 'blocker')} before merge
    {/if}
    {#if review.verdict}
      <span class="mg-verdict">Verdict: {verdictLabel(review.verdict)}</span>
    {/if}
    {#if review.summary_md}
      <span class="mg-summary">{review.summary_md}</span>
    {/if}
  </div>
{/if}

{#if error}
  <LoadState what="merge readiness" {error} empty={readiness === null} variant="compact" {onretry} />
{/if}
{#if readiness !== null || loading}
  <div class="mg-readiness card">
    <span class="mg-label">Merge readiness</span>
    {#if loading}
      <span class="dim mg-chip">Checking merge readiness…</span>
    {:else if readiness !== null}
      {@const ci = readiness.ci_status ?? 'none'}
      <span class="chip mg-chip mg-ci-{ci}" title="CI: {ci}">{CI_TEXT[ci] ?? `CI ${ci}`}</span>
      {@const approvals = readiness.approvals ?? 0}
      <span class="chip mg-chip" class:ok={approvals > 0} class:dim={approvals === 0}>{plural(approvals, 'approval')}</span>
      {#if readiness.unresolved_total > 0}
        <span class="chip mg-chip mg-bad">{plural(readiness.unresolved_total, 'open finding')}</span>
      {:else}
        <span class="chip ok mg-chip">No open findings</span>
      {/if}
      {#if readiness.mergeable === true}
        <span class="chip ok mg-chip">Mergeable</span>
      {:else if readiness.mergeable === false}
        <span class="chip mg-chip mg-conflict">Conflicts</span>
      {/if}
    {/if}
  </div>
{/if}

<style>
  .mg-gate {
    display: flex; align-items: center; gap: 8px; flex-wrap: wrap;
    padding: 6px 10px; margin: 0 0 8px; border-radius: var(--radius-s);
    font-size: var(--fs-xs); font-weight: 500;
  }
  .mg-ok { background: var(--success-soft); border: 1px solid color-mix(in srgb, var(--success) 40%, var(--border)); color: var(--success); }
  .mg-blocked { background: var(--danger-soft); border: 1px solid color-mix(in srgb, var(--danger) 35%, var(--border)); color: var(--danger); }
  .mg-verdict { margin-inline-start: auto; font-weight: 400; opacity: 0.8; }
  .mg-summary { flex-basis: 100%; font-weight: 400; color: var(--text); }
  .mg-readiness { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; padding: 8px 12px; margin: 0 0 8px; }
  .mg-label { font-size: var(--fs-xs); font-weight: 600; color: var(--text-dim); white-space: nowrap; }
  .mg-chip { font-size: var(--fs-xs); padding: 2px 6px; }
  .mg-bad { background: var(--danger-soft); color: var(--danger); }
  .mg-conflict { color: var(--danger); }
  .mg-ci-success { background: var(--success-soft); color: var(--success); }
  .mg-ci-failure { background: var(--danger-soft); color: var(--danger); }
  .mg-ci-pending { background: var(--warning-soft); color: var(--warning); }
  .mg-ci-none { background: var(--surface-2); color: var(--text-dim); }
  .dim { color: var(--text-dim); }
</style>
