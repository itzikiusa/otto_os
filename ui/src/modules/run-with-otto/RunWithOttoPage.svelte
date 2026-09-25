<script lang="ts">
  // Run with Otto — the "one button" flow. Turn any source (a Jira story, a
  // GitHub issue/PR, a Slack thread, a finding, a failing test) into a reviewed,
  // evidence-backed PR draft. The page has three areas: the launcher (the one
  // button), the runs list, and the open run's detail panel.
  import { untrack } from 'svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
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
  import { humanize, sourceColor, sourceLabel, statusTone } from './runStatus';

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

  function onLaunched(run: OttoRun): void {
    void runWithOtto.open(run.id);
  }

  function selectRun(run: OttoRun): void {
    void runWithOtto.open(run.id);
  }
</script>

<div class="rwo-page">
<PageHeader
  title="Run with Otto"
  subtitle="Turn any source — a Jira story, a GitHub issue/PR, a Slack thread, a finding, a failing test — into a reviewed, evidence-backed PR draft. One button."
/>
<PageBody>
<div class="rwo">

  {#if ws.currentId}
    <RunLauncher wsId={ws.currentId} {onLaunched} />
  {/if}

  <div class="body" class:has-detail={openRun}>
    <section class="list-col">
      <LoadState
        what="runs"
        loading={runWithOtto.loadingList}
        error={runWithOtto.listError}
        empty={list.length === 0}
        onretry={() => ws.currentId && void runWithOtto.loadList(ws.currentId)}
      >
        {#snippet emptyView()}
          <EmptyState icon="play" title="No runs yet" body="Paste a source above and press Run with Otto." />
        {/snippet}
        <ul class="runs">
          {#each list as r (r.id)}
            <li>
              <button
                class="run"
                class:selected={openRun?.id === r.id}
                onclick={() => selectRun(r)}
              >
                <div class="run-top">
                  <span class="badge src-badge" style="--src: {sourceColor(r.source_kind)}">{sourceLabel(r.source_kind)}</span>
                  <span class="run-title" title={r.title || r.source_ref}>{r.title || r.source_ref}</span>
                  <span class="pill {statusTone(r.status)}">{humanize(r.status)}</span>
                </div>
                <div class="run-meta">
                  <RunStageRail status={r.status} mini />
                  {#if r.proof_pack_id && r.proof_status}
                    <ProofStatusChip status={r.proof_status} risk={r.risk_score} compact />
                  {/if}
                  <span class="findings">
                    {r.findings_total} findings
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
      <section class="detail-col">
        <!-- keyed: the reject draft / action error belong to ONE run -->
        {#key openRun.id}
          <RunDetail run={openRun} onClose={() => runWithOtto.closeDetail()} />
        {/key}
      </section>
    {/if}
  </div>
</div>
</PageBody>
</div>

<style>
  .rwo-page { display: flex; flex-direction: column; height: 100%; min-height: 0; }
  .body { display: grid; grid-template-columns: 1fr; gap: 1rem; align-items: start; }
  .body.has-detail { grid-template-columns: minmax(0, 1fr) minmax(0, 1.1fr); }
  @media (max-width: 860px) {
    .body.has-detail { grid-template-columns: 1fr; }
  }
  .runs { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 0.5rem; }
  .run {
    width: 100%; text-align: start; cursor: pointer;
    border: 1px solid var(--border); background: var(--surface); color: var(--text);
    border-radius: var(--radius-m); padding: 0.6rem 0.75rem;
    display: flex; flex-direction: column; gap: 0.35rem; font: inherit;
  }
  .run:hover { border-color: color-mix(in srgb, var(--accent) 45%, var(--border)); }
  .run.selected { border-color: var(--accent); background: color-mix(in srgb, var(--accent) 7%, var(--surface)); }
  .run-top { display: flex; align-items: center; gap: 0.5rem; flex-wrap: wrap; }
  .run-title { font-size: 0.95rem; font-weight: 600; flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .run-meta { display: flex; align-items: center; gap: 0.55rem; flex-wrap: wrap; font-size: 0.8rem; color: var(--text-dim); }
  .findings { font-variant-numeric: tabular-nums; }
  .blocking {
    margin-inline-start: 0.3rem; font-size: 0.7rem; padding: 0.02rem 0.4rem; border-radius: 999px;
    background: color-mix(in srgb, var(--status-exited) 16%, transparent); color: var(--status-exited);
  }
  .when { margin-inline-start: auto; font-variant-numeric: tabular-nums; }
  .badge {
    font-size: 0.7rem; padding: 0.05rem 0.45rem; border-radius: 999px;
    border: 1px solid var(--border); color: var(--text-dim); text-transform: capitalize;
  }
  .src-badge {
    color: var(--src);
    border-color: color-mix(in srgb, var(--src) 40%, var(--border));
    background: color-mix(in srgb, var(--src) 10%, transparent);
  }
  .agent { font-size: 0.74rem; font-family: var(--font-mono); }
  .pill {
    font-size: 0.7rem; padding: 0.05rem 0.5rem; border-radius: 999px;
    border: 1px solid transparent; text-transform: capitalize; white-space: nowrap;
  }
  .pill.ok { background: color-mix(in srgb, var(--status-working) 16%, transparent); color: var(--status-working); }
  .pill.bad { background: color-mix(in srgb, var(--status-exited) 16%, transparent); color: var(--status-exited); }
  .pill.warn { background: color-mix(in srgb, var(--status-warn) 18%, transparent); color: var(--status-warn); }
  .pill.active { background: color-mix(in srgb, var(--accent) 16%, transparent); color: var(--accent-text); }
  .pill.dim { background: color-mix(in srgb, var(--text-dim) 14%, transparent); color: var(--text-dim); }
</style>
