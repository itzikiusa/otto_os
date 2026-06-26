<script lang="ts">
  // Run with Otto — the "one button" flow. Turn any source (a Jira story, a
  // GitHub issue/PR, a Slack thread, a finding, a failing test) into a reviewed,
  // evidence-backed PR draft. The page has three areas: the launcher (the one
  // button), the runs list, and the open run's detail panel.
  import { ws } from '../../lib/stores/workspace.svelte';
  import { runWithOtto } from '../../lib/stores/runWithOtto.svelte';
  import ProofStatusChip from '../../lib/components/ProofStatusChip.svelte';
  import RunLauncher from './RunLauncher.svelte';
  import RunDetail from './RunDetail.svelte';
  import type { OttoRun } from '../../lib/api/types';
  import { humanize, statusTone } from './runStatus';

  // Load the workspace's runs whenever the active workspace changes. This effect
  // reads ONLY ws.currentId (not the run list it loads), so it never self-loops.
  $effect(() => {
    const id = ws.currentId;
    if (id) void runWithOtto.loadList(id);
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

<div class="rwo">
  <header class="head">
    <div>
      <h1>Run with Otto</h1>
      <p class="sub">
        Turn any source — a Jira story, a GitHub issue/PR, a Slack thread, a finding, a failing
        test — into a reviewed, evidence-backed PR draft. One button.
      </p>
    </div>
  </header>

  {#if ws.currentId}
    <RunLauncher wsId={ws.currentId} {onLaunched} />
  {/if}

  <div class="body" class:has-detail={openRun}>
    <section class="list-col">
      {#if runWithOtto.loadingList && list.length === 0}
        <div class="muted">Loading runs…</div>
      {:else if list.length === 0}
        <div class="empty">No runs yet. Paste a source above and press Run with Otto.</div>
      {:else}
        <ul class="runs">
          {#each list as r (r.id)}
            <li>
              <button
                class="run"
                class:selected={openRun?.id === r.id}
                onclick={() => selectRun(r)}
              >
                <div class="run-top">
                  <span class="badge src-{r.source_kind}">{r.source_kind}</span>
                  <span class="run-title">{r.title || r.source_ref}</span>
                  <span class="pill {statusTone(r.status)}">{humanize(r.status)}</span>
                </div>
                <div class="run-meta">
                  {#if r.proof_pack_id && r.proof_status}
                    <ProofStatusChip status={r.proof_status} risk={r.risk_score} compact />
                  {/if}
                  <span class="findings">
                    {r.findings_total} findings
                    {#if r.findings_blocking > 0}
                      <span class="blocking">{r.findings_blocking} blocking</span>
                    {/if}
                  </span>
                  <span class="when">{r.updated_at}</span>
                </div>
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </section>

    {#if openRun}
      <section class="detail-col">
        <RunDetail run={openRun} onClose={() => runWithOtto.closeDetail()} />
      </section>
    {/if}
  </div>
</div>

<style>
  .rwo { padding: 1rem 1.25rem; max-width: 1100px; margin: 0 auto; }
  .head { margin-bottom: 0.75rem; }
  .head h1 { margin: 0; font-size: 1.25rem; color: var(--text); }
  .sub { margin: 0.25rem 0 0; color: var(--text-dim); font-size: 0.85rem; max-width: 72ch; }
  .body { display: grid; grid-template-columns: 1fr; gap: 1rem; align-items: start; }
  .body.has-detail { grid-template-columns: minmax(0, 1fr) minmax(0, 1.1fr); }
  @media (max-width: 860px) {
    .body.has-detail { grid-template-columns: 1fr; }
  }
  .empty, .muted { color: var(--text-dim); padding: 0.75rem 0; font-size: 0.9rem; }
  .runs { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 0.5rem; }
  .run {
    width: 100%; text-align: left; cursor: pointer;
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
    margin-left: 0.3rem; font-size: 0.7rem; padding: 0.02rem 0.4rem; border-radius: 999px;
    background: color-mix(in srgb, var(--status-exited) 16%, transparent); color: var(--status-exited);
  }
  .when { margin-left: auto; font-variant-numeric: tabular-nums; }
  .badge {
    font-size: 0.7rem; padding: 0.05rem 0.45rem; border-radius: 999px;
    border: 1px solid var(--border); color: var(--text-dim); text-transform: capitalize;
  }
  .pill {
    font-size: 0.7rem; padding: 0.05rem 0.5rem; border-radius: 999px;
    border: 1px solid transparent; text-transform: capitalize; white-space: nowrap;
  }
  .pill.ok { background: color-mix(in srgb, var(--status-working) 16%, transparent); color: var(--status-working); }
  .pill.bad { background: color-mix(in srgb, var(--status-exited) 16%, transparent); color: var(--status-exited); }
  .pill.warn { background: color-mix(in srgb, var(--status-warn) 18%, transparent); color: var(--status-warn); }
  .pill.active { background: color-mix(in srgb, var(--accent) 16%, transparent); color: var(--accent); }
  .pill.dim { background: color-mix(in srgb, var(--text-dim) 14%, transparent); color: var(--text-dim); }
</style>
