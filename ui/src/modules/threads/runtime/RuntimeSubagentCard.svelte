<script lang="ts">
  // A subagent the agent launched (Claude's Agent / Task tool) as one compact
  // row (RuntimeAgentRow, shared with the agent's thread_open calls): name,
  // type, status, elapsed time and one line — its live last step while it
  // runs, its result once done; expanded, its own messages and tool calls
  // (the items whose `parent_item_id` is this card) plus its final answer.
  // A background subagent keeps running after its turn ended; the card says
  // so until its task finishes.
  import { untrack } from 'svelte';
  import type { TurnItem } from '../../../lib/api/types';
  import Markdown from '../../agents/conversation/Markdown.svelte'; // ui-guards: allow — shared chat markdown
  import { now } from '../../../lib/stores/now.svelte';
  import RuntimeAgentRow from './RuntimeAgentRow.svelte';
  import RuntimeWorkRow from './RuntimeWorkRow.svelte';
  import RuntimeSwarmWork from './RuntimeSwarmWork.svelte';

  type SubagentItem = Extract<TurnItem, { type: 'subagent' }>;
  interface Props {
    item: SubagentItem;
    /** Its own activity, in order. */
    activity: TurnItem[];
    /** Live `command_output.delta` text by item id. */
    output?: Record<string, string>;
    /** Its thread (an Antigravity swarm member's work is read from the daemon). */
    sessionId?: string | null;
    /** Given: the row opens it elsewhere (Agents & tasks' drill-in) instead of expanding. */
    onopen?: () => void;
    /** Expanded from the start (the drill-in). */
    startOpen?: boolean;
  }
  let { item, activity, output = {}, sessionId = null, onopen, startOpen = false }: Props = $props();

  let open = $state(untrack(() => startOpen));
  /** agy's swarm member: its work is on disk, not in the item stream. */
  const swarm = $derived(item.tool_name === 'swarm' && !!sessionId);
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
  const tone = $derived<'live' | 'done' | 'failed' | 'off'>(running ? 'live' : item.status === 'completed' ? 'done' : item.status === 'failed' ? 'failed' : 'off');
  const sub = $derived([item.subagent_type && item.subagent_type !== name ? item.subagent_type : null, item.background ? 'Background' : null].filter(Boolean).join(' · ') || null);
  /** One line: the live step, else the error or the answer's first line. */
  const result = $derived(
    running
      ? [steps, lastStep ? `last: ${lastStep}` : null].filter(Boolean).join(' · ') || null
      : (item.error ?? item.output)?.trim().split('\n').find((l) => l.trim())?.trim() ?? steps,
  );
</script>

<div class="sa" class:failed={item.status === 'failed'} data-testid="runtime-subagent-{item.id}" data-status={item.status}>
  <RuntimeAgentRow {name} {sub} {tone} status={statusText} {elapsed} {result} expanded={open} ontoggle={onopen ?? (() => (open = !open))} />
  {#if open && !onopen}
    <div class="sa-body">
      {#if item.prompt}
        <p class="sa-label">Task</p>
        <p class="sa-prompt" dir="auto">{item.prompt}</p>
      {/if}
      {#if swarm && sessionId}
        <RuntimeSwarmWork {sessionId} member={item.description ?? name} live={running} />
      {:else if activity.length > 0}
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
