<script lang="ts">
  // The open run's detail panel (right-side drawer): the goal + source link, the
  // stage timeline, proof + findings, the approval gate, and the PR draft. Reads
  // the open run + its events straight from the store.
  import { runWithOtto } from '../../lib/stores/runWithOtto.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import ProofStatusChip from '../../lib/components/ProofStatusChip.svelte';
  import RunStageRail from './RunStageRail.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import RelTime from '../../lib/components/RelTime.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import type { OttoRun } from '../../lib/api/types';
  import { humanize, isTerminal, sourceColor, sourceLabel, statusTone } from './runStatus';

  interface Props {
    run: OttoRun;
    onClose: () => void;
  }
  let { run, onClose }: Props = $props();

  let busy = $state(false);
  let error = $state('');
  let rejectNote = $state('');
  let rejecting = $state(false);

  const events = $derived(runWithOtto.eventsByRun[run.id] ?? []);

  // Best-effort parse of the stored PR draft into title/description.
  const prDraft = $derived.by(() => {
    if (!run.pr_draft_json) return null;
    try {
      const j = JSON.parse(run.pr_draft_json) as Record<string, unknown>;
      const title = (j.title ?? j.pr_title ?? '') as string;
      const description = (j.description ?? j.body ?? j.description_md ?? '') as string;
      const source = (j.source_branch ?? '') as string;
      const target = (j.target_branch ?? '') as string;
      return { title, description, source, target };
    } catch {
      return null;
    }
  });

  async function approve(decision: 'approve' | 'reject'): Promise<void> {
    error = '';
    busy = true;
    try {
      await runWithOtto.approve(run.id, {
        decision,
        note: decision === 'reject' && rejectNote.trim() ? rejectNote.trim() : undefined,
      });
      rejecting = false;
      rejectNote = '';
    } catch (e) {
      error = e instanceof Error ? e.message : 'Decision failed';
    } finally {
      busy = false;
    }
  }

  async function cancel(): Promise<void> {
    if (!(await confirmer.ask('Cancel this run? The agent stops and the run can’t be resumed — you would launch a new one.', { title: 'Cancel run', confirmLabel: 'Cancel run', cancelLabel: 'Keep running' }))) return;
    error = '';
    busy = true;
    try {
      await runWithOtto.cancel(run.id);
    } catch (e) {
      error = e instanceof Error ? e.message : 'Cancel failed';
    } finally {
      busy = false;
    }
  }

  // Mirrors the daemon's open-PR gate (otto-core open_pr_block_reason) so the
  // button says WHY it can't open instead of failing with the raw gate text.
  const prBlock = $derived.by(() => {
    if (run.approval_decision !== 'approved') return 'Approve the run first';
    if (run.proof_status !== 'passed' && run.proof_status !== 'waived') return 'The proof pack must pass (or be waived) before a PR can be opened';
    if (!run.repo_id) return 'This run has no repository to open a PR in';
    return '';
  });

  /** The one outward-facing action: say where it goes and who sees it first. */
  async function openPr(): Promise<void> {
    const into = prDraft?.target || run.base_branch || 'the default branch';
    const from = prDraft?.source || run.branch || 'the run branch';
    const where = run.repo_path ? ` in ${run.repo_path}` : '';
    const ok = await confirmer.ask(
      `Push ${from} and open “${prDraft?.title || run.title}” as a pull request into ${into}${where}? It is created on the remote, where everyone with access to the repository can see it.`,
      { title: 'Open pull request', confirmLabel: 'Open PR', danger: false },
    );
    if (!ok) return;
    error = '';
    busy = true;
    try {
      await runWithOtto.openPr(run.id);
    } catch (e) {
      error = e instanceof Error ? e.message : 'Open PR failed';
    } finally {
      busy = false;
    }
  }
</script>

<aside class="detail">
  <header class="d-head">
    <div class="d-title">
      <span class="badge src-badge" style="--src: {sourceColor(run.source_kind)}">{sourceLabel(run.source_kind)}</span>
      <strong>{run.title || run.source_ref}</strong>
      <span class="pill {statusTone(run.status)}">{humanize(run.status)}</span>
    </div>
    <button class="btn small" onclick={onClose} aria-label="Close">Close</button>
  </header>

  {#if error}<div class="err" role="alert">{error}</div>{/if}
  {#if run.status === 'failed' && run.error}
    <div class="err" role="status"><strong>Run failed:</strong> {run.error}</div>
  {/if}

  <!-- where the run is on the pipeline right now -->
  <section class="block">
    <RunStageRail status={run.status} />
  </section>

  <section class="block">
    <div class="goal">{run.goal || '(no goal text)'}</div>
    <div class="src-row">
      {#if run.source_url}
        <a class="link" href={run.source_url} target="_blank" rel="noreferrer">{run.source_ref} <Icon name="external" size={11} /></a>
      {:else}
        <span class="muted">{run.source_ref}</span>
      {/if}
      <span class="dot">·</span>
      <span class="muted">{humanize(run.mode)}</span>
      {#if run.provider}
        <span class="dot">·</span>
        <span class="muted mono">{run.provider}{run.model ? ` · ${run.model}` : ''}</span>
      {/if}
    </div>
  </section>

  <!-- proof + findings -->
  <section class="block stats">
    {#if run.proof_pack_id && run.proof_status}
      <ProofStatusChip status={run.proof_status} risk={run.risk_score} />
    {/if}
    <span class="findings">
      <span class="fnum">{run.findings_total}</span> findings
      {#if run.findings_blocking > 0}
        <span class="blocking" title="blocking findings">{run.findings_blocking} blocking</span>
      {/if}
    </span>
    {#if run.branch}<span class="muted mono">{run.branch}</span>{/if}
  </section>

  <!-- stage timeline -->
  <section class="block">
    <h3 class="h">Stage timeline</h3>
    <LoadState
      what="the stage timeline"
      variant="compact"
      loading={!runWithOtto.eventsByRun[run.id] && !runWithOtto.eventsError[run.id]}
      error={runWithOtto.eventsError[run.id]}
      empty={events.length === 0}
      onretry={() => void runWithOtto.loadEvents(run.id)}
    >
      {#snippet emptyView()}<div class="muted">No stage events yet.</div>{/snippet}
      <ol class="timeline">
        {#each events as ev (ev.id)}
          <li class="tl-item">
            <span class="tl-dot {ev.status ? statusTone(ev.status) : 'dim'}"></span>
            <div class="tl-body">
              <div class="tl-top">
                <span class="tl-kind">{humanize(ev.kind)}</span>
                {#if ev.status}<span class="pill {statusTone(ev.status)} tiny">{humanize(ev.status)}</span>{/if}
                <span class="tl-when"><RelTime iso={ev.created_at} /></span>
              </div>
              {#if ev.message}<div class="tl-msg">{ev.message}</div>{/if}
            </div>
          </li>
        {/each}
      </ol>
    </LoadState>
  </section>

  <!-- approval gate -->
  {#if run.status === 'awaiting_approval'}
    <section class="block gate">
      <h3 class="h">Awaiting your approval</h3>
      {#if rejecting}
        <textarea bind:value={rejectNote} rows="2" placeholder="Optional reason for rejecting…"></textarea>
        <div class="actions">
          <button class="btn danger" disabled={busy} onclick={() => approve('reject')}>Confirm reject</button>
          <button class="btn" disabled={busy} onclick={() => { rejecting = false; rejectNote = ''; }}>Back</button>
        </div>
      {:else}
        <div class="actions">
          <button class="btn primary" disabled={busy} onclick={() => approve('approve')}>Approve</button>
          <button class="btn danger" disabled={busy} onclick={() => (rejecting = true)}>Reject</button>
        </div>
      {/if}
    </section>
  {/if}

  <!-- PR draft -->
  {#if prDraft}
    <section class="block pr">
      <h3 class="h">PR draft</h3>
      <div class="pr-title">{prDraft.title || '(untitled)'}</div>
      {#if prDraft.description}<pre class="pr-desc">{prDraft.description}</pre>{/if}
      <div class="actions">
        {#if run.pr_url}
          <a class="btn primary" href={run.pr_url} target="_blank" rel="noreferrer">View PR <Icon name="external" size={12} /></a>
        {:else}
          <button class="btn primary" disabled={busy || !!prBlock} title={prBlock || 'Push the branch and open this draft as a real pull request'} onclick={openPr}>Open PR</button>
          {#if prBlock}<span class="muted hint">{prBlock}.</span>{/if}
        {/if}
      </div>
    </section>
  {:else if run.pr_url}
    <section class="block pr">
      <h3 class="h">Pull request</h3>
      <a class="btn primary" href={run.pr_url} target="_blank" rel="noreferrer">View PR <Icon name="external" size={12} /></a>
    </section>
  {/if}

  {#if run.result_summary}
    <section class="block">
      <h3 class="h">Result</h3>
      <div class="muted summary">{run.result_summary}</div>
    </section>
  {/if}

  <!-- cancel (non-terminal only) -->
  {#if !isTerminal(run.status)}
    <section class="block">
      <button class="btn small danger" disabled={busy} onclick={cancel}>Cancel run</button>
    </section>
  {/if}
</aside>

<style>
  .detail {
    border: 1px solid var(--border);
    background: var(--surface);
    border-radius: var(--radius-l);
    padding: 0.85rem 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.85rem;
    color: var(--text);
  }
  .d-head { display: flex; justify-content: space-between; align-items: flex-start; gap: 0.75rem; }
  .d-title { display: flex; align-items: center; gap: 0.5rem; flex-wrap: wrap; min-width: 0; }
  .d-title strong { font-size: 1rem; min-width: 0; overflow-wrap: anywhere; }
  .block { display: flex; flex-direction: column; gap: 0.5rem; }
  .h { margin: 0; font-size: 0.78rem; text-transform: uppercase; letter-spacing: 0.04em; color: var(--text-dim); }
  .goal { font-size: 0.92rem; line-height: 1.45; }
  .src-row { display: flex; align-items: center; gap: 0.4rem; font-size: 0.82rem; flex-wrap: wrap; }
  .link { color: var(--accent-text); display: inline-flex; align-items: center; gap: 0.25rem; overflow-wrap: anywhere; }
  .muted { color: var(--text-dim); }
  .mono { font-family: var(--font-mono); font-size: 0.78rem; }
  .dot { color: var(--text-dim); }
  .stats { flex-direction: row; align-items: center; gap: 0.65rem; flex-wrap: wrap; }
  .findings { font-size: 0.82rem; color: var(--text-dim); }
  .fnum { color: var(--text); font-weight: 600; }
  .blocking {
    margin-inline-start: 0.35rem; font-size: 0.72rem; padding: 0.05rem 0.45rem; border-radius: 999px;
    background: color-mix(in srgb, var(--status-exited) 16%, transparent); color: var(--status-exited);
  }
  .timeline { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 0.55rem; }
  .tl-item { display: flex; gap: 0.6rem; }
  .tl-dot { width: 9px; height: 9px; border-radius: 999px; margin-top: 0.3rem; flex: none; background: var(--text-dim); }
  .tl-dot.ok { background: var(--status-working); }
  .tl-dot.bad { background: var(--status-exited); }
  .tl-dot.warn { background: var(--status-warn); }
  .tl-dot.active { background: var(--accent); }
  .tl-dot.dim { background: var(--text-dim); }
  .tl-body { display: flex; flex-direction: column; gap: 0.15rem; min-width: 0; }
  .tl-top { display: flex; align-items: center; gap: 0.45rem; flex-wrap: wrap; }
  .tl-kind { font-size: 0.85rem; font-weight: 600; }
  .tl-when { color: var(--text-dim); font-size: 0.74rem; font-variant-numeric: tabular-nums; }
  .tl-msg { font-size: 0.82rem; color: var(--text-dim); line-height: 1.4; }
  .gate { border: 1px solid color-mix(in srgb, var(--status-warn) 40%, transparent); border-radius: var(--radius-m); padding: 0.65rem 0.75rem; background: color-mix(in srgb, var(--status-warn) 7%, transparent); }
  .actions { display: flex; align-items: center; gap: 0.5rem; flex-wrap: wrap; }
  .hint { font-size: 0.8rem; }
  .pr-title { font-size: 0.92rem; font-weight: 600; }
  .pr-desc {
    white-space: pre-wrap; word-break: break-word; font-family: var(--font-mono);
    font-size: 0.78rem; line-height: 1.45; background: var(--bg); border: 1px solid var(--border);
    border-radius: var(--radius-s); padding: 0.5rem 0.6rem; max-height: 16rem; overflow: auto; margin: 0;
  }
  .summary { font-size: 0.85rem; line-height: 1.45; }
  textarea {
    width: 100%; box-sizing: border-box; background: var(--bg); color: var(--text);
    border: 1px solid var(--border); border-radius: var(--radius-s); padding: 0.45rem 0.55rem; font: inherit;
  }
  .err {
    background: var(--danger-soft);
    color: var(--danger); padding: 0.5rem 0.75rem; overflow-wrap: anywhere;
    border-radius: var(--radius-s); font-size: 0.85rem;
  }
  .badge {
    font-size: 0.7rem; padding: 0.05rem 0.45rem; border-radius: 999px;
    border: 1px solid var(--border); color: var(--text-dim); text-transform: capitalize;
  }
  .src-badge {
    color: var(--src);
    border-color: color-mix(in srgb, var(--src) 40%, var(--border));
    background: color-mix(in srgb, var(--src) 10%, transparent);
  }
  .pill {
    font-size: 0.7rem; padding: 0.05rem 0.5rem; border-radius: 999px;
    border: 1px solid transparent; text-transform: capitalize; white-space: nowrap;
  }
  .pill.tiny { font-size: var(--fs-xs); padding: 0.02rem 0.4rem; }
  .pill.ok { background: color-mix(in srgb, var(--status-working) 16%, transparent); color: var(--status-working); }
  .pill.bad { background: color-mix(in srgb, var(--status-exited) 16%, transparent); color: var(--status-exited); }
  .pill.warn { background: color-mix(in srgb, var(--status-warn) 18%, transparent); color: var(--status-warn); }
  .pill.active { background: color-mix(in srgb, var(--accent) 16%, transparent); color: var(--accent-text); }
  .pill.dim { background: color-mix(in srgb, var(--text-dim) 14%, transparent); color: var(--text-dim); }
</style>
