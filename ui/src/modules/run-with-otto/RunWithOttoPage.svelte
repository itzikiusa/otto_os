<script lang="ts">
  // Run with Otto — the "one button" flow. Turn any source (a Jira story, a
  // GitHub issue/PR, a Slack thread, a finding, a failing test) into a reviewed,
  // evidence-backed PR draft. The page has three areas: the launcher (the one
  // button), the runs list, and the open run's detail panel.
  import { untrack } from 'svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { ui } from '../../lib/stores/ui.svelte';
  import { runWithOtto } from '../../lib/stores/runWithOtto.svelte';
  import ProofStatusChip from '../../lib/components/ProofStatusChip.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import RelTime from '../../lib/components/RelTime.svelte';
  import RunLauncher from './RunLauncher.svelte';
  import RunDetail from './RunDetail.svelte';
  import RunStageRail from './RunStageRail.svelte';
  import type { OttoRun } from '../../lib/api/types';
  import StatusBadge from '../../lib/components/StatusBadge.svelte';
  import { initialSelection, rememberSelection } from '../../lib/lastSelection';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { runStatusInfo, sourceLabel } from './runStatus';

  // Load the workspace's runs whenever the active workspace changes. This effect
  // reads ONLY ws.currentId (not the run list it loads), so it never self-loops.
  // An open run from ANOTHER workspace is closed — its cached record would
  // otherwise keep showing next to this workspace's list.
  $effect(() => {
    const id = ws.currentId;
    if (!id) return;
    untrack(() => {
      const open = runWithOtto.openRun;
      if (open && open.workspace_id !== id) runWithOtto.closeDetail();
    });
    void runWithOtto.loadList(id);
  });

  const list = $derived(runWithOtto.list);
  const openRun = $derived(runWithOtto.openRun);

  let detailEl: HTMLElement | undefined = $state();

  // A list/detail page opens ON a run, never on a list with nothing beside it
  // (layout.md): once the list is in, open the remembered run (else the
  // newest). Desktop only — on narrow widths the detail stacks under the list
  // and auto-opening would push the list off-screen. Runs once per workspace
  // list, so closing the panel sticks.
  let autoOpenedFor = '';
  $effect(() => {
    const id = ws.currentId;
    const rows = list;
    if (!id || runWithOtto.loadingList || rows.length === 0 || autoOpenedFor === id) return;
    autoOpenedFor = id;
    untrack(() => {
      if (runWithOtto.openRun || !viewport.isDesktop) return;
      const pick = initialSelection('run-with-otto', rows, (r) => r.id);
      if (pick) void runWithOtto.open(pick);
    });
  });

  function show(run: OttoRun): void {
    rememberSelection('run-with-otto', run.id);
    void runWithOtto.open(run.id);
    // Stacked layout: bring the freshly opened detail into view.
    if (!viewport.isDesktop) {
      requestAnimationFrame(() => detailEl?.scrollIntoView({ block: 'start', behavior: 'smooth' }));
    }
  }

  function onLaunched(run: OttoRun): void {
    show(run);
  }

  function selectRun(run: OttoRun): void {
    show(run);
  }
</script>

<div class="rwo-page">
<PageHeader
  title="Run with Otto"
  subtitle="Turn a Jira story, GitHub issue or PR, finding or failing test into a reviewed PR draft"
/>
<PageBody>
<div class="rwo">

  {#if ws.currentId}
    <RunLauncher wsId={ws.currentId} {onLaunched} />

  <div class="body-wrap">
  <div class="body" class:has-detail={openRun}>
    <section class="list-col" aria-label="Runs">
      <LoadState
        what="runs"
        loading={runWithOtto.loadingList}
        error={runWithOtto.listError}
        empty={list.length === 0}
        onretry={() => ws.currentId && void runWithOtto.loadList(ws.currentId)}
      >
        {#snippet emptyView()}
          <EmptyState icon="play" title="No runs yet" body="Paste a source above and press Run with Otto. Each run shows up here with its stage, proof and findings." />
        {/snippet}
        <h2 class="list-h">Runs <span class="count">{list.length}</span></h2>
        <ul class="runs">
          {#each list as r (r.id)}
            <li>
              <button
                class="run"
                class:selected={openRun?.id === r.id}
                aria-current={openRun?.id === r.id ? 'true' : undefined}
                onclick={() => selectRun(r)}
              >
                <div class="run-top">
                  <span class="chip">{sourceLabel(r.source_kind)}</span>
                  <span class="run-title" title={r.title || r.source_ref}>{r.title || r.source_ref}</span>
                  <StatusBadge status={runStatusInfo(r.status)} />
                </div>
                <div class="run-meta">
                  <RunStageRail status={r.status} mini />
                  {#if r.proof_pack_id && r.proof_status}
                    <ProofStatusChip status={r.proof_status} risk={r.risk_score} compact />
                  {/if}
                  <span class="findings">
                    {r.findings_total} {r.findings_total === 1 ? 'finding' : 'findings'}
                    {#if r.findings_blocking > 0}
                      <span class="blocking">{r.findings_blocking} blocking</span>
                    {/if}
                  </span>
                  <span class="agent mono" title="Executing agent (provider · model)">
                    {r.provider}{r.model ? ` · ${r.model}` : ''}
                  </span>
                  <span class="when"><RelTime iso={r.updated_at} /></span>
                </div>
              </button>
            </li>
          {/each}
        </ul>
      </LoadState>
    </section>

    {#if openRun}
      <section class="detail-col" bind:this={detailEl} aria-label="Run detail">
        <!-- keyed: the reject draft / action error belong to ONE run -->
        {#key openRun.id}
          <RunDetail run={openRun} onClose={() => runWithOtto.closeDetail()} />
        {/key}
      </section>
    {/if}
  </div>
  </div>
  {:else}
    <EmptyState
      variant="page"
      icon="folder"
      title="Add a workspace to get started"
      body="Runs belong to a workspace. Add your project folder, then choose the source you want Otto to work on."
      actionLabel="Add workspace"
      actionIcon="plus"
      onaction={() => (ui.newWorkspaceOpen = true)}
    />
  {/if}
</div>
</PageBody>
</div>

<style>
  .rwo-page { display: flex; flex-direction: column; height: 100%; min-height: 0; }
  /* Container query, not a viewport one: the page also renders in the narrow
     side-by-side pane, where the viewport is wide but the column is not. */
  .body-wrap { container-type: inline-size; }
  .body { display: grid; grid-template-columns: 1fr; gap: 16px; align-items: start; }
  .body.has-detail { grid-template-columns: minmax(0, 1fr) minmax(0, 1.1fr); }
  @container (max-width: 860px) {
    .body.has-detail { grid-template-columns: 1fr; }
  }
  .detail-col { min-width: 0; }
  .list-h {
    margin: 0 0 8px;
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .count { font-variant-numeric: tabular-nums; font-weight: 500; }
  .runs { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
  .run {
    width: 100%; text-align: start; cursor: pointer;
    border: 1px solid var(--border); background: var(--surface); color: var(--text);
    border-radius: var(--radius-m); padding: 10px 12px;
    display: flex; flex-direction: column; gap: 6px; font: inherit;
  }
  .run:hover { border-color: color-mix(in srgb, var(--accent) 45%, var(--border)); }
  .run.selected { border-color: var(--accent); background: var(--accent-soft); }
  .run-top { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .run-title {
    font-size: var(--fs-m); font-weight: 600; flex: 1; min-width: 0;
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  .run-meta { display: flex; align-items: center; gap: 4px 10px; flex-wrap: wrap; font-size: var(--fs-s); color: var(--text-dim); }
  .findings { font-variant-numeric: tabular-nums; }
  .blocking {
    margin-inline-start: 4px; font-size: var(--fs-xs); padding: 0 6px; border-radius: 999px;
    background: var(--danger-soft); color: var(--danger);
  }
  .when { margin-inline-start: auto; font-variant-numeric: tabular-nums; }
  .agent {
    font-size: var(--fs-xs); font-family: var(--font-mono);
    max-width: 16rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
</style>
