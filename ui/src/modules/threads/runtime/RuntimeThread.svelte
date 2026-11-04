<script lang="ts">
  // A Threads conversation on the structured agent runtime: the timeline
  // (your messages, the agent's streaming answer, a quiet work log of
  // reasoning / commands / edits / tools with approvals inline) over one
  // composer whose send turns into Stop while a turn runs; a message sent
  // meanwhile is queued and goes out when the turn ends.
  // Ported from T3 Code (MIT, Copyright (c) 2026 T3 Tools Inc.):
  // apps/web/src/components/chat/MessagesTimeline.tsx + ThreadTimeline.tsx
  // (messages with work-log groups between them, follow the bottom),
  // ComposerPrimaryActions.tsx (send ↔ stop), QueuedRunsControl.tsx (one
  // queued follow-up), ThreadStatusLine.tsx ("Working for 12s"),
  // ThreadErrorBanner.tsx, and apps/web/src/session-logic.ts (`derivePhase`).
  import { tick, untrack } from 'svelte';
  import { announceShown } from '../../../lib/announce.svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../../lib/components/ProviderIcon.svelte';
  import { harnessLabel } from '../../../lib/providers';
  import { now } from '../../../lib/stores/now.svelte';
  import { toastError } from '../../../lib/toastError';
  import type { ApprovalDecision } from '../../../lib/api/types';
  import Markdown from '../../agents/conversation/Markdown.svelte'; // ui-guards: allow — shared chat markdown
  import { isRunning, latestTurn, pendingApprovals, timelineRows } from './projection';
  import RuntimeApprovalRow from './RuntimeApprovalRow.svelte';
  import RuntimeWorkRow from './RuntimeWorkRow.svelte';
  import { runtimeThreads } from './runtimeThreads.svelte';

  interface Props {
    sessionId: string;
    focused: boolean;
  }
  let { sessionId, focused }: Props = $props();

  $effect(() => {
    const id = sessionId;
    return untrack(() => runtimeThreads.open(id));
  });

  const session = $derived(runtimeThreads.get(sessionId));
  const p = $derived(runtimeThreads.projections[sessionId] ?? null);
  const loadError = $derived(runtimeThreads.loadError[sessionId] ?? null);
  const rows = $derived(p ? timelineRows(p.items) : []);
  const approvals = $derived(new Map((p ? pendingApprovals(p) : []).map((a) => [a.requestId, a] as const)));
  const turn = $derived(p ? latestTurn(p) : null);
  const closed = $derived(session?.status === 'closed');
  const sessionError = $derived(p?.session?.status === 'error' ? p.session.last_error : null);
  const plan = $derived.by(() => {
    if (!p || !turn) return null;
    const pl = p.plans.findLast((x) => x.turn_id === turn.id);
    return pl && pl.steps.some((s) => s.status !== 'completed') ? pl : null;
  });

  let text = $state('');
  let queued = $state<string | null>(null);
  let sending = $state(false);
  /** The accepted prompt's first sequence, until the projection reaches it. */
  let awaitSeq = $state<number | null>(null);
  let stopping = $state(false);
  let scroller = $state<HTMLDivElement | null>(null);
  let composerEl = $state<HTMLTextAreaElement | null>(null);
  let atBottom = true;

  const running = $derived((p ? isRunning(p) : false) || sending || (awaitSeq !== null && (p?.lastSequence ?? 0) < awaitSeq));
  const workingFor = $derived.by(() => {
    if (!running || !turn?.started_at || turn.status !== 'running') return null;
    const s = Math.max(0, Math.floor((now() - Date.parse(turn.started_at)) / 1000));
    return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m ${s % 60}s`;
  });

  async function send(t: string): Promise<void> {
    sending = true;
    try {
      awaitSeq = (await runtimeThreads.prompt(sessionId, t)).sequence;
    } catch (e) {
      toastError('Couldn’t send message', e);
      if (!text) text = t;
    } finally {
      sending = false;
    }
  }

  function submit(): void {
    const t = text.trim();
    if (!t || closed) return;
    text = '';
    if (running) queued = t;
    else void send(t);
  }

  // The queued follow-up goes out once the turn has ended.
  $effect(() => {
    if (!running && queued) {
      const q = queued;
      queued = null;
      void send(q);
    }
  });

  async function stop(): Promise<void> {
    if (stopping) return;
    stopping = true;
    try {
      await runtimeThreads.interrupt(sessionId);
    } catch (e) {
      toastError('Couldn’t stop the turn', e);
    } finally {
      stopping = false;
    }
  }

  async function respond(requestId: string, decision: ApprovalDecision): Promise<void> {
    try {
      await runtimeThreads.respond(sessionId, requestId, decision);
    } catch (e) {
      toastError('Couldn’t answer the approval', e);
    }
  }

  function onKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      submit();
    }
  }

  // Follow the bottom while the reader is there (T3's scroll anchoring).
  function onScroll(): void {
    if (scroller) atBottom = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 40;
  }
  $effect(() => {
    void p?.lastSequence;
    if (!atBottom) return;
    void tick().then(() => scroller?.scrollTo({ top: scroller.scrollHeight }));
  });
  $effect(() => {
    if (focused && p) composerEl?.focus({ preventScroll: true });
  });
</script>

<section class="rt" aria-label="Conversation" data-testid="runtime-thread-{sessionId}">
  <div class="rt-scroll" bind:this={scroller} onscroll={onScroll}>
    <div class="rt-col">
      <LoadState what="conversation" loading={!p && !loadError} error={p ? null : loadError} empty={!p} onretry={() => runtimeThreads.retry(sessionId)}>
        {#if rows.length === 0}
          <EmptyState
            icon="threads"
            title="Start the conversation"
            body="Replies, tool calls and approvals stream here as the agent works."
          />
        {/if}
        {#each rows as row (row.kind === 'message' ? row.item.id : row.id)}
          {#if row.kind === 'message' && row.item.type === 'user_message'}
            <div class="rt-user"><div class="rt-bubble" dir="auto">{row.item.text}</div></div>
          {:else if row.kind === 'message' && row.item.type === 'assistant_message'}
            <div class="rt-assistant" class:streaming={row.item.streaming}>
              <Markdown md={row.item.text} />
            </div>
          {:else if row.kind === 'work'}
            <div class="rt-work">
              {#each row.items as item (item.id)}
                {@const pending = item.type === 'approval_request' ? approvals.get(item.request_id) : undefined}
                {#if pending}
                  <RuntimeApprovalRow approval={pending} onrespond={respond} />
                {:else}
                  <RuntimeWorkRow {item} output={p?.output[item.id]} />
                {/if}
              {/each}
            </div>
          {/if}
        {/each}
        {#if workingFor}
          <p class="rt-status"><span class="spinner" aria-hidden="true"></span>Working for {workingFor}</p>
        {:else if turn && !running && turn.status === 'interrupted'}
          <p class="rt-status">Stopped</p>
        {/if}
      </LoadState>
    </div>
  </div>

  <div class="rt-dock">
    {#if sessionError}
      <p class="rt-banner" dir="auto" use:announceShown={{ text: sessionError, assertive: true }}><Icon name="warning" size={13} />{sessionError}</p>
    {/if}
    {#if plan}
      <ol class="rt-plan" aria-label="Plan">
        {#each plan.steps as s (s.id)}
          <li class="step-{s.status}" dir="auto"><Icon name={s.status === 'completed' ? 'check' : 'dot'} size={12} />{s.text}</li>
        {/each}
      </ol>
    {/if}
    {#if queued}
      <div class="rt-queued" data-testid="runtime-queued">
        <span class="ellipsis" dir="auto">Queued: {queued}</span>
        <button type="button" class="btn small" onclick={() => ((text = queued ?? ''), (queued = null))}>Edit</button>
      </div>
    {/if}
    <div class="rt-composer">
      <textarea
        bind:this={composerEl}
        bind:value={text}
        dir="auto"
        rows={2}
        disabled={closed}
        placeholder={closed
          ? 'This thread is closed'
          : approvals.size > 0
            ? 'Waiting for your approval above'
            : running
              ? 'Queue a follow-up for when this turn ends'
              : 'Ask anything'}
        aria-label="Message"
        data-testid="runtime-composer"
        onkeydown={onKeydown}
      ></textarea>
      <div class="rt-foot">
        <span class="rt-agent">
          {#if session && hasProviderIcon(session.provider)}<ProviderIcon provider={session.provider} size={13} />{/if}
          <span class="ellipsis">{session ? harnessLabel(session.provider) : ''}{session?.model ? ` · ${session.model}` : ''}</span>
        </span>
        {#if running && !text.trim()}
          <button
            type="button"
            class="rt-send stop"
            aria-label="Stop the turn"
            title="Stop"
            data-testid="runtime-stop"
            disabled={stopping}
            onclick={() => void stop()}
          >
            <Icon name="stop" size={14} />
          </button>
        {:else}
          <button
            type="button"
            class="rt-send"
            aria-label={running ? 'Queue message' : 'Send message'}
            title={running ? 'Queue (↵)' : 'Send (↵)'}
            data-testid="runtime-send"
            disabled={closed || !text.trim()}
            onclick={submit}
          >
            <Icon name="arrowUp" size={16} />
          </button>
        {/if}
      </div>
    </div>
  </div>
</section>

<style>
  .rt {
    display: flex;
    flex-direction: column;
    min-block-size: 0;
    block-size: 100%;
    background: var(--bg);
  }
  .rt-scroll {
    flex: 1;
    min-block-size: 0;
    overflow-y: auto;
    overflow-x: hidden;
  }
  .rt-col,
  .rt-dock {
    inline-size: 100%;
    max-inline-size: 760px;
    margin-inline: auto;
    padding-inline: 16px;
  }
  .rt-col {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding-block: 16px 24px;
  }
  .rt-user {
    display: flex;
    justify-content: flex-end;
    padding-block-start: 10px;
  }
  .rt-bubble {
    max-inline-size: min(80%, 78ch);
    padding: 8px 14px;
    border: 1px solid color-mix(in srgb, var(--accent) 34%, var(--border));
    border-radius: var(--radius-l);
    background: color-mix(in srgb, var(--accent) 16%, var(--surface));
    color: var(--text);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    unicode-bidi: plaintext;
  }
  .rt-assistant {
    min-width: 0;
    color: var(--text);
  }
  .rt-assistant.streaming :global(.md > :last-child)::after {
    content: '▍';
    margin-inline-start: 2px;
    color: var(--text-dim);
  }
  .rt-work {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .rt-status {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 4px 0 0;
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .rt-status .spinner {
    --spinner-size: 11px;
  }
  .rt-dock {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding-block: 0 12px;
  }
  .rt-banner {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    padding: 6px 10px;
    border-radius: var(--radius-m);
    background: var(--danger-soft);
    color: var(--danger);
    font-size: var(--fs-s);
    overflow-wrap: anywhere;
  }
  .rt-plan {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: 6px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    color: var(--text-dim);
    font-size: var(--fs-s);
    list-style: none;
  }
  .rt-plan li {
    display: flex;
    align-items: center;
    gap: 6px;
    overflow-wrap: anywhere;
  }
  .rt-plan .step-running {
    color: var(--text);
  }
  .rt-plan .step-completed {
    color: var(--success);
  }
  .rt-queued {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    padding-block: 4px;
    padding-inline: 10px 6px;
    border-radius: var(--radius-m);
    background: var(--surface-2);
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .rt-queued .ellipsis {
    flex: 1;
    min-width: 0;
  }
  .rt-composer {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    background: var(--surface);
  }
  .rt-composer:focus-within {
    border-color: var(--border-strong);
  }
  .rt-composer textarea {
    inline-size: 100%;
    min-block-size: 44px;
    max-block-size: 220px;
    resize: none;
    border: 0;
    padding: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
    line-height: 1.55;
    unicode-bidi: plaintext;
    text-align: start;
    outline: none;
    field-sizing: content;
  }
  .rt-foot {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .rt-agent {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    flex: 1;
    min-width: 0;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .rt-send {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    inline-size: 30px;
    block-size: 30px;
    border: 0;
    border-radius: 50%;
    background: var(--accent-solid);
    color: var(--accent-contrast);
    cursor: pointer;
  }
  .rt-send.stop {
    background: var(--text);
    color: var(--bg);
  }
  .rt-send:disabled {
    background: var(--surface-3);
    color: var(--text-dim);
    cursor: default;
  }
</style>
