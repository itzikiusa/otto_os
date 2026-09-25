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
  import StatusBadge from '../../lib/components/StatusBadge.svelte';
  import { humanize, isTerminal, runStatusInfo, sourceLabel } from './runStatus';

  interface Props {
    run: OttoRun;
    onClose: () => void;
  }
  let { run, onClose }: Props = $props();

  let busy = $state(false);
  let error = $state('');
  let rejectNote = $state('');
  let rejecting = $state(false);
  let rejectEl: HTMLTextAreaElement | undefined = $state();
  // Reject opens the reason box — put the caret in it.
  $effect(() => {
    if (rejecting) rejectEl?.focus();
  });

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
      <span class="chip">{sourceLabel(run.source_kind)}</span>
      <StatusBadge status={runStatusInfo(run.status)} />
    </div>
    <div class="d-actions">
      <!-- Stop stays reachable at the top while the run is live (patterns.md §1),
           never beside a primary. -->
      {#if !isTerminal(run.status)}
        <button class="btn small danger" disabled={busy} onclick={cancel}>Cancel run</button>
      {/if}
      <button class="icon-btn" onclick={onClose} aria-label="Close run detail" title="Close run detail">
        <Icon name="x" size={14} />
      </button>
    </div>
  </header>
  <h2 class="d-name">{run.title || run.source_ref}</h2>

  {#if error}<div class="err" role="alert">{error}</div>{/if}
  {#if run.status === 'failed' && run.error}
    <div class="err" role="status"><strong>Run failed:</strong> {run.error}</div>
  {/if}

  <!-- where the run is on the pipeline right now -->
  <section class="block">
    <RunStageRail status={run.status} />
  </section>

  <section class="block">
    {#if run.goal}
      <div class="goal">{run.goal}</div>
    {:else}
      <div class="goal muted">No goal text.</div>
    {/if}
    <div class="src-row">
      {#if run.source_url}
        <a class="link" href={run.source_url} target="_blank" rel="noreferrer" title={run.source_url}>{run.source_ref} <Icon name="external" size={12} /></a>
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
      <span class="fnum">{run.findings_total}</span> {run.findings_total === 1 ? 'finding' : 'findings'}
      {#if run.findings_blocking > 0}
        <span class="blocking" title="Findings that block the PR">{run.findings_blocking} blocking</span>
      {/if}
    </span>
    {#if run.branch}<span class="muted mono branch" title={run.branch}><Icon name="branch" size={12} /> <span class="branch-name">{run.branch}</span></span>{/if}
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
            <span class="tl-dot tone-{ev.status ? runStatusInfo(ev.status).tone : 'neutral'}" aria-hidden="true"></span>
            <div class="tl-body">
              <div class="tl-top">
                <span class="tl-kind">{humanize(ev.kind)}</span>
                {#if ev.status}<StatusBadge status={{ ...runStatusInfo(ev.status), live: false }} variant="text" dot={false} />{/if}
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
      <p class="gate-note">
        Approve to draft the PR from <span class="mono">{run.branch || 'the run branch'}</span>. Nothing is pushed
        until you open the PR. Reject ends the run and removes its worktree.
      </p>
      {#if rejecting}
        <textarea
          bind:this={rejectEl}
          bind:value={rejectNote}
          rows="2"
          aria-label="Reason for rejecting (optional)"
          placeholder="Optional reason for rejecting…"
          onkeydown={(e) => { if (e.key === 'Escape') { e.stopPropagation(); rejecting = false; rejectNote = ''; } }}
        ></textarea>
        <div class="actions">
          <button class="btn danger" disabled={busy} onclick={() => approve('reject')}>Reject run</button>
          <button class="btn" disabled={busy} onclick={() => { rejecting = false; rejectNote = ''; }}>Cancel</button>
        </div>
      {:else}
        <div class="actions">
          <button class="btn primary" disabled={busy} onclick={() => approve('approve')}>Approve</button>
          <button class="btn danger" disabled={busy} onclick={() => (rejecting = true)}>Reject…</button>
        </div>
      {/if}
    </section>
  {/if}

  <!-- PR draft -->
  {#if prDraft}
    <section class="block pr">
      <h3 class="h">PR draft</h3>
      <div class="pr-title">{prDraft.title || 'Untitled PR'}</div>
      {#if prDraft.source || prDraft.target}
        <div class="muted mono">{prDraft.source || run.branch}{prDraft.target ? ` → ${prDraft.target}` : ''}</div>
      {/if}
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

</aside>

<style>
  .detail {
    border: 1px solid var(--border);
    background: var(--surface);
    border-radius: var(--radius-l);
    padding: 12px 16px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    color: var(--text);
    min-width: 0;
  }
  .d-head { display: flex; justify-content: space-between; align-items: center; gap: 12px; }
  .d-title { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; min-width: 0; }
  .d-actions { display: flex; align-items: center; gap: 6px; flex: none; }
  .d-name { margin: -6px 0 0; font-size: var(--fs-l); font-weight: 600; line-height: 1.35; overflow-wrap: anywhere; }
  .block { display: flex; flex-direction: column; gap: 8px; min-width: 0; }
  .h {
    margin: 0; font-size: var(--fs-xs); font-weight: 600; text-transform: uppercase;
    letter-spacing: 0.04em; color: var(--text-dim);
  }
  .goal { font-size: var(--fs-m); line-height: 1.5; overflow-wrap: anywhere; }
  .src-row { display: flex; align-items: center; gap: 6px; font-size: var(--fs-s); flex-wrap: wrap; min-width: 0; }
  .link { color: var(--accent-text); display: inline-flex; align-items: center; gap: 4px; overflow-wrap: anywhere; }
  .muted { color: var(--text-dim); }
  .mono { font-family: var(--font-mono); font-size: var(--fs-s); }
  .dot { color: var(--text-dim); }
  .stats { flex-direction: row; align-items: center; gap: 10px; flex-wrap: wrap; }
  .findings { font-size: var(--fs-s); color: var(--text-dim); }
  .fnum { color: var(--text); font-weight: 600; }
  .blocking {
    margin-inline-start: 4px; font-size: var(--fs-xs); padding: 0 6px; border-radius: 999px;
    background: var(--danger-soft); color: var(--danger);
  }
  .branch { display: inline-flex; align-items: center; gap: 4px; min-width: 0; max-width: 100%; }
  .branch-name { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .timeline { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
  .tl-item { display: flex; gap: 10px; }
  .tl-dot { width: 8px; height: 8px; border-radius: 999px; margin-top: 5px; flex: none; background: var(--text-dim); }
  .tl-dot.tone-success { background: var(--status-working); }
  .tl-dot.tone-danger { background: var(--status-exited); }
  .tl-dot.tone-warning { background: var(--status-warn); }
  .tl-dot.tone-info { background: var(--accent); }
  .tl-body { display: flex; flex-direction: column; gap: 2px; min-width: 0; flex: 1; }
  .tl-top { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .tl-kind { font-size: var(--fs-m); font-weight: 600; }
  .tl-when { color: var(--text-dim); font-size: var(--fs-xs); font-variant-numeric: tabular-nums; margin-inline-start: auto; }
  .tl-msg { font-size: var(--fs-s); color: var(--text-dim); line-height: 1.45; overflow-wrap: anywhere; }
  .gate {
    border: 1px solid color-mix(in srgb, var(--warning) 40%, transparent);
    border-radius: var(--radius-m);
    padding: 10px 12px;
    background: var(--warning-soft);
  }
  .gate-note { margin: 0; font-size: var(--fs-s); color: var(--text); line-height: 1.45; }
  .actions { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .hint { font-size: var(--fs-s); }
  .pr-title { font-size: var(--fs-m); font-weight: 600; overflow-wrap: anywhere; }
  .pr-desc {
    white-space: pre-wrap; word-break: break-word; font-family: var(--font-mono);
    font-size: var(--fs-s); line-height: 1.45; background: var(--bg); border: 1px solid var(--border);
    border-radius: var(--radius-s); padding: 8px 10px; max-height: 16rem; overflow: auto; margin: 0;
  }
  .summary { font-size: var(--fs-m); line-height: 1.5; overflow-wrap: anywhere; }
  textarea {
    width: 100%; box-sizing: border-box; background: var(--bg); color: var(--text);
    border: 1px solid var(--border); border-radius: var(--radius-s); padding: 6px 9px; font: inherit;
  }
  textarea:focus-visible {
    outline: none;
    border-color: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 22%, transparent);
  }
  .err {
    background: var(--danger-soft);
    color: var(--danger); padding: 6px 10px; overflow-wrap: anywhere;
    border-radius: var(--radius-s); font-size: var(--fs-s);
  }
</style>
