<script lang="ts">
  // The PR Summary's details column: reviewers, CI checks, labels, facts.
  import type { PrChecksResp, PrCommit, PrDetail } from '../../lib/api/types';
  import Icon from '../../lib/components/Icon.svelte';
  import { openExternal } from '../../lib/external';
  import { rel } from '../../lib/stores/now.svelte';
  import PrAvatar from './PrAvatar.svelte';
  import PrMergeChecks from './PrMergeChecks.svelte';
  import PrReviewersEdit from './PrReviewersEdit.svelte';
  import { currentReviewers } from './pr-reviewers-edit';

  interface Props {
    pr: PrDetail;
    checks: PrChecksResp | null;
    commits: PrCommit[] | null;
    providerName: string;
    /** With these, an open PR's reviewers can be edited (PrReviewersEdit). */
    repoId?: string;
    /** The confirm's "Where" line. */
    where?: string;
    onreviewerschanged?: () => void;
  }
  let { pr, checks, commits, providerName, repoId, where, onreviewerschanged }: Props = $props();
  let editing = $state(false);
  const canEdit = $derived(!!repoId && !!onreviewerschanged && pr.state === 'open');
  const glyph = (s: string) => (s === 'success' ? 'check' : s === 'failure' ? 'x' : s === 'pending' ? 'clock' : 'dot') as 'check' | 'x' | 'clock' | 'dot';
</script>

<aside class="side" aria-label="Pull request details">
  <section>
    <h3 class="h h-row">Reviewers
      {#if canEdit}
        <button class="sc-edit" data-testid="pr-reviewers-edit" aria-label="Edit reviewers" title="Add or remove reviewers (asks first)"
          onclick={() => (editing = true)}><Icon name="edit" size={12} /></button>
      {/if}
    </h3>
    {#each pr.reviewers as r, i (i)}
      <div class="sc-row">
        <PrAvatar name={r.name} url={r.avatar_url} />
        <span class="ell">{r.name}</span>
        <span class="grow"></span>
        {#if r.approved}
          <span class="ok" title="Approved{r.reviewed_at ? ` ${rel(r.reviewed_at)}` : ''}"><Icon name="check" size={12} /> Approved</span>
        {:else if r.changes_requested}
          <span class="warn" title="Changes requested{r.reviewed_at ? ` ${rel(r.reviewed_at)}` : ''}"><Icon name="warning" size={12} /> Changes requested</span>
        {:else}
          <span class="dim">Pending</span>
        {/if}
      </div>
    {:else}
      {#each pr.approved_by as name, i (i)}
        <div class="sc-row"><PrAvatar {name} /><span class="ell">{name}</span><span class="grow"></span><span class="ok"><Icon name="check" size={12} /> Approved</span></div>
      {:else}
        <p class="dim none">No reviewers yet.</p>
      {/each}
    {/each}
  </section>
  {#if checks && checks.checks.length > 0}
    <section>
      <h3 class="h">Checks <span class="dim">{checks.ci.passed}/{checks.ci.total}</span></h3>
      {#each checks.checks as c, i (i)}
        <div class="sc-row">
          <span class="ck ck-{c.state}" aria-hidden="true"><Icon name={glyph(c.state)} size={12} /></span>
          {#if c.url}
            <button class="sc-link ell" onclick={() => openExternal(c.url)} title="{c.name}: {c.state}">{c.name}</button>
          {:else}
            <span class="ell" title="{c.name}: {c.state}">{c.name}</span>
          {/if}
        </div>
      {/each}
    </section>
  {/if}
  <PrMergeChecks {pr} />
  {#if (pr.labels ?? []).length > 0}
    <section>
      <h3 class="h">Labels</h3>
      <div class="labels">{#each pr.labels ?? [] as l (l)}<span class="chip">{l}</span>{/each}</div>
    </section>
  {/if}
  <section>
    <h3 class="h">Details</h3>
    <dl class="dl">
      <dt>Provider</dt><dd>{providerName}</dd>
      {#if commits}<dt>Commits</dt><dd>{commits.length}</dd>{/if}
      {#if pr.head_sha}<dt>Head</dt><dd class="mono">{pr.head_sha.slice(0, 8)}</dd>{/if}
      {#if pr.created_at}<dt>Opened</dt><dd title={new Date(pr.created_at).toLocaleString()}>{rel(pr.created_at)}</dd>{/if}
    </dl>
  </section>
</aside>

{#if editing && repoId}
  <PrReviewersEdit {repoId} number={pr.number} current={currentReviewers(pr)} where={where ?? `PR #${pr.number}`} {providerName}
    onclose={() => (editing = false)} onsaved={() => { editing = false; onreviewerschanged?.(); }} />
{/if}

<style>
  .side { display: flex; flex-direction: column; gap: 18px; font-size: var(--fs-s); min-width: 0; }
  .h {
    margin: 0 0 8px;
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--text-dim);
  }
  .h-row { display: flex; align-items: center; justify-content: space-between; gap: 6px; }
  .sc-edit { background: none; border: none; padding: 4px; margin: -4px; color: var(--text-dim); cursor: pointer; display: inline-flex; border-radius: var(--radius-s); }
  .sc-edit:hover, .sc-edit:focus-visible { color: var(--text); }
  @media (max-width: 1024px) { .sc-edit { padding: 8px; margin: -8px; } }
  .sc-row { display: flex; align-items: center; gap: 8px; padding-block: 4px; min-width: 0; }
  .ell { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; text-align: start; }
  .ok { display: inline-flex; align-items: center; gap: 4px; color: var(--success); font-size: var(--fs-xs); white-space: nowrap; }
  .warn { display: inline-flex; align-items: center; gap: 4px; color: var(--warning); font-size: var(--fs-xs); white-space: nowrap; }
  .none { font-size: var(--fs-s); margin: 0; }
  .ck { display: inline-flex; flex: none; }
  .ck-success { color: var(--success); }
  .ck-failure { color: var(--danger); }
  .ck-pending { color: var(--warning); }
  .ck-skipped, .ck-neutral { color: var(--text-dim); }
  .labels { display: flex; flex-wrap: wrap; gap: 4px; }
  .dl { display: grid; grid-template-columns: auto 1fr; gap: 4px 12px; margin: 0; }
  .dl dt { color: var(--text-dim); }
  .dl dd { margin: 0; min-width: 0; overflow: hidden; text-overflow: ellipsis; }
  .sc-link { background: none; border: none; padding: 0; color: var(--accent-text); cursor: pointer; font: inherit; }
  .sc-link:hover { text-decoration: underline; }
</style>
