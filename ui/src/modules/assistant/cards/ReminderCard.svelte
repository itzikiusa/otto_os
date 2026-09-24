<script lang="ts">
  // A one-shot reminder: when it fires and where it is delivered (back to the
  // origin that asked, plus a notification). Cancel while it's still pending.
  import ActionCard from './ActionCard.svelte';
  import StatePill from './StatePill.svelte';
  import { assistant, describeError } from '../../../lib/stores/assistant.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { taskStateLabel, taskTone } from '../model';
  import { deliverLabel, whenLabel } from '../format';
  import type { AssistantTask } from '../../../lib/api/types';

  interface Props {
    task: AssistantTask;
  }
  let { task }: Props = $props();

  let busy = $state(false);
  async function cancel(): Promise<void> {
    busy = true;
    try {
      await assistant.act(task.id, 'cancel');
    } catch (e) {
      toasts.error('Couldn’t cancel the reminder', describeError(e));
    } finally {
      busy = false;
    }
  }
</script>

<ActionCard icon="clock" kind="Reminder" summary={task.title} testid="card-reminder">
  {#snippet pill()}<StatePill tone={taskTone(task)} label={taskStateLabel(task)} />{/snippet}
  <p class="line">
    {#if task.run_at}
      <time datetime={task.run_at} title={new Date(task.run_at).toLocaleString()}>{whenLabel(task.run_at)}</time>
      <span class="sep" aria-hidden="true">·</span>
    {/if}
    <span>{deliverLabel(task.origin)}</span>
    <span class="sep" aria-hidden="true">·</span>
    <span class="dim">one-shot</span>
  </p>
  {#if task.detail}<p class="detail">{task.detail}</p>{/if}
  {#snippet footer()}
    {#if task.state === 'queued'}
      <button class="btn small ghost" onclick={() => void cancel()} disabled={busy}>{busy ? 'Cancelling…' : 'Cancel reminder'}</button>
    {/if}
  {/snippet}
</ActionCard>

<style>
  .line {
    margin: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 4px 6px;
  }
  .detail {
    margin: 6px 0 0;
  }
  .sep,
  .dim {
    color: var(--text-dim);
  }
</style>
