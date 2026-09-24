<script lang="ts">
  // Picks the card for a task in the thread: needs-you items (approval,
  // question, limit, takeover, memory) → NeedsYouCard; a browser chore →
  // BrowserCard; a reminder → ReminderCard; a delegation → a quiet line;
  // anything else → TaskCard. The task row is LIVE (store), so the card
  // follows it through queued → running → needs you → done.
  import { untrack } from 'svelte';
  import { assistant } from '../../../lib/stores/assistant.svelte';
  import { browserOf } from '../model';
  import NeedsYouCard from './NeedsYouCard.svelte';
  import BrowserCard from './BrowserCard.svelte';
  import ReminderCard from './ReminderCard.svelte';
  import TaskCard from './TaskCard.svelte';
  import SystemLine from './SystemLine.svelte';
  import type { AssistantTurn } from '../../../lib/api/types';

  interface Props {
    taskId: string;
    turn: AssistantTurn | null;
    onopentasks?: () => void;
  }
  let { taskId, turn, onopentasks }: Props = $props();

  const task = $derived(assistant.task(taskId));
  $effect(() => {
    const id = taskId;
    if (!task) untrack(() => void assistant.ensureTask(id));
  });
  const progress = $derived(browserOf(task));
  const NEEDS = new Set<string>(['approval', 'question', 'limit', 'takeover', 'memory_review']);
</script>

{#if !task}
  {#if turn}<SystemLine {turn} />{/if}
{:else if progress}
  <BrowserCard {task} {progress} />
{:else if NEEDS.has(task.kind) || task.needs_you}
  <NeedsYouCard {task} />
{:else if task.kind === 'reminder'}
  <ReminderCard {task} />
{:else if task.kind === 'delegation'}
  <SystemLine {turn} {task} />
{:else}
  <TaskCard {task} onopen={onopentasks} />
{/if}
