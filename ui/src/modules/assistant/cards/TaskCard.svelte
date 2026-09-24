<script lang="ts">
  // A longer job the assistant took on, with its LIVE state (queued → running
  // → needs you → done / failed), what it is doing now, and Stop while it runs.
  import ActionCard from './ActionCard.svelte';
  import StatePill from './StatePill.svelte';
  import { assistant, describeError } from '../../../lib/stores/assistant.svelte';
  import { rel } from '../../../lib/stores/now.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { confirmer } from '../../../lib/confirm.svelte';
  import { TASK_KIND, taskStateLabel, taskTone } from '../model';
  import type { AssistantTask } from '../../../lib/api/types';

  interface Props {
    task: AssistantTask;
    onopen?: () => void;
  }
  let { task, onopen }: Props = $props();

  const summary = $derived(typeof task.result?.summary === 'string' ? (task.result.summary as string) : null);
  let busy = $state(false);
  async function stop(): Promise<void> {
    const ok = await confirmer.ask(`Stop “${task.title}”? Otto drops what it has not finished.`, { title: 'Stop task', confirmLabel: 'Stop' });
    if (!ok) return;
    busy = true;
    try {
      await assistant.act(task.id, 'cancel');
    } catch (e) {
      toasts.error('Couldn’t stop the task', describeError(e));
    } finally {
      busy = false;
    }
  }
</script>

<ActionCard icon="check" kind={TASK_KIND[task.kind] ?? 'Task'} summary={task.title} attention={task.state === 'needs_you'} testid="card-task">
  {#snippet pill()}
    <StatePill tone={taskTone(task)} label={taskStateLabel(task)} live={task.state === 'running'} />
  {/snippet}
  {#if task.detail}<p class="detail">{task.detail}</p>{/if}
  {#if summary && task.state !== 'running'}<p class="detail">{summary}</p>{/if}
  <p class="meta">
    Updated <time datetime={task.updated_at} title={new Date(task.updated_at).toLocaleString()}>{rel(task.updated_at)}</time>
  </p>
  {#snippet footer()}
    {#if task.state === 'running' || task.state === 'queued'}
      <button class="btn small ghost" onclick={() => void stop()} disabled={busy}>{busy ? 'Stopping…' : 'Stop'}</button>
    {/if}
    {#if onopen}<button class="btn small" onclick={onopen}>Open in Tasks</button>{/if}
  {/snippet}
</ActionCard>

<style>
  .detail {
    margin: 0 0 6px;
    white-space: pre-line;
  }
  .meta {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
</style>
