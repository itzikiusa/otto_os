<!--
  The center of the Personal Agents home: the selected agent's ONE continuous
  conversation. Its header (avatar, name, live status) opens the side panel;
  hand-offs it led show as live chips above the chat (working / done / failed,
  each opens its thread); before the first message an intro with "Try
  asking" chips stands in (no tokens are spent until one is picked).
-->
<script lang="ts">
  import Icon from '../../lib/components/Icon.svelte';
  import RelTime from '../../lib/components/RelTime.svelte';
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

  const ChatLazy = lazyComponent(() => import('./AgentChat.svelte'));
  import { introLine, starterPrompts } from './agentHome';

  interface Props {
    agent: PersonalAgent;
    panelOpen: boolean;
    ontogglepanel: () => void;
    ondelete: (a: PersonalAgent) => void;
  }
  let { agent, panelOpen, ontogglepanel, ondelete }: Props = $props();

  const main = $derived(!!personalAgents.autonomyById[agent.id]?.primary);
  const presence = $derived(personalAgents.presence(agent.id));
  const next = $derived(personalAgents.nextRunAt(agent.id));
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
  $effect(() => {
    void agent.id;
    started = !!agent.chat_session_id;
  });
  $effect(() => {
    void personalAgents.loadDelegations(agent.id);
  });

  async function startWith(text: string | null): Promise<void> {
    if (sending) return;
    sending = true;
    try {
      const chat = await personalAgentsApi.chatSession(agent.id);
      if (text && chat.runtime) await api.post(`/runtime-sessions/${chat.session_id}/prompt`, { text });
      started = true;
    } catch (e) {
      toasts.error(`Couldn’t start the chat with ${agent.name}`, loadErrorText(e));
    } finally {
      sending = false;
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

  function more(e: MouseEvent): void {
    ctxMenu.show(e, [
      { label: 'Details', icon: 'info', action: ontogglepanel },
      { label: 'Run now', icon: 'play', action: () => void runNow() },
      { label: agent.enabled ? 'Pause routines' : 'Resume routines', icon: agent.enabled ? 'pause' : 'play', action: () => void toggle() },
      ...(agent.chat_session_id ? [{ label: 'Open chat in Threads', icon: 'threads' as const, action: () => router.go(`threads/${agent.chat_session_id}`) }] : []),
      { label: 'All settings', icon: 'gear', action: () => router.go(`personal-agents/${agent.id}/overview`) },
      { separator: true },
      { label: 'Delete…', icon: 'trash', danger: true, action: () => ondelete(agent) },
    ]);
  }

  const STATUS: Record<string, string> = { needs_you: 'Needs you', working: 'Working…', idle: 'Idle' };
  const HANDOFF: Record<string, string> = { running: 'working…', done: 'done', failed: 'failed', stopped: 'stopped', interrupted: 'interrupted' };
</script>

<section class="conv" aria-label="Conversation with {agent.name}" data-testid="pa-conversation">
  <header class="conv-head">
    <button type="button" class="conv-who" aria-expanded={panelOpen} title="{panelOpen ? 'Hide' : 'Show'} {agent.name}’s details" data-testid="pa-conv-header" onclick={ontogglepanel}>
      <AgentAvatar avatar={agent.avatar} name={agent.name} size={32} state={agent.enabled ? presence : null} />
      <span class="conv-who-t">
        <span class="conv-name">
          <strong class="ellipsis" dir="auto">{agent.name}</strong>
          {#if main}<span class="conv-main" title="Your main agent — it routes your work to the others"><Icon name="star" size={12} /> Main agent</span>{/if}
        </span>
        <span class="conv-status" data-presence={presence}>
          <span class="conv-dot {agent.enabled ? presence : 'paused'}" aria-hidden="true"></span>
          {agent.enabled ? STATUS[presence] : 'Paused'}
          {#if agent.enabled && next} · next routine <RelTime iso={next} />{/if}
        </span>
      </span>
      <Icon name="chevronRight" size={14} />
    </button>
    <button type="button" class="icon-btn" aria-label="More for {agent.name}" title="More" onclick={more}><Icon name="more" size={14} /></button>
  </header>

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
    {#if started}
      {#if ChatLazy.component}
        <ChatLazy.component agentId={agent.id} agentName={agent.name} workspaceId={agent.workspace_id} />
      {:else}
        <LoadState what="the chat" error={ChatLazy.error} loading={!ChatLazy.error} empty onretry={() => ChatLazy.retry()} />
      {/if}
    {:else}
      <div class="intro" data-testid="pa-intro">
        <AgentAvatar avatar={agent.avatar} name={agent.name} size={64} />
        <p class="intro-line" dir="auto">{introLine(agent.name, main)}</p>
        <div class="intro-chips" role="group" aria-label="Try asking">
          {#each starterPrompts(main) as p (p)}
            <button type="button" class="chip-btn" disabled={sending} onclick={() => void startWith(p)}>{p}</button>
          {/each}
        </div>
        <button type="button" class="btn primary" disabled={sending} data-testid="pa-start-chat" onclick={() => void startWith(null)}>
          <Icon name="comment" size={12} /> {sending ? 'Opening…' : `Message ${agent.name}`}
        </button>
      </div>
    {/if}
  </div>
</section>

<style>
  .conv { flex: 1; min-inline-size: 0; min-block-size: 0; display: flex; flex-direction: column; }
  .conv-head { display: flex; align-items: center; gap: 8px; padding: 8px 12px; border-block-end: 1px solid var(--border); flex: none; }
  .conv-who {
    flex: 1; min-inline-size: 0; display: flex; align-items: center; gap: 10px; padding: 4px 6px; margin-inline-start: -6px;
    border: 0; border-radius: var(--radius-m); background: transparent; color: var(--text); font: inherit; text-align: start; cursor: pointer;
  }
  .conv-who:hover { background: var(--hover); }
  .conv-who:focus-visible { outline: 2px solid var(--accent-text); outline-offset: -2px; }
  .conv-who > :global(svg) { color: var(--text-dim); margin-inline-start: auto; }
  .conv-who-t { display: flex; flex-direction: column; min-inline-size: 0; }
  .conv-name { display: flex; align-items: center; gap: 8px; min-inline-size: 0; font-size: var(--fs-l); }
  .conv-main { display: inline-flex; align-items: center; gap: 4px; font-size: var(--fs-xs); color: var(--accent-text); white-space: nowrap; }
  .conv-status { display: flex; align-items: center; gap: 6px; font-size: var(--fs-s); color: var(--text-dim); }
  .conv-dot { inline-size: 8px; block-size: 8px; border-radius: 50%; background: var(--text-dim); }
  .conv-dot.working { background: var(--success); }
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
  .conv-body { flex: 1; min-block-size: 0; display: flex; flex-direction: column; padding: 12px; }
  .intro { margin: auto; max-inline-size: 520px; display: flex; flex-direction: column; align-items: center; gap: 14px; text-align: center; padding: 24px 8px; }
  .intro-line { margin: 0; font-size: var(--fs-l); line-height: 1.5; color: var(--text); }
  .intro-chips { display: flex; flex-wrap: wrap; justify-content: center; gap: 8px; }
  .chip-btn {
    border: 1px solid var(--border); border-radius: 999px; background: var(--surface); color: var(--text);
    padding: 6px 12px; font: inherit; font-size: var(--fs-s); cursor: pointer; text-align: start;
  }
  .chip-btn:hover { border-color: var(--accent-line); background: var(--accent-soft); }
  .chip-btn:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 1px; }
  @media (max-width: 640px) {
    .conv-body { padding: 8px; }
  }
</style>
