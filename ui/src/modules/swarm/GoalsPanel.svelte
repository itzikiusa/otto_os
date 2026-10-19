<script lang="ts">
  import { toastError } from '../../lib/toastError';
  // Per-task Goals view: the goals the Coordinator verifies for a task, each with
  // a live status, the measured value + verdict summary, and retry budget. Lets
  // you add / edit / delete goals and run (or stop) verification on demand.
  import Modal from '../../lib/components/Modal.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import GoalEditor from './GoalEditor.svelte';
  import { untrack } from 'svelte';
  import { swarm } from '../../lib/stores/swarm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import StatusBadge from '../../lib/components/StatusBadge.svelte';
  import { sentenceCase, type StatusInfo } from '../../lib/status';
  import type { GoalStatus, SwarmGoal, SwarmTask } from './types';

  interface Props {
    task: SwarmTask;
    onclose: () => void;
  }
  let { task, onclose }: Props = $props();

  const goals = $derived(swarm.goalsForTask(task.id));
  const goalsKnown = $derived(task.id in swarm.goalsByTask);
  const goalsLoading = $derived(!!swarm.taskGoalsLoading[task.id]);
  const goalsError = $derived(swarm.taskGoalsError[task.id] ?? null);
  const verify = $derived(swarm.verifyByTask[task.id] ?? null);
  const running = $derived(verify?.running ?? false);

  let editorOpen = $state(false);
  let editGoal = $state<SwarmGoal | null>(null);
  let verifying = $state(false);

  // loadVerification writes taskGoalsLoading before its first await. Tracking
  // that read would reschedule this effect on every load and freeze the page.
  $effect(() => {
    const id = task.id;
    untrack(() => void swarm.loadVerification(id));
  });

  function goalStatus(status: GoalStatus): StatusInfo {
    switch (status) {
      case 'verifying':
        return { key: status, label: 'Verifying', tone: 'info', live: true };
      case 'passed':
        return { key: status, label: 'Passed', tone: 'success' };
      case 'warned':
        return { key: status, label: 'Warned', tone: 'warning' };
      case 'unmet':
        return { key: status, label: 'Unmet', tone: 'danger' };
      case 'error':
        return { key: status, label: 'Error', tone: 'danger' };
      case 'skipped':
        return { key: status, label: 'Skipped', tone: 'neutral' };
      default:
        return { key: status, label: 'Pending', tone: 'neutral' };
    }
  }

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
      else toasts.info("Verification didn’t start", res.reason ?? 'Verification could not start.');
    } catch (e) {
      toastError("Couldn’t start verification", e);
    } finally {
      verifying = false;
    }
  }

  async function stop() {
    const ok = await confirmer.ask(
      'Stop verifying this task? Goals already checked keep their result. Checks still running do not.',
      { title: 'Stop verification', confirmLabel: 'Stop', danger: true },
    );
    if (!ok) return;
    try {
      await swarm.stopVerify(task.id);
      toasts.info('Verification stopped');
    } catch (e) {
      toastError("Couldn’t stop verification", e);
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
    if (
      await confirmer.ask(`Delete goal “${g.title}”? Its verification history goes with it.`, {
        title: 'Delete goal',
        confirmLabel: 'Delete',
        danger: true,
      })
    ) {
      try {
        await swarm.deleteGoal(g.id);
      } catch (e) {
        toastError("Couldn’t delete the goal", e);
      }
    }
  }
</script>

<Modal title="Goals — {task.title}" width={620} {onclose}>
  <div class="bar">
    {#if running}
      <span class="running" role="status"><span class="spinner" style="--spinner-size: 10px" aria-hidden="true"></span> Verifying…</span>
      <button class="btn small danger" onclick={() => void stop()} title="Stop verifying — goals already checked keep their result"><Icon name="stop" size={12} /> Stop…</button>
    {:else}
      <button class="btn small primary" onclick={runVerify} disabled={verifying || !goals.length} title={goals.length ? 'Run every goal check for this task now' : 'Add a goal first'}>
        <Icon name="check" size={12} /> Verify now
      </button>
    {/if}
    <span class="grow"></span>
    <button class="btn small" onclick={add}><Icon name="plus" size={12} /> Add goal</button>
  </div>

  {#if goalsLoading && !goalsKnown}
    <LoadState what="goals" variant="compact" loading empty rows={3} />
  {:else if goalsError && !goalsKnown}
    <LoadState what="goals" variant="compact" error={goalsError} empty onretry={() => void swarm.loadVerification(task.id)} />
  {:else if goals.length === 0}
    <EmptyState icon="check" title="No goals yet" body="Add a goal so the Coordinator can verify this task is actually done." />
  {:else}
    {#if goalsError}
      <LoadState what="goals" variant="compact" error={goalsError} empty={false} onretry={() => void swarm.loadVerification(task.id)} />
    {/if}
    <div class="goals">
      {#each goals as g (g.id)}
        {@const m = measured(g)}
        <div class="goal">
          <div class="g-head">
            <StatusBadge status={goalStatus(g.status)} />
            <span class="kind">{sentenceCase(g.kind)}</span>
            {#if g.blocking}<span class="blocking" title="Blocks task completion until it passes">Blocking</span>{/if}
            <span class="g-title">{g.title}</span>
            <span class="grow"></span>
            <button class="icon-btn small" onclick={() => edit(g)} aria-label="Edit goal “{g.title}”" title="Edit goal “{g.title}”"><Icon name="edit" size={13} /></button>
            <button class="icon-btn small" onclick={() => del(g)} aria-label="Delete goal “{g.title}”" title="Delete goal “{g.title}”"><Icon name="trash" size={13} /></button>
          </div>
          {#if g.description}<div class="g-desc">{g.description}</div>{/if}
          <div class="g-meta">
            {#if g.metric}
              <span class="pair"><span class="k">Metric</span> {g.metric}{g.comparator ? ` ${g.comparator}` : ''}{g.target_value != null ? ` ${g.target_value}` : ''}</span>
            {/if}
            {#if m !== null}
              <span class="pair measured"><span class="k">Measured</span> {m}</span>
            {/if}
            <span class="pair"><span class="k">Attempts</span> {g.iterations}/{g.max_retries}</span>
            {#if g.verify_cmd}<span class="pair cmd" title={g.verify_cmd}><span class="k">Command</span> {g.verify_cmd}</span>{/if}
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
    font-size: var(--fs-s);
    color: var(--accent-text);
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
    font-size: var(--fs-s);
  }
  .g-title {
    font-weight: 600;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .kind {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 0 6px;
  }
  .blocking {
    font-size: var(--fs-xs);
    color: var(--danger);
    border: 1px solid color-mix(in srgb, var(--danger) 40%, transparent);
    border-radius: 999px;
    padding: 0 6px;
  }
  .g-desc {
    font-size: var(--fs-s);
    color: var(--text-dim);
    margin-top: 4px;
    white-space: pre-wrap;
  }
  .g-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 8px 12px;
    margin-top: 6px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .pair .k {
    color: var(--text-dim);
    margin-inline-end: 2px;
  }
  .pair.measured {
    color: var(--text);
    font-weight: 600;
  }
  .pair.cmd {
    font-family: var(--font-mono);
    max-width: 220px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .verdict {
    margin-top: 6px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    border-inline-start: 2px solid var(--border);
    padding-inline-start: 8px;
  }
  .verdict.bad {
    color: var(--danger);
    border-inline-start-color: var(--danger);
  }

</style>
