<script lang="ts">
  // A subagent the agent launched (Claude's Agent / Task tool): name and
  // description, status (running / done / failed / stopped), elapsed time,
  // its live last step, and — expanded — its own messages and tool calls
  // (the items whose `parent_item_id` is this card) plus its final answer.
  // A background subagent keeps running after its turn ended; the card says
  // so until its task finishes.
  import Icon from '../../../lib/components/Icon.svelte';
  import type { TurnItem } from '../../../lib/api/types';
  import Markdown from '../../agents/conversation/Markdown.svelte'; // ui-guards: allow — shared chat markdown
  import { now } from '../../../lib/stores/now.svelte';
  import RuntimeWorkRow from './RuntimeWorkRow.svelte';

  type SubagentItem = Extract<TurnItem, { type: 'subagent' }>;
  interface Props {
    item: SubagentItem;
    /** Its own activity, in order. */
    activity: TurnItem[];
    /** Live `command_output.delta` text by item id. */
    output?: Record<string, string>;
  }
  let { item, activity, output = {} }: Props = $props();

  let open = $state(false);
  const running = $derived(item.status === 'running' || item.status === 'pending' || item.status === 'waiting');
  const statusText = $derived(
    running ? 'Running' : item.status === 'completed' ? 'Done' : item.status === 'failed' ? 'Failed' : 'Stopped',
  );
  const name = $derived(item.description ?? item.title ?? item.subagent_type ?? 'Subagent');
  const elapsed = $derived.by(() => {
    if (!item.started_at) return null;
    const end = running ? now() : Date.parse(item.completed_at ?? item.updated_at);
    const s = Math.max(0, Math.floor((end - Date.parse(item.started_at)) / 1000));
    return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m ${s % 60}s`;
  });
  const lastStep = $derived.by(() => {
    if (!running) return null;
    const last = activity.findLast((i) => i.type !== 'assistant_message');
    return item.last_tool ?? last?.title ?? null;
  });
  const steps = $derived(item.tool_uses > 0 ? `${item.tool_uses} ${item.tool_uses === 1 ? 'step' : 'steps'}` : null);
</script>

<div class="sa" class:failed={item.status === 'failed'} data-testid="runtime-subagent-{item.id}" data-status={item.status}>
  <button type="button" class="sa-head" aria-expanded={open} onclick={() => (open = !open)}>
    <Icon name="assistant" size={14} />
    <span class="sa-name ellipsis" dir="auto">{name}</span>
    {#if item.subagent_type && item.subagent_type !== name}<span class="sa-chip">{item.subagent_type}</span>{/if}
    {#if item.background}<span class="sa-chip" title="Runs in the background while the conversation goes on">Background</span>{/if}
    <span class="sa-grow"></span>
    <span class="sa-state" class:live={running}>
      {#if running}<span class="spinner" aria-hidden="true"></span>{:else if item.status === 'completed'}<Icon name="check" size={12} />{:else if item.status === 'failed'}<Icon name="x" size={12} />{/if}
      {statusText}{#if elapsed} · {elapsed}{/if}
    </span>
    <Icon name={open ? 'chevronDown' : 'chevronRight'} size={12} />
  </button>
  {#if !open && (lastStep || steps)}
    <p class="sa-meta ellipsis" dir="auto">{[steps, lastStep ? `last: ${lastStep}` : null].filter(Boolean).join(' · ')}</p>
  {/if}
  {#if open}
    <div class="sa-body">
      {#if item.prompt}
        <p class="sa-label">Task</p>
        <p class="sa-prompt" dir="auto">{item.prompt}</p>
      {/if}
      {#if activity.length > 0}
        <p class="sa-label">Activity</p>
        <div class="sa-activity">
          {#each activity as a (a.id)}
            {#if a.type === 'assistant_message'}
              <div class="sa-text" dir="auto"><Markdown md={a.text} /></div>
            {:else}
              <RuntimeWorkRow item={a} output={output[a.id]} />
            {/if}
          {/each}
        </div>
      {:else if running}
        <p class="sa-meta">Working…</p>
      {/if}
      {#if item.error}
        <p class="sa-error" dir="auto">{item.error}</p>
      {:else if item.output}
        <p class="sa-label">Result</p>
        <div class="sa-text" dir="auto"><Markdown md={item.output} /></div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .sa {
    display: flex;
    flex-direction: column;
    min-width: 0;
    margin-block: 2px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
  }
  .sa.failed {
    border-color: color-mix(in srgb, var(--danger) 40%, var(--border));
  }
  .sa-head {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    padding: 6px 10px;
    border: 0;
    border-radius: var(--radius-m);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    text-align: start;
    cursor: pointer;
  }
  .sa-head:hover {
    background: var(--hover);
  }
  .sa-name {
    min-width: 0;
    font-weight: 600;
  }
  .sa-grow {
    flex: 1;
  }
  .sa-chip {
    flex-shrink: 0;
    padding-inline: 6px;
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .sa-state {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .sa-state.live {
    color: var(--text);
  }
  .sa-state .spinner {
    --spinner-size: 10px;
  }
  .sa-meta {
    margin: 0;
    padding-block: 0 6px;
    padding-inline: 32px 10px;
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .sa-body {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding-block: 2px 8px;
    padding-inline: 32px 10px;
    min-width: 0;
  }
  .sa-label {
    margin: 4px 0 0;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }
  .sa-prompt {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--fs-s);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .sa-activity {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .sa-text {
    min-width: 0;
    color: var(--text);
    font-size: var(--fs-s);
  }
  .sa-error {
    margin: 0;
    color: var(--danger);
    font-size: var(--fs-s);
    overflow-wrap: anywhere;
  }
</style>
