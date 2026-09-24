<script lang="ts">
  // The Chat tab: the thread as a conversation, NOT a terminal. Messages are
  // rendered by the Conversation view's own pieces (TurnItem + the transcript
  // store under the same CONV_CTX): the thread's turn index is the backbone
  // (it spans provider hand-offs), and replies of the current CLI session come
  // from its transcript with their tool steps and images (mergeWithTranscript).
  // Every action the assistant takes sits between the messages as a card
  // (threadCards → buildTimeline); each reply carries who wrote it.
  import { setContext, tick, untrack } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import ProviderIcon from '../../lib/components/ProviderIcon.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import StatusDot from '../../lib/components/StatusDot.svelte';
  import TurnItem from '../agents/conversation/TurnItem.svelte';
  import { groupTurns } from '../agents/conversation/format';
  import { CONV_CTX, type ConvContext } from '../agents/conversation/context';
  import { transcript } from '../../lib/stores/transcript.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { assistant } from '../../lib/stores/assistant.svelte';
  import { buildTimeline, providerLabel, threadCards, type ChatCard } from './model';
  import { mergeWithTranscript, messageAuthor, type ChatMessage } from './chat';
  import { clock } from './format';
  import AssistantComposer from './AssistantComposer.svelte';
  import MemoryChips from './cards/MemoryChips.svelte';
  import TaskEntry from './cards/TaskEntry.svelte';
  import SystemLine from './cards/SystemLine.svelte';
  import type { AssistantThread } from '../../lib/api/types';

  interface Props {
    thread: AssistantThread;
    onopentasks: () => void;
    onopenmemory: () => void;
  }
  let { thread, onopentasks, onopenmemory }: Props = $props();

  // ── data ───────────────────────────────────────────────────────────────────
  $effect(() => {
    const id = thread.id;
    untrack(() => void assistant.loadTurns(id));
  });
  $effect(() => {
    if (untrack(() => assistant.tasks.state) === 'idle') void assistant.loadTasks();
  });
  const index = $derived(assistant.turns[thread.id]);
  const turns = $derived(index?.data ?? []);

  // The transcript of the thread's current CLI session (when there is one).
  const src = $derived(thread.session_id ? { sessionId: thread.session_id } : null);
  const conv = $derived(src ? transcript.conversation(src) : null);
  $effect(() => {
    const s = src;
    if (!s) return;
    void conv;
    return untrack(() => transcript.acquireView(s));
  });

  // Context for the Conversation-view pieces (images, tool steps, subagents).
  const ctx: ConvContext = $state(
    untrack(() => ({
      conv: transcript.conversation(src ?? { sessionId: thread.id }),
      sessionId: thread.session_id,
      readonly: true,
      provider: thread.provider === 'codex' ? ('codex' as const) : ('claude' as const),
      queuedLive: [],
    })),
  );
  setContext(CONV_CTX, ctx);
  $effect(() => {
    if (conv) ctx.conv = conv;
    ctx.sessionId = thread.session_id;
    ctx.provider = conv?.transcript?.provider ?? (thread.provider === 'codex' ? 'codex' : 'claude');
  });

  const live = $derived(conv ? groupTurns(conv.turns) : []);
  const messages = $derived(mergeWithTranscript(turns, live, thread.session_id));
  const cards = $derived(threadCards(turns, assistant.tasks.data, thread.id));
  const timeline = $derived(
    buildTimeline(
      messages.map((m) => ({ id: m.item.id, role: m.item.role, ts: m.item.ts, m })),
      cards,
    ),
  );
  const pending = $derived(assistant.pending[thread.id] ?? []);

  const sessionStatus = $derived(thread.session_id ? (ws.statusMap[thread.session_id] ?? null) : null);
  const working = $derived(thread.status === 'working' || sessionStatus === 'working' || pending.length > 0);

  const loading = $derived(!index || (index.state === 'loading' && !index.data.length));
  const failed = $derived(index?.state === 'error' && !index.data.length);
  const empty = $derived(!loading && !failed && timeline.length === 0 && pending.length === 0);

  function author(m: ChatMessage): { provider: string; label: string } {
    const a = messageAuthor(m, thread);
    return { provider: a.provider, label: providerLabel(a.provider, a.model) };
  }
  function cardKey(c: ChatCard): string {
    return `c:${c.id}`;
  }

  // ── scroll: follow the tail unless the reader scrolled up ──────────────────
  let listEl = $state<HTMLDivElement | null>(null);
  let atBottom = $state(true);
  function onScroll(): void {
    const el = listEl;
    if (el) atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 48;
  }
  function toBottom(): void {
    const el = listEl;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
    atBottom = true;
  }
  let paintedFor = '';
  $effect(() => {
    const n = timeline.length + pending.length + (working ? 1 : 0);
    if (loading) return;
    const first = paintedFor !== thread.id;
    paintedFor = thread.id;
    if (first || untrack(() => atBottom)) void tick().then(toBottom);
    void n;
  });
</script>

<div class="chat">
  <div class="scroll" bind:this={listEl} onscroll={onScroll} data-testid="assistant-thread">
    <div class="col">
      {#if loading}
        <div class="state" aria-busy="true" aria-label="Loading the conversation">
          <Skeleton rows={4} height={44} />
        </div>
      {:else if failed}
        <div class="error" role="alert">
          <Icon name="warning" size={14} />
          <div class="error-t">
            <strong>Couldn’t load this conversation.</strong>
            <span class="dim">{index?.error}</span>
          </div>
          <button class="btn small" onclick={() => void assistant.loadTurns(thread.id)}>Retry</button>
        </div>
      {:else if empty}
        <EmptyState icon="assistant" title="Nothing here yet" body="Ask Otto anything — plan, research, remember, remind. Every action shows up here, and it asks before anything leaves your Mac." />
      {:else}
        {#each timeline as entry (entry.kind === 'turn' ? `t:${entry.turn.id}` : cardKey(entry.card))}
          {#if entry.kind === 'turn'}
            {@const m = entry.turn.m}
            {#if m.item.role === 'assistant'}
              {@const w = author(m)}
              <div class="msg agent" data-testid="assistant-message">
                <div class="who">
                  <span class="avatar" aria-hidden="true"><Icon name="assistant" size={12} /></span>
                  <strong>Otto</strong>
                  <span class="badge" data-testid="provider-badge" title={`Written by ${w.label}`}><ProviderIcon provider={w.provider} size={12} />{w.label}</span>
                  {#if m.item.ts}<span class="dim">· <time datetime={m.item.ts} title={new Date(m.item.ts).toLocaleString()}>{clock(m.item.ts)}</time></span>{/if}
                </div>
                <MemoryChips cards={entry.memory} onreview={onopenmemory} />
                <TurnItem item={m.item} />
              </div>
            {:else}
              <div class="msg user">
                <TurnItem item={m.item} />
                {#if m.turn?.attachments.length}
                  <ul class="atts" aria-label="Attachments">
                    {#each m.turn.attachments as a (a.id)}
                      <li class="att" title={a.path}><Icon name="file" size={12} />{a.name}</li>
                    {/each}
                  </ul>
                {/if}
              </div>
            {/if}
          {:else}
            {@const c = entry.card}
            <div class="entry">
              {#if c.kind === 'memory'}
                <MemoryChips cards={[c]} onreview={onopenmemory} />
              {:else if c.kind === 'task'}
                <TaskEntry taskId={c.task_id} turn={c.turn} {onopentasks} />
              {:else}
                <SystemLine turn={c.turn} />
              {/if}
            </div>
          {/if}
        {/each}
        {#each pending as p (p.id)}
          <div class="pending-bubble" data-testid="pending-turn">
            <div class="bubble-text">{p.text}</div>
            <span class="dim">Sending…</span>
          </div>
        {/each}
        {#if working}
          <p class="working" role="status">
            <StatusDot status="working" size={7} /> Otto is working…
          </p>
        {/if}
      {/if}
    </div>
  </div>
  {#if !atBottom && !loading}
    <button class="jump btn small" onclick={toBottom}><Icon name="arrowDown" size={12} /> Jump to latest</button>
  {/if}
  <AssistantComposer {thread} busy={thread.status === 'working'} />
</div>

<style>
  .chat {
    position: relative;
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
    min-width: 0;
  }
  .scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
  }
  .col {
    max-width: 820px;
    padding: 18px 20px 12px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  /* The Conversation-view pieces bring their own padding and a tinted card
     for replies; in the assistant thread a reply is plain prose under an
     attribution line, marked by the agent rule (patterns §2). */
  .col :global(.turn) {
    padding: 0;
  }
  .msg {
    min-width: 0;
  }
  .msg.user {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 4px;
  }
  .msg.user > :global(.turn) {
    width: 100%;
  }
  .msg.agent {
    border-inline-start: 2px solid var(--border-strong);
    padding-inline-start: 12px;
  }
  .msg.agent :global(.turn.assistant .resp) {
    background: none;
    border: 0;
    padding: 0;
    border-radius: 0;
  }
  .msg.agent :global(.turn.assistant .meta .ts),
  .msg.agent :global(.turn.assistant .meta .model) {
    display: none;
  }
  .who {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    margin-bottom: 6px;
    font-size: var(--fs-s);
  }
  .who strong {
    font-weight: 600;
  }
  .avatar {
    width: 18px;
    height: 18px;
    border-radius: var(--radius-s);
    display: inline-grid;
    place-items: center;
    background: var(--surface-2);
    border: 1px solid var(--border);
    color: var(--text);
  }
  .badge {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 20px;
    padding: 0 8px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text-dim);
    font-size: var(--fs-xs);
    font-weight: 500;
  }
  .atts {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: 6px;
  }
  .att {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 22px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    font-size: var(--fs-xs);
  }
  .dim {
    color: var(--text-dim);
  }
  .entry {
    min-width: 0;
  }
  .state {
    padding: 8px 0;
  }
  .error {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
  }
  .error > :global(svg) {
    color: var(--danger);
    margin-top: 2px;
  }
  .error-t {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    font-size: var(--fs-s);
  }
  .pending-bubble {
    align-self: flex-end;
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 4px;
    max-width: min(78%, 720px);
    font-size: var(--fs-xs);
  }
  .bubble-text {
    background: var(--accent-soft);
    border-radius: 14px 14px 4px 14px;
    padding: 8px 12px;
    font-size: var(--fs-m);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  :global([dir='rtl']) .bubble-text {
    border-radius: 14px 14px 14px 4px;
  }
  .working {
    margin: 0;
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .jump {
    position: absolute;
    inset-inline-start: 50%;
    transform: translateX(-50%);
    bottom: 84px;
    box-shadow: var(--shadow);
  }
  :global([dir='rtl']) .jump {
    transform: translateX(50%);
  }
  @media (max-width: 640px) {
    .col {
      padding: 12px 12px 8px;
    }
  }
</style>
