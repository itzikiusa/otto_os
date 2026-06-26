<script lang="ts">
  // Per-task Goals view: the goals the Coordinator verifies for a task, each with
  // a live status, the measured value + verdict summary, and retry budget. Lets
  // you add / edit / delete goals and run (or stop) verification on demand.
  import Modal from '../../lib/components/Modal.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import GoalEditor from './GoalEditor.svelte';
  import { swarm } from '../../lib/stores/swarm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import type { GoalStatus, SwarmGoal, SwarmTask } from './types';

  interface Props {
    task: SwarmTask;
    onclose: () => void;
  }
  let { task, onclose }: Props = $props();

  const goals = $derived(swarm.goalsForTask(task.id));
  const verify = $derived(swarm.verifyByTask[task.id] ?? null);
  const running = $derived(verify?.running ?? false);

  let editorOpen = $state(false);
  let editGoal = $state<SwarmGoal | null>(null);
  let verifying = $state(false);

  $effect(() => {
    void swarm.loadVerification(task.id);
  });

  const STATUS_LABEL: Record<GoalStatus, string> = {
    pending: 'Pending',
    verifying: 'Verifying',
    passed: 'Passed',
    warned: 'Warned',
    unmet: 'Unmet',
    skipped: 'Skipped',
    error: 'Error',
  };

  function measured(g: SwarmGoal): string | null {
    const m = g.verdict?.measured;
    if (m === null || m === undefined || m === '') return null;
    return String(m);
  }

  async function runVerify() {
    if (verifying) return;
    verifying = true;
    try {
      const res = await swarm.verifyTask(task.id);
      if (res.started) toasts.success('Verification started', 'Watch goal statuses update live.');
      else toasts.info('Not started', res.reason ?? 'Verification could not start.');
    } catch (e) {
      toasts.error('Verify failed', e instanceof Error ? e.message : String(e));
    } finally {
      verifying = false;
    }
  }

  async function stop() {
    try {
      await swarm.stopVerify(task.id);
      toasts.info('Verification stopped');
    } catch (e) {
      toasts.error('Stop failed', e instanceof Error ? e.message : String(e));
    }
  }

  function add() {
    editGoal = null;
    editorOpen = true;
  }
  function edit(g: SwarmGoal) {
    editGoal = g;
    editorOpen = true;
  }
  async function del(g: SwarmGoal) {
    if (await confirmer.ask(g.title, { title: 'Delete goal?' })) {
      try {
        await swarm.deleteGoal(g.id);
      } catch (e) {
        toasts.error('Delete failed', e instanceof Error ? e.message : String(e));
      }
    }
  }
</script>

<Modal title="Goals — {task.title}" width={620} {onclose}>
  <div class="bar">
    {#if running}
      <span class="running"><span class="spinner-xs"></span> Verifying…</span>
      <button class="btn small" onclick={stop}><Icon name="square" size={12} /> Stop</button>
    {:else}
      <button class="btn small primary" onclick={runVerify} disabled={verifying || !goals.length}>
        <Icon name="check" size={12} /> Verify now
      </button>
    {/if}
    <span class="grow"></span>
    <button class="btn small" onclick={add}><Icon name="plus" size={12} /> Add goal</button>
  </div>

  {#if goals.length === 0}
    <EmptyState icon="check" title="No goals yet" body="Add a goal so the Coordinator can verify this task is actually done." />
  {:else}
    <div class="goals">
      {#each goals as g (g.id)}
        {@const m = measured(g)}
        <div class="goal">
          <div class="g-head">
            <span class="status {g.status}" class:pulse={g.status === 'verifying'}>{STATUS_LABEL[g.status]}</span>
            <span class="kind">{g.kind}</span>
            {#if g.blocking}<span class="blocking" title="Blocks task completion until it passes">blocking</span>{/if}
            <span class="g-title">{g.title}</span>
            <span class="grow"></span>
            <button class="icon-btn small" onclick={() => edit(g)} aria-label="Edit goal"><Icon name="edit" size={13} /></button>
            <button class="icon-btn small" onclick={() => del(g)} aria-label="Delete goal"><Icon name="trash" size={13} /></button>
          </div>
          {#if g.description}<div class="g-desc">{g.description}</div>{/if}
          <div class="g-meta">
            {#if g.metric}
              <span class="pair"><span class="k">metric</span> {g.metric}{g.comparator ? ` ${g.comparator}` : ''}{g.target_value != null ? ` ${g.target_value}` : ''}</span>
            {/if}
            {#if m !== null}
              <span class="pair measured"><span class="k">measured</span> {m}</span>
            {/if}
            <span class="pair"><span class="k">attempts</span> {g.iterations}/{g.max_retries}</span>
            {#if g.verify_cmd}<span class="pair cmd" title={g.verify_cmd}><span class="k">cmd</span> {g.verify_cmd}</span>{/if}
          </div>
          {#if g.verdict?.summary}
            <div class="verdict" class:bad={g.verdict.blocker}>{g.verdict.summary}</div>
          {/if}
        </div>
      {/each}
    </div>
  {/if}
</Modal>

{#if editorOpen}
  <GoalEditor
    goal={editGoal}
    scope={editGoal ? null : { task: task.id }}
    onclose={() => { editorOpen = false; editGoal = null; }}
  />
{/if}

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 10px;
  }
  .running {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--accent);
  }
  .spinner-xs {
    width: 11px;
    height: 11px;
    border: 2px solid color-mix(in srgb, var(--accent) 35%, transparent);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: gp-spin 0.7s linear infinite;
  }
  @keyframes gp-spin {
    to {
      transform: rotate(360deg);
    }
  }
  .goals {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .goal {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 8px 10px;
    background: var(--surface);
  }
  .g-head {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12.5px;
  }
  .g-title {
    font-weight: 600;
  }
  .kind {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 0 6px;
  }
  .blocking {
    font-size: 10px;
    color: var(--status-exited);
    border: 1px solid color-mix(in srgb, var(--status-exited) 40%, transparent);
    border-radius: 999px;
    padding: 0 6px;
  }
  .g-desc {
    font-size: 12px;
    color: var(--text-dim);
    margin-top: 4px;
    white-space: pre-wrap;
  }
  .g-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 8px 12px;
    margin-top: 6px;
    font-size: 11px;
    color: var(--text-dim);
  }
  .pair .k {
    color: var(--text-dim);
    opacity: 0.7;
    margin-inline-end: 3px;
  }
  .pair.measured {
    color: var(--text);
    font-weight: 600;
  }
  .pair.cmd {
    font-family: var(--mono, monospace);
    max-width: 220px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .verdict {
    margin-top: 6px;
    font-size: 11.5px;
    color: var(--text-dim);
    border-inline-start: 2px solid var(--border);
    padding-inline-start: 8px;
  }
  .verdict.bad {
    color: var(--status-exited);
    border-inline-start-color: var(--status-exited);
  }

  /* Status chips — colours per the goal lifecycle. `passed` is the high-contrast
     light-green + black selection colour; verifying pulses. */
  .status {
    font-size: 10.5px;
    font-weight: 600;
    padding: 1px 8px;
    border-radius: 999px;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    background: color-mix(in srgb, var(--text-dim) 18%, transparent);
    color: var(--text-dim);
  }
  .status.verifying {
    background: color-mix(in srgb, #0a84ff 22%, transparent);
    color: #0a84ff;
  }
  .status.passed {
    background: #7ee787;
    color: #0a0a0a;
  }
  .status.warned {
    background: color-mix(in srgb, #e3b341 26%, transparent);
    color: #e3b341;
  }
  .status.unmet,
  .status.error {
    background: color-mix(in srgb, var(--status-exited) 22%, transparent);
    color: var(--status-exited);
  }
  .status.pulse {
    animation: gp-pulse 1.2s ease-in-out infinite;
  }
  @keyframes gp-pulse {
    50% {
      opacity: 0.55;
    }
  }
</style>
