<!--
  The Personal Agents list: one readable row per agent — avatar with a live
  presence dot, full name (the main agent first, starred) and one muted status
  line (Working… / Needs you / Paused / next routine). "New agent" sits on top.
  ⌘1–9 jump to an agent.
-->
<script lang="ts">
  import Icon from '../../lib/components/Icon.svelte';
  import { personalAgents, type AgentPresence } from '../../lib/stores/personalAgents.svelte';
  import type { PersonalAgent } from '../../lib/api/types';
  import AgentAvatar from './AgentAvatar.svelte';
  import { nextRunWords } from './agentHome';
  import { now } from '../../lib/stores/now.svelte';
  import { agentRunsWorking } from '../../lib/stores/agentRunsWorking.svelte';

  interface Props {
    agents: PersonalAgent[];
    selectedId: string | null;
    onselect: (id: string) => void;
    oncreate: () => void;
  }
  let { agents, selectedId, onselect, oncreate }: Props = $props();

  function status(a: PersonalAgent, p: AgentPresence): string {
    if (!a.enabled) return 'Paused';
    if (p === 'working') return 'Working…';
    if (p === 'needs_you') return 'Needs you';
    if (personalAgents.startingRoutine(a.id, now())) return 'Starting…';
    const next = personalAgents.nextRunAt(a.id);
    if (next) return `Next: ${nextRunWords(next)}`;
    const list = personalAgents.schedulesByAgent[a.id];
    if (list?.length === 0) return 'No routines yet';
    return list?.every((s) => !s.enabled) ? 'Routines paused' : '';
  }

  // A due routine the scheduler's minute tick hasn't started yet counts as
  // working in the status bar too, so it never says "0 working" while the
  // rail and chat say "Starting…".
  $effect(() => {
    const t = now();
    for (const a of agents) {
      agentRunsWorking.set(a.id, a.enabled && !!personalAgents.startingRoutine(a.id, t), 'starting');
      // Its chat turn (a run's follow-up) too — the rail says "Working…".
      agentRunsWorking.set(a.id, personalAgents.presence(a.id) === 'working', 'presence');
    }
  });

  function onkey(e: KeyboardEvent): void {
    // ⌘1–9 switches agents (list order).
    if (!(e.metaKey || e.ctrlKey) || e.altKey || e.shiftKey) return;
    const n = Number(e.key);
    if (!Number.isInteger(n) || n < 1 || n > 9) return;
    const a = agents[n - 1];
    if (!a) return;
    e.preventDefault();
    onselect(a.id);
  }
</script>

<svelte:window onkeydown={onkey} />

<nav class="rail" aria-label="Personal agents" data-testid="pa-rail">
  <button type="button" class="rail-add" data-testid="pa-rail-add" aria-label="New agent" title="New agent" onclick={oncreate}>
    <Icon name="plus" size={14} /> <span>New agent</span>
  </button>
  <ul class="rail-list">
    {#each agents as a, i (a.id)}
      {@const p = personalAgents.presence(a.id)}
      {@const main = !!personalAgents.autonomyById[a.id]?.primary}
      {@const line = status(a, p)}
      <li>
        <button
          type="button"
          class="pa-row"
          class:selected={a.id === selectedId}
          class:paused={!a.enabled}
          aria-current={a.id === selectedId ? 'true' : undefined}
          aria-label={a.name}
          aria-describedby={line ? `pa-st-${a.id}` : undefined}
          title={i < 9 ? `${a.name} (⌘${i + 1})` : a.name}
          data-testid="pa-rail-{a.id}"
          data-presence={p}
          onclick={() => onselect(a.id)}
        >
          <span class="face">
            <AgentAvatar avatar={a.avatar} name={a.name} size={32} />
            {#if a.enabled && p !== 'idle'}<span class="dot {p}" aria-hidden="true"></span>{/if}
          </span>
          <span class="txt">
            <span class="name">
              <span class="ellipsis full" dir="auto">{a.name}</span>
              <span class="short" dir="auto" aria-hidden="true">{a.name}</span>
              {#if main}<span class="star" title="Main agent"><Icon name="star" size={12} /><span class="sr-only">Main agent</span></span>{/if}
            </span>
            {#if line}<span class="line ellipsis" id="pa-st-{a.id}" class:busy={a.enabled && p !== 'idle'}>{line}</span>{/if}
          </span>
        </button>
      </li>
    {/each}
  </ul>
</nav>

<style>
  .rail {
    display: flex;
    flex-direction: column;
    gap: 8px;
    inline-size: 232px;
    flex: none;
    padding: 10px 8px;
    border-inline-end: 1px solid var(--border);
    overflow-y: auto;
    min-block-size: 0;
  }
  .rail-add {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border-radius: var(--radius-m);
    border: 1px dashed var(--border-strong);
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-m);
    cursor: pointer;
  }
  .rail-add:hover {
    color: var(--text);
    background: var(--hover);
  }
  .rail-add:focus-visible,
  .pa-row:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  .rail-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .pa-row {
    display: flex;
    align-items: center;
    gap: 10px;
    inline-size: 100%;
    padding: 6px 8px;
    border: 0;
    border-radius: var(--radius-m);
    background: transparent;
    color: var(--text);
    font: inherit;
    text-align: start;
    cursor: pointer;
  }
  .pa-row:hover {
    background: var(--hover);
  }
  .pa-row.selected {
    background: var(--accent-soft);
  }
  .pa-row.paused .face {
    opacity: 0.55;
  }
  .face {
    position: relative;
    display: inline-grid;
    flex: none;
  }
  .dot {
    position: absolute;
    inset-block-end: -2px;
    inset-inline-end: -2px;
    inline-size: 10px;
    block-size: 10px;
    border-radius: 50%;
    border: 2px solid var(--surface);
  }
  .dot.working {
    background: var(--success);
  }
  .dot.needs_you {
    background: var(--warning);
  }
  @media (prefers-reduced-motion: no-preference) {
    .dot.working {
      animation: otto-pulse 1.6s ease-in-out infinite alternate;
    }
  }
  .txt {
    display: flex;
    flex-direction: column;
    min-inline-size: 0;
    flex: 1;
  }
  .name {
    display: flex;
    align-items: center;
    gap: 4px;
    min-inline-size: 0;
    font-size: var(--fs-m);
    font-weight: 500;
  }
  .star {
    display: inline-flex;
    color: var(--accent-text);
    flex: none;
  }
  .line {
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .line.busy {
    color: var(--text);
  }
  .short {
    display: none;
  }
  /* Phone: a strip of faces, each with its name under it (two lines, then …); a paused
     agent is dimmed and a working one keeps its live dot, as on desktop. */
  @media (max-width: 640px) {
    .rail {
      flex-direction: row;
      align-items: center;
      inline-size: 100%;
      padding: 6px 12px;
      border-inline-end: 0;
      border-block-end: 1px solid var(--border);
      overflow-x: auto;
      overflow-y: hidden;
      /* The strip scrolls: its end fades out, a cue that more agents follow
         (the end padding lets the last tile clear the fade). A mask reads
         only alpha, so any opaque token works as the "show" colour. */
      padding-inline-end: 28px;
      mask-image: linear-gradient(to right, var(--text) calc(100% - 28px), transparent);
    }
    :global([dir='rtl']) .rail {
      mask-image: linear-gradient(to left, var(--text) calc(100% - 28px), transparent);
    }
    /* Like an agent tile: the + over a "New agent" caption. */
    .rail-add {
      flex-direction: column;
      gap: 2px;
      inline-size: 64px;
      flex: none;
      font-size: var(--fs-s);
    }
    .rail-list {
      flex-direction: row;
      gap: 4px;
    }
    .rail-add span {
      white-space: nowrap;
    }
    .pa-row {
      flex-direction: column;
      gap: 2px;
      inline-size: 76px;
      padding: 4px 2px;
    }
    .txt {
      inline-size: 100%;
    }
    .name {
      justify-content: center;
      font-size: var(--fs-s);
      font-weight: 400;
    }
    .line,
    .full {
      display: none;
    }
    .short {
      display: -webkit-box;
      -webkit-box-orient: vertical;
      -webkit-line-clamp: 2;
      line-clamp: 2;
      overflow: hidden;
      text-align: center;
      line-height: 1.2;
      /* Break between words ("Casino / Reviewer"); a single long word still wraps. */
      overflow-wrap: break-word;
    }
    /* The star rides on the name's row end, out of the name's width. */
    .pa-row {
      position: relative;
    }
    .star {
      position: absolute;
      inset-block-start: 2px;
      inset-inline-end: 6px;
    }
    .pa-row.paused .txt {
      opacity: 0.55;
    }
  }
</style>
