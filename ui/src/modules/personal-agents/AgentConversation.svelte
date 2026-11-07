<!--
  The center of the Personal Agents home: the selected agent's ONE continuous
  conversation. Header: avatar, name, status (a paused agent gets a calm
  banner with Resume — its routines don't run), the side-panel toggle and ⋯.
  Hand-offs it led show as live chips above the chat. Before the first
  message the intro shows a ready composer; the "Try asking" chips prefill it
  (no tokens are spent until you send).
-->
<script lang="ts">
  import Icon from '../../lib/components/Icon.svelte';
  import { api } from '../../lib/api/client';
  import { personalAgentsApi } from '../../lib/api/personalAgents';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { confirmRunAndDeliver } from '../../lib/confirmOutward';
  import { personalAgents } from '../../lib/stores/personalAgents.svelte';
  import { router } from '../../lib/router.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import type { PersonalAgent } from '../../lib/api/types';
  import AgentAvatar from './AgentAvatar.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import { now } from '../../lib/stores/now.svelte';
  import { routineLabel } from '../../lib/personalAgentBadges';

  const ChatLazy = lazyComponent(() => import('./AgentChat.svelte'));
  import { introLine, nextRunWords, starterPrompts } from './agentHome';

  interface Props {
    agent: PersonalAgent;
    panelOpen: boolean;
    ontogglepanel: () => void;
    ondelete: (a: PersonalAgent) => void;
  }
  let { agent, panelOpen, ontogglepanel, ondelete }: Props = $props();

  const main = $derived(!!personalAgents.autonomyById[agent.id]?.primary);
  const presence = $derived(personalAgents.presence(agent.id));
  /** A routine (or Run now) in progress — its result card lands in this chat. */
  const routineRunning = $derived(
    !!personalAgents.runningByAgent[agent.id] || !!personalAgents.runsByAgent[agent.id]?.some((r) => r.status === 'running'),
  );
  /** The routine a running run serves, by name (null for a run with none). */
  const runningName = $derived.by(() => {
    const r = personalAgents.runsByAgent[agent.id]?.find((x) => x.status === 'running');
    const s = r?.schedule_id ? (personalAgents.schedulesByAgent[agent.id] ?? []).find((x) => x.id === r.schedule_id) : undefined;
    return s ? routineLabel(s.directive) : null;
  });
  const next = $derived(personalAgents.nextRunAt(agent.id));
  /** A routine whose time has come but the scheduler's minute tick hasn't
   *  started yet (up to ~1 min) — say so, so 09:00 never looks idle. Bounded
   *  to 3 min past due so a stale next-run can't pin it. */
  const startingRoutine = $derived.by(() => {
    if (!agent.enabled) return null;
    const s = personalAgents.startingRoutine(agent.id, now());
    return s ? routineLabel(s.directive) : null;
  });
  /** Open hand-offs this agent led, plus the ones that ended in the last day. */
  const handoffs = $derived(
    (personalAgents.delegationsByAgent[agent.id] ?? [])
      .filter((d) => d.lead_agent_id === agent.id)
      .filter((d) => d.status === 'running' || Date.now() - Date.parse(d.finished_at ?? d.created_at) < 86_400_000)
      .slice(0, 8),
  );
  // An agent never chatted with opens on its intro; a chat that exists opens.
  let started = $state(false);
  let sending = $state(false);
  let draft = $state('');
  let composer = $state<HTMLTextAreaElement | undefined>();
  /** Shown at once while a first message is on its way (the chat view takes over). */
  let echo = $state('');
  $effect(() => {
    void agent.id;
    started = !!agent.chat_session_id;
  });
  const hasRoutine = $derived((personalAgents.schedulesByAgent[agent.id] ?? []).length > 0);
  $effect(() => {
    void personalAgents.loadDelegations(agent.id);
  });

  async function startWith(text: string | null): Promise<void> {
    if (sending) return;
    sending = true;
    echo = text ?? '';
    try {
      const chat = await personalAgentsApi.chatSession(agent.id);
      if (text && !chat.runtime) throw new Error('This agent’s chat is a terminal — type your message there.');
      if (text) await api.post(`/runtime-sessions/${chat.session_id}/prompt`, { text });
      draft = '';
      started = true;
    } catch (e) {
      toasts.error(`Couldn’t send it to ${agent.name}`, loadErrorText(e));
      started = !!agent.chat_session_id;
    } finally {
      sending = false;
      echo = '';
    }
  }

  async function runNow(): Promise<void> {
    if (!(await confirmRunAndDeliver(agent.name, agent.delivery))) return;
    try {
      await personalAgents.runNow(agent.id);
      toasts.success(`${agent.name} is running`, 'Its result is posted into this chat when it finishes.');
    } catch (e) {
      toasts.error(`Couldn’t run ${agent.name}`, loadErrorText(e));
    }
  }

  async function toggle(): Promise<void> {
    try {
      await personalAgents.setEnabled(agent.id, !agent.enabled);
    } catch (e) {
      toasts.error(agent.enabled ? `Couldn’t pause ${agent.name}` : `Couldn’t resume ${agent.name}`, loadErrorText(e));
    }
  }

  async function runRoutine(id: string, name: string): Promise<void> {
    if (!(await confirmRunAndDeliver(agent.name, agent.delivery))) return;
    try {
      await personalAgents.runNow(agent.id, id);
      toasts.success(`${name} is running`, 'Its result is posted into this chat when it finishes.');
    } catch (e) {
      toasts.error(`Couldn’t run ${name}`, loadErrorText(e));
    }
  }

  /** ⋯ Run: one routine → "Run <name> now"; several → pick one; none → a plain run. */
  function runItem(e: MouseEvent): { label: string; icon: 'play'; action: () => void } {
    const list = personalAgents.schedulesByAgent[agent.id] ?? [];
    if (list.length === 0) return { label: 'Run now', icon: 'play', action: () => void runNow() };
    if (list.length === 1) {
      const name = routineLabel(list[0].directive);
      return { label: `Run “${name}” now`, icon: 'play', action: () => void runRoutine(list[0].id, name) };
    }
    return {
      label: 'Run a routine now…',
      icon: 'play',
      action: () =>
        ctxMenu.show(e, list.map((r) => {
          const name = routineLabel(r.directive);
          return { label: name, icon: 'play' as const, action: () => void runRoutine(r.id, name) };
        })),
    };
  }

  function more(e: MouseEvent): void {
    ctxMenu.show(e, [
      runItem(e),
      agent.enabled
        ? ({ label: 'Pause', icon: 'pause', action: () => void toggle() } as const)
        : ({ label: 'Resume', icon: 'refresh', action: () => void toggle() } as const),
      { label: 'Settings', icon: 'gear', action: () => router.go(`personal-agents/${agent.id}/about`) },
      ...(agent.chat_session_id ? [{ label: 'Open chat in Threads', icon: 'threads' as const, action: () => router.go(`threads/${agent.chat_session_id}`) }] : []),
      { separator: true },
      { label: 'Delete…', icon: 'trash', danger: true, action: () => ondelete(agent) },
    ]);
  }

  const STATUS: Record<string, string> = { needs_you: 'Needs you', working: 'Working…', idle: 'Active' };
  const HANDOFF: Record<string, string> = { running: 'working…', done: 'done', failed: 'failed', stopped: 'stopped', interrupted: 'interrupted' };
</script>

<section class="conv" aria-label="Conversation with {agent.name}" data-testid="pa-conversation">
  <header class="conv-head">
    <AgentAvatar avatar={agent.avatar} name={agent.name} size={32} state={agent.enabled ? presence : null} />
    <div class="conv-who" data-testid="pa-conv-header">
      <span class="conv-name">
        <strong class="ellipsis" dir="auto">{agent.name}</strong>
        {#if main}<span class="conv-main" title="Your main agent — it routes your work to the others"><Icon name="star" size={12} /> Main agent</span>{/if}
      </span>
      <span class="conv-status" data-presence={presence}>
        <span class="conv-dot {agent.enabled ? presence : 'paused'}" aria-hidden="true"></span>
        {#if !agent.enabled}Paused{:else if startingRoutine && presence === 'idle'}Starting “{startingRoutine}”…{:else}{STATUS[presence]}{/if}
        {#if agent.enabled && !startingRoutine && next && Date.parse(next) > Date.now()}<span>· next routine {nextRunWords(next)}</span>{/if}
      </span>
    </div>
    <button type="button" class="icon-btn" class:pa-on={panelOpen} aria-expanded={panelOpen} aria-label="{panelOpen ? 'Hide' : 'Show'} routines and runs" title="{panelOpen ? 'Hide' : 'Show'} routines and runs" data-testid="pa-panel-toggle" onclick={ontogglepanel}><Icon name="panel" size={14} /></button>
    <button type="button" class="icon-btn" aria-label="More for {agent.name}" title="More" onclick={more}><Icon name="more" size={14} /></button>
  </header>

  {#if !agent.enabled}
    <p class="pa-paused" role="note" data-testid="pa-paused-banner">
      <Icon name="pause" size={12} />
      <span>Paused — its routines won’t run. Chat still works.</span>
      <button type="button" class="btn small" onclick={() => void toggle()}>Resume</button>
    </p>
  {/if}

  {#if started && agent.enabled && (routineRunning || startingRoutine)}
    <p class="pa-running" data-testid="pa-run-strip">
      <span class="spinner" aria-hidden="true"></span>
      <span>{routineRunning ? (runningName ? `Running “${runningName}”` : 'Running a run') : `Starting “${startingRoutine}”`} — its result lands here when it’s done.</span>
    </p>
  {/if}

  {#if handoffs.length > 0}
    <ul class="handoffs" aria-label="Work {agent.name} handed off" data-testid="pa-handoffs">
      {#each handoffs as d (d.id)}
        {@const m = personalAgents.agent(d.member_agent_id)}
        <li>
          <button type="button" class="handoff {d.status}" title="{d.member_name}: {d.task}" onclick={() => router.go(`threads/${d.id}`)}>
            <AgentAvatar avatar={m?.avatar} name={d.member_name} size={18} round />
            <span class="ellipsis" dir="auto">{d.member_name}</span>
            {#if d.status === 'running'}<span class="spinner" aria-hidden="true"></span>{/if}
            <span class="handoff-state">{HANDOFF[d.status] ?? d.status}</span>
          </button>
        </li>
      {/each}
    </ul>
  {/if}

  <div class="conv-body">
    {#if echo && !started}
      <div class="echo" data-testid="pa-echo"><p class="echo-bubble" dir="auto">{echo}</p><p class="echo-state"><span class="spinner" aria-hidden="true"></span> Sending to {agent.name}…</p></div>
    {:else if started}
      {#if ChatLazy.component}
        <ChatLazy.component agentId={agent.id} agentName={agent.name} workspaceId={agent.workspace_id} />
      {:else}
        <LoadState what="the chat" error={ChatLazy.error} loading={!ChatLazy.error} empty onretry={() => ChatLazy.retry()} />
      {/if}
    {:else}
      <div class="intro" data-testid="pa-intro">
        <AgentAvatar avatar={agent.avatar} name={agent.name} size={56} />
        {#if agent.enabled && (routineRunning || startingRoutine)}
          <!-- No chat yet: the run's result opens it here — no intro chips meanwhile. -->
          <p class="intro-line run-wait" data-testid="pa-run-wait">
            <span class="spinner" aria-hidden="true"></span>
            {routineRunning ? (runningName ? `Running “${runningName}”…` : 'Running…') : `Starting “${startingRoutine}”…`}
          </p>
          <p class="intro-sub">Its result lands here when it’s done.</p>
        {:else}
          <p class="intro-line" dir="auto">{introLine(agent.name, main)}</p>
          <div class="intro-chips" role="group" aria-label="Try asking">
            {#each starterPrompts(main, hasRoutine, !agent.enabled) as p (p)}
              <button type="button" class="chip-btn" disabled={sending} onclick={() => { draft = p; composer?.focus(); }}>{p}</button>
            {/each}
          </div>
        {/if}
      </div>
      <!-- The same box, size and send button as the chat's own composer, so
           nothing jumps when the first message opens the chat. -->
      <div class="dock">
        <form class="composer" onsubmit={(e) => { e.preventDefault(); if (draft.trim()) void startWith(draft.trim()); }}>
          <textarea
            bind:this={composer}
            class="composer-input"
            rows="2"
            dir="auto"
            aria-label="Message {agent.name}"
            placeholder="Message {agent.name}…"
            data-testid="pa-composer"
            bind:value={draft}
            onkeydown={(e) => { if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) { e.preventDefault(); if (draft.trim()) void startWith(draft.trim()); } }}
          ></textarea>
          <div class="composer-foot">
            <button type="submit" class="send" disabled={sending || !draft.trim()} aria-label="Send" title="Send (Enter)" data-testid="pa-start-chat"><Icon name="arrowUp" size={14} /></button>
          </div>
        </form>
      </div>
    {/if}
  </div>
</section>

<style>
  /* Phone: toasts sit above this chat's composer, not on top of it. */
  @media (max-width: 640px) {
    :global(:root:has([data-testid='pa-conversation'])) {
      --toast-lift-phone: 104px;
    }
  }
  .conv { flex: 1; min-inline-size: 0; min-block-size: 0; display: flex; flex-direction: column; }
  .conv-head { display: flex; align-items: center; gap: 8px; padding: 8px 12px; border-block-end: 1px solid var(--border); flex: none; }
  .conv-who { flex: 1; min-inline-size: 0; display: flex; flex-direction: column; }
  .pa-on { color: var(--accent-text); }
  .pa-paused {
    display: flex; align-items: center; gap: 8px; margin: 0; padding: 6px 12px; flex: none;
    /* A neutral surface with an amber edge: 14% amber over the dark theme read as muddy khaki. */
    background: var(--surface-2); color: var(--text); font-size: var(--fs-s); border-block-end: 1px solid var(--border);
    border-inline-start: 3px solid var(--warning);
  }
  .pa-paused > :global(svg) { color: var(--warning); flex: none; }
  .pa-paused > span { flex: 1; min-inline-size: 0; }
  /* Mirrors the chat composer (threads/runtime/RuntimeThread.svelte .rt-dock / .rt-composer / .rt-send). */
  .dock { inline-size: 100%; max-inline-size: 760px; margin-inline: auto; padding-inline: 16px; padding-block: 0 12px; box-sizing: border-box; }
  .composer {
    display: flex; flex-direction: column; gap: 6px; padding: 10px 12px 8px;
    border: 1px solid var(--border); border-radius: var(--radius-l); background: var(--surface);
  }
  .composer:focus-within { border-color: var(--border-strong); }
  .composer-input {
    inline-size: 100%; min-block-size: 44px; max-block-size: 220px; resize: none; border: 0; padding: 0;
    background: transparent; color: var(--text); font: inherit; font-size: var(--fs-m); line-height: 1.55; outline: none; field-sizing: content;
  }
  .composer-foot { display: flex; justify-content: flex-end; }
  .send {
    display: grid; place-items: center; flex-shrink: 0; inline-size: 30px; block-size: 30px; border: 0; border-radius: 50%;
    background: var(--accent-solid); color: var(--accent-contrast); cursor: pointer;
  }
  .send:disabled { background: var(--surface-3); color: var(--text-dim); cursor: default; }
  .send:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
  .conv-name { display: flex; align-items: center; gap: 8px; min-inline-size: 0; font-size: var(--fs-l); }
  .conv-main { display: inline-flex; align-items: center; gap: 4px; font-size: var(--fs-xs); color: var(--accent-text); white-space: nowrap; }
  .conv-status { display: flex; align-items: center; gap: 6px; font-size: var(--fs-s); color: var(--text-dim); }
  .conv-dot { inline-size: 8px; block-size: 8px; border-radius: 50%; background: var(--text-dim); }
  .conv-dot.idle { background: var(--success); }
  .conv-dot.working { background: var(--info); }
  .conv-dot.needs_you { background: var(--warning); }
  .handoffs { list-style: none; margin: 0; padding: 6px 12px; display: flex; gap: 6px; flex-wrap: wrap; border-block-end: 1px solid var(--border); flex: none; }
  .handoff {
    display: inline-flex; align-items: center; gap: 6px; max-inline-size: 260px; padding-block: 2px; padding-inline: 4px 10px; border-radius: 999px;
    border: 1px solid var(--border); background: var(--surface); color: var(--text); font: inherit; font-size: var(--fs-s); cursor: pointer;
  }
  .handoff:hover { border-color: var(--border-strong); }
  .handoff-state { color: var(--text-dim); white-space: nowrap; }
  .handoff.done .handoff-state { color: var(--success); }
  .handoff.failed .handoff-state, .handoff.interrupted .handoff-state { color: var(--danger); }
  .handoff.stopped .handoff-state { color: var(--warning); }
  .conv-body { flex: 1; min-block-size: 0; display: flex; flex-direction: column; padding: 0; }
  .echo { margin-block-start: auto; padding: 16px; display: flex; flex-direction: column; align-items: flex-end; gap: 6px; max-inline-size: 760px; inline-size: 100%; margin-inline: auto; box-sizing: border-box; }
  .echo-bubble { margin: 0; padding: 8px 12px; border-radius: var(--radius-l); background: var(--accent-soft); color: var(--text); font-size: var(--fs-m); white-space: pre-wrap; max-inline-size: 80%; }
  .echo-state { margin: 0; display: flex; align-items: center; gap: 6px; color: var(--text-dim); font-size: var(--fs-s); }
  .intro { margin-inline: auto; }
  .intro { margin: auto auto 16px; max-inline-size: 520px; display: flex; flex-direction: column; align-items: center; gap: 14px; text-align: center; padding: 24px 8px; }
  .intro-line { margin: 0; font-size: var(--fs-l); line-height: 1.5; color: var(--text); }
  .run-wait { display: inline-flex; align-items: center; gap: 8px; }
  .intro-sub { margin: 0; color: var(--text-dim); font-size: var(--fs-s); }
  .intro-chips { display: flex; flex-wrap: wrap; justify-content: center; gap: 8px; }
  .chip-btn {
    border: 1px solid var(--border); border-radius: 999px; background: var(--surface); color: var(--text);
    padding: 6px 12px; font: inherit; font-size: var(--fs-s); cursor: pointer; text-align: start;
  }
  /* Hover only outlines (a filled chip read as pre-selected under a resting pointer). */
  @media (hover: hover) { .chip-btn:hover { border-color: var(--accent-line); } }
  .chip-btn:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 1px; }
  @media (max-width: 640px) {
    .conv-body { padding: 8px; }
    .conv-head { padding-block: 6px; }
    .conv-main { display: none; }
  }
  .pa-running {
    display: flex; align-items: center; gap: 8px; margin: 0; padding: 6px 12px; flex: none;
    background: var(--info-soft); color: var(--text); font-size: var(--fs-s); border-block-end: 1px solid var(--border);
  }
</style>
