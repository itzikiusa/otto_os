<script lang="ts">
  // Quiet one-line entries in the thread: a delegation to a Personal Agent
  // ("Asked Daily Recap to …"), a provider switch, a limit notice, or any
  // other system line the daemon posted. Attributed, never louder than chat.
  import Icon from '../../../lib/components/Icon.svelte';
  import ProviderIcon from '../../../lib/components/ProviderIcon.svelte';
  import StatePill from './StatePill.svelte';
  import { router } from '../../../lib/router.svelte';
  import { taskStateLabel, taskTone } from '../model';
  import type { AssistantTask, AssistantTurn } from '../../../lib/api/types';

  interface Props {
    turn: AssistantTurn | null;
    /** The delegation task, when this line is one. */
    task?: AssistantTask | null;
  }
  let { turn, task = null }: Props = $props();

  const to = $derived(typeof turn?.data?.to === 'string' ? (turn.data.to as string) : turn?.provider ?? null);
  const agentName = $derived(typeof task?.result?.agent_name === 'string' ? (task.result.agent_name as string) : null);
</script>

{#if task?.kind === 'delegation' || turn?.kind === 'delegation'}
  <p class="line" data-testid="line-delegation">
    <Icon name="user" size={12} />
    <span class="what">{turn?.text || `Asked ${agentName ?? 'a Personal Agent'}: ${task?.title ?? ''}`}</span>
    {#if task?.agent_id}
      <button class="link" onclick={() => router.go(`personal-agents/${task.agent_id}`)}>Open agent</button>
    {/if}
    {#if task}<StatePill tone={taskTone(task)} label={taskStateLabel(task)} live={task.state === 'running'} />{/if}
  </p>
{:else if turn?.kind === 'route'}
  <p class="line center" data-testid="line-route">
    {#if to}<ProviderIcon provider={to} size={12} />{/if}
    <span class="what">{turn.text}</span>
  </p>
{:else if turn?.kind === 'limit'}
  <p class="line limit" data-testid="line-limit" role="status">
    <Icon name="warning" size={12} />
    <span class="what">{turn.text}</span>
  </p>
{:else if turn}
  <p class="line center"><span class="what">{turn.text}</span></p>
{/if}

<style>
  .line {
    margin: 0;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .line.center {
    justify-content: center;
    text-align: center;
  }
  .line.limit :global(svg) {
    color: var(--warning);
  }
  .what {
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .link {
    border: 0;
    background: transparent;
    padding: 0;
    font: inherit;
    color: var(--accent-text);
    cursor: pointer;
    border-radius: var(--radius-s);
  }
  .link:hover {
    text-decoration: underline;
  }
</style>
