<script lang="ts">
  // Merge confirmation for a hosted PR. Nothing merges until the user clicks
  // Merge here — and the modal first shows everything that argues against it:
  // per-check CI, approvals, mergeability, open blocker findings, and the two
  // facts only the local checkout knows (unpushed commits, branch freshness).
  // Blocking reasons disable Merge until "Merge anyway" is ticked.
  import { api, ApiError } from '../../lib/api/client';
  import type { MergeStrategy, PrChecksResp, PrDetail, PrReadiness } from '../../lib/api/types';
  import { router } from '../../lib/router.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { openExternal } from '../../lib/external';
  import Modal from '../../lib/components/Modal.svelte';
  import Icon from '../../lib/components/Icon.svelte';

  interface Props {
    repoId: string;
    number: number;
    pr: PrDetail;
    onclose: () => void;
    onmerged: () => void;
  }
  let { repoId, number, pr, onclose, onmerged }: Props = $props();

  let strategy: MergeStrategy = $state('merge');
  let deleteSource = $state(false);
  let mergeAnyway = $state(false);
  let merging = $state(false);
  let error = $state<string | null>(null);

  // Both probes are best-effort: a provider that answers neither still lets the
  // user merge, the rows just read "unavailable" instead of lying with zeros.
  let checks = $state<PrChecksResp | null>(null);
  let checksLoading = $state(true);
  let readiness = $state<PrReadiness | null>(null);
  let readinessLoading = $state(true);

  $effect(() => {
    const id = repoId;
    const n = number;
    checksLoading = true;
    void api
      .get<PrChecksResp>(`/repos/${id}/prs/${n}/checks`)
      .then((r) => (checks = r))
      .catch(() => (checks = null))
      .finally(() => (checksLoading = false));
  });

  $effect(() => {
    const id = repoId;
    const n = number;
    readinessLoading = true;
    void api
      .get<PrReadiness>(`/repos/${id}/prs/${n}/readiness`)
      .then((r) => (readiness = r))
      .catch(() => (readiness = null))
      .finally(() => (readinessLoading = false));
  });

  const blockers = $derived(readiness?.review?.unresolved_blocker_count ?? 0);
  const mergeable = $derived(readiness?.mergeable ?? pr.mergeable ?? null);

  // What argues against merging. Empty ⇒ Merge is enabled outright.
  const reasons = $derived(
    [
      checks?.ci.state === 'failure' ? 'CI failing' : null,
      mergeable === false ? 'Not mergeable' : null,
      blockers > 0 ? `${blockers} blocker finding${blockers === 1 ? '' : 's'}` : null,
    ].filter((r): r is string => r !== null),
  );
  const loading = $derived(checksLoading || readinessLoading);
  const canMerge = $derived(!merging && !loading && (reasons.length === 0 || mergeAnyway));

  const FRESHNESS: Record<string, string> = {
    fresh: 'Up to date with the target branch',
    behind: 'Behind the target branch',
    unknown: 'Unknown (branch not checked out locally)',
  };

  function checkIcon(state: string): { glyph: string; cls: string } {
    if (state === 'success') return { glyph: '✓', cls: 'ok' };
    if (state === 'failure') return { glyph: '✗', cls: 'bad' };
    if (state === 'skipped' || state === 'neutral') return { glyph: '–', cls: 'dim' };
    return { glyph: '●', cls: 'dim' };
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
    merging = true;
    error = null;
    try {
      await api.post(`/repos/${repoId}/prs/${number}/merge`, {
        strategy,
        delete_source_branch: deleteSource,
      });
      toasts.success('PR merged', `#${number}`);
      onmerged();
    } catch (e) {
      // Keep the provider's own words — "Required status check failing" and
      // friends are the actionable text.
      error = e instanceof ApiError || e instanceof Error ? e.message : String(e);
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
    </div>

    <!-- readiness rows -->
    <div class="rows">
      <div class="row-item">
        <span class="rlabel">CI</span>
        <span class="rvalue">
          {#if checksLoading}
            <span class="dim">Loading…</span>
          {:else if checks === null}
            <span class="dim">unavailable</span>
          {:else if checks.checks.length === 0}
            <span class="dim">no checks reported</span>
          {:else}
            <span class="checks">
              {#each checks.checks as c (c.name)}
                {@const ic = checkIcon(c.state)}
                {#if c.url}
                  <button
                    class="check link {ic.cls}"
                    title="{c.name} — {c.state}"
                    onclick={() => void openExternal(c.url)}
                  >
                    <span class="glyph">{ic.glyph}</span>{c.name}
                  </button>
                {:else}
                  <span class="check {ic.cls}" title="{c.name} — {c.state}">
                    <span class="glyph">{ic.glyph}</span>{c.name}
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
          {#if readinessLoading}<span class="dim">Loading…</span>
          {:else if readiness === null}<span class="dim">unavailable</span>
          {:else}{readiness.approvals}{/if}
        </span>
      </div>

      <div class="row-item">
        <span class="rlabel">Mergeable</span>
        <span class="rvalue">
          {#if mergeable === true}<span class="ok">yes</span>
          {:else if mergeable === false}<span class="bad">no</span>
          {:else}<span class="dim">unknown</span>{/if}
        </span>
      </div>

      <div class="row-item">
        <span class="rlabel">Open blockers</span>
        <span class="rvalue">
          {#if readinessLoading}<span class="dim">Loading…</span>
          {:else if readiness?.review}
            <span class:bad={blockers > 0}>{blockers}</span>
            <span class="dim">of {readiness.review.unresolved_total} unresolved</span>
            <button class="linkbtn" onclick={openReview}>Open review</button>
          {:else}
            <span class="dim">no review run</span>
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
        <select class="input" bind:value={strategy} style="width: 110px">
          <option value="merge">merge</option>
          <option value="squash">squash</option>
          <option value="rebase">rebase</option>
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
      <div class="err"><Icon name="x" size={13} /><span>{error}</span></div>
    {/if}
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
        {merging ? 'Merging…' : 'Merge'}
      </button>
    </div>
  {/snippet}
</Modal>

<style>
  .body {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .prline {
    display: flex;
    align-items: baseline;
    gap: 6px;
    font-size: 13px;
  }
  .ptitle {
    font-weight: 600;
    overflow-wrap: anywhere;
  }
  .flow {
    font-size: 11.5px;
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
    padding: 7px 10px;
    font-size: 12px;
  }
  .row-item + .row-item {
    border-top: 1px solid var(--border);
  }
  .rlabel {
    flex: 0 0 168px;
    color: var(--text-dim);
    font-size: 11.5px;
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
    font-size: 11.5px;
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
    font-weight: 700;
  }
  .ok {
    color: var(--status-idle, #34c759);
  }
  .bad {
    color: var(--status-exited);
  }
  .dim {
    color: var(--text-dim);
  }
  .linkbtn {
    background: none;
    border: none;
    padding: 0;
    color: var(--accent);
    cursor: pointer;
    font-size: 11.5px;
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
    font-size: 12px;
    cursor: pointer;
  }
  .check-opt input {
    accent-color: var(--accent);
  }
  .anyway {
    color: var(--status-exited);
  }
  .err {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border-radius: var(--radius-m);
    background: color-mix(in srgb, var(--status-exited) 14%, transparent);
    color: var(--status-exited);
    font-size: 11.5px;
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
    font-size: 11.5px;
    color: var(--status-exited);
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
      gap: 3px;
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
