<script lang="ts">
  import { loops } from '../../lib/stores/loops.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import SessionView from '../agents/SessionView.svelte';
  import IterationRow from './IterationRow.svelte';
  import { humanVerification } from './verification';
  import type { GoalLoop } from '../../lib/api/types';

  let { id, onback }: { id: string; onback: () => void } = $props();

  let answers = $state<Record<string, string>>({});
  let evidence = $state<Record<string, string>>({});

  let openSessionId = $state<string | null>(null);

  // Load + poll the open detail; stop polling when this view goes away.
  $effect(() => {
    void loops.loadDetail(id);
    loops.startPoll(id);
    return () => loops.stopPoll();
  });

  const detail = $derived(loops.detail && loops.detail.loop.id === id ? loops.detail : null);
  const loop = $derived<GoalLoop | null>(detail?.loop ?? null);
  // Newest iteration first.
  const iterations = $derived([...(detail?.iterations ?? [])].reverse());

  const PHASES = ['planning', 'executing', 'evaluating', 'digesting'] as const;
  function phaseIndex(loop: GoalLoop): number {
    const p = loop.phase === 'waiting' ? 'executing' : loop.phase;
    return PHASES.indexOf(p as (typeof PHASES)[number]);
  }

  function elapsedSecs(l: GoalLoop): number {
    let s = l.elapsed_secs;
    if (l.status === 'running' && l.run_started_at) {
      s += Math.max(0, (Date.now() - new Date(l.run_started_at).getTime()) / 1000);
    }
    return s;
  }
  function mins(secs: number): string {
    return `${Math.round(secs / 60)}m`;
  }

  async function act(fn: () => Promise<void>, label: string): Promise<void> {
    try {
      await fn();
    } catch (e) {
      toasts.error(`${label} failed`, e instanceof Error ? e.message : String(e));
    }
  }
  async function del(): Promise<void> {
    await act(() => loops.remove(id), 'Delete');
    onback();
  }
</script>

<div class="detail">
  <header class="head">
    <button class="btn ghost" onclick={onback}>← Back</button>
    {#if loop}
      <h1>{loop.name}</h1>
      <span class="status {loop.status}">{loop.status}</span>
      <span class="spacer"></span>
      {#if loop.status === 'running'}
        <button class="btn" onclick={() => act(() => loops.pause(id), 'Pause')}>Pause</button>
        <button class="btn danger" onclick={() => act(() => loops.stop(id), 'Stop')}>Stop</button>
      {:else if loop.status === 'paused' || loop.status === 'blocked' || loop.status === 'exhausted'}
        <button class="btn primary" disabled={loop.ledger?.questions.some(q => !q.answer)} onclick={() => act(() => loops.resume(id), 'Resume')}>Resume</button>
        <button class="btn danger" onclick={() => act(() => loops.stop(id), 'Stop')}>Stop</button>
        <button class="btn ghost" onclick={del}>Delete</button>
      {:else}
        <button class="btn ghost" onclick={del}>Delete</button>
      {/if}
    {/if}
  </header>

  {#if !loop}
    <p class="muted">Loading…</p>
  {:else}
    <div class="bar"><span class="bar-fill" style:width={`${loop.progress_pct}%`}></span></div>

    <div class="stepper">
      {#each PHASES as p, i (p)}
        <span class="step" class:active={loop.status === 'running' && phaseIndex(loop) === i}>
          <span class="step-dot"></span>{p}
        </span>
      {/each}
    </div>

    <div class="meta">
      <span>iteration {loop.current_iteration}/{loop.limits.max_iterations}</span>
      <span>{mins(elapsedSecs(loop))} / {mins(loop.limits.max_runtime_secs)}</span>
      <span>{loop.progress_pct}% complete</span>
      {#if loop.branch}<span class="mono">{loop.branch}</span>{/if}
    </div>

    {#if loop.summary}
      <p class="summary">{loop.summary}</p>
    {/if}
    {#if loop.error}
      <p class="errline">{loop.error}</p>
    {/if}

    {#if loop.worktree_path}<p class="muted">Retained work: <code>{loop.worktree_path}</code></p>{/if}
    {#if loop.ledger?.next_action}<p class="summary"><strong>Next action:</strong> {loop.ledger.next_action}</p>{/if}
    {#if loop.ledger?.review_summary}
      <details><summary>Completion review · {loop.ledger.review_passed ? 'passed' : 'needs attention'}</summary><pre>{loop.ledger.review_summary}</pre></details>
    {/if}
    {#each loop.ledger?.questions ?? [] as q (q.id)}
      <section class="goal">
        <h3>Decision needed</h3><p>{q.question}</p>
        {#if q.answer}<p>{q.answer} <span class="dim">— {q.answered_by}</span></p>
        {:else}
          <textarea aria-label="Answer question" bind:value={answers[q.id]}></textarea>
          <button class="btn" disabled={!answers[q.id]?.trim() || loop.status !== 'blocked'} onclick={() => act(() => loops.answerQuestion(id, q.id, answers[q.id]), 'Answer')}>Record answer</button>
        {/if}
      </section>
    {/each}
    <section class="goal">
      <h3>Goal</h3>
      <p class="muted">{loop.definition.summary || loop.definition.title}</p>
      <ul class="crit-list">
        {#each loop.definition.acceptance_criteria as c (c.id)}
          <li><strong>{c.id}</strong> {c.text} <span class="mono dim">— {c.verify}</span>
            {#if c.verify_kind === 'human'}
              {@const approval = humanVerification(c, loop.ledger)}
              {#if approval}<p>Verified by {approval.verified_by}: {approval.evidence}</p>
              {:else}
                <p class="dim">Human verification required</p>
                <textarea aria-label={`Evidence for ${c.id}`} placeholder="What did you verify?" bind:value={evidence[c.id]}></textarea>
                <button class="btn small" disabled={!evidence[c.id]?.trim() || !['paused', 'blocked', 'exhausted'].includes(loop.status)} onclick={() => act(() => loops.verifyCriterion(id, c.id, evidence[c.id]), 'Verification')}>Record verification</button>
              {/if}
            {/if}
          </li>
        {/each}
      </ul>
    </section>

    {#if openSessionId}
      <section class="sess">
        <SessionView
          sessionId={openSessionId}
          focused={true}
          showClose={true}
          onfocus={() => {}}
          onclosepane={() => (openSessionId = null)}
        />
      </section>
    {/if}

    <section class="timeline">
      <h3>Iterations</h3>
      {#if iterations.length === 0}
        <p class="muted">No iterations yet.</p>
      {:else}
        {#each iterations as it, i (it.id)}
          <IterationRow
            iter={it}
            loopId={id}
            loopStatus={loop.status}
            executorCount={loop.config.executors.length}
            open={i === 0}
            onopensession={(sid) => (openSessionId = sid)}
          />
        {/each}
      {/if}
    </section>
  {/if}
</div>

<style>
  textarea { width: 100%; min-height: 60px; box-sizing: border-box; background: var(--surface); color: var(--text); border: 1px solid var(--border); }
  pre { white-space: pre-wrap; overflow-wrap: anywhere; }
  .detail {
    padding: 16px 22px;
    overflow-y: auto;
    height: 100%;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 12px;
  }
  h1 {
    font-size: 16px;
    margin: 0;
  }
  .spacer {
    flex: 1;
  }
  .status {
    font-size: 11px;
    padding: 1px 8px;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text-dim);
    text-transform: capitalize;
  }
  .status.running {
    background: color-mix(in srgb, var(--status-working) 18%, transparent);
    color: var(--status-working);
  }
  .status.succeeded {
    background: #7ee787;
    color: #0a0a0a;
    font-weight: 600;
  }
  .status.failed,
  .status.stopped {
    background: color-mix(in srgb, var(--status-exited) 16%, transparent);
    color: var(--status-exited);
  }
  .status.blocked,
  .status.exhausted {
    background: var(--status-warn-soft);
    color: var(--status-warn);
  }
  .bar {
    height: 6px;
    border-radius: 3px;
    background: var(--surface-2);
    overflow: hidden;
    margin-bottom: 12px;
  }
  .bar-fill {
    display: block;
    height: 100%;
    background: var(--status-working);
  }
  .stepper {
    display: flex;
    gap: 18px;
    margin-bottom: 10px;
  }
  .step {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--text-dim);
    text-transform: capitalize;
  }
  .step.active {
    color: var(--status-working);
    font-weight: 600;
  }
  .step-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--surface-2);
  }
  .step.active .step-dot {
    background: var(--status-working);
  }
  .meta {
    display: flex;
    gap: 16px;
    font-size: 12px;
    color: var(--text-dim);
    margin-bottom: 10px;
  }
  .mono {
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 11.5px;
  }
  .dim {
    color: var(--text-dim);
  }
  .summary {
    font-size: 12.5px;
    margin: 0 0 10px;
  }
  .errline {
    font-size: 12.5px;
    color: var(--status-exited);
    margin: 0 0 10px;
  }
  h3 {
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
    margin: 16px 0 8px;
  }
  .crit-list {
    margin: 0;
    padding-left: 18px;
    font-size: 12.5px;
  }
  .crit-list li {
    margin-bottom: 3px;
  }
  .sess {
    height: 360px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
    margin: 12px 0;
  }
</style>
