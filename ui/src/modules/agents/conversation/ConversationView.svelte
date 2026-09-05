<script lang="ts" module>
  /** Exactly one of `sessionId` / `transcriptPath`; the latter reads the
   *  workspace history route (read-only, `on_disk` entries). */
  export interface ConversationViewProps {
    sessionId?: string;
    transcriptPath?: string;
    workspaceId: string;
    readonly?: boolean;
  }
</script>

<script lang="ts">
  // The agent session as a Claude/Codex-app-style conversation, rebuilt from
  // the provider's transcript on disk (docs/design/conversation-view.md §5.2).
  // Newest page first + "Load earlier" (scroll-anchored), auto-follow at the
  // bottom with a "↓ new" pill otherwise, live tail via `transcript_appended`.
  import { setContext, tick, untrack } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../../lib/components/ProviderIcon.svelte';
  import TurnItem from './TurnItem.svelte';
  import Composer from './Composer.svelte';
  import { transcript, type TranscriptSource } from '../../../lib/stores/transcript.svelte';
  import { ws } from '../../../lib/stores/workspace.svelte';
  import { activity } from '../../../lib/stores/activity.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { groupTurns, activeQueued, fmtCost, fmtDuration, fmtTokens } from './format';
  import type { SessionStatus, TranscriptUnavailableReason } from '../../../lib/api/types';
  import { CONV_CTX, type ConvContext } from './context';

  let { sessionId, transcriptPath, workspaceId, readonly = false }: ConversationViewProps = $props();

  const src = $derived<TranscriptSource>(
    sessionId ? { sessionId } : { workspaceId, transcriptPath: transcriptPath ?? '' },
  );
  const conv = $derived(transcript.conversation(src));

  // Context for the tree (images, file opens, lazy subagents). Kept as one
  // reactive object so nested components see prop changes without re-mounting.
  const ctx: ConvContext = $state(
    untrack(() => ({ conv: transcript.conversation(src), sessionId: null, readonly, provider: 'claude' as const, queuedLive: [] })),
  );
  setContext(CONV_CTX, ctx);
  $effect(() => {
    ctx.conv = conv;
    ctx.sessionId = sessionId ?? null;
    ctx.readonly = readonly || !sessionId;
    ctx.provider = conv.transcript?.provider ?? 'claude';
    // "Queued: …" chips survive only until a later dequeue/remove of that text.
    ctx.queuedLive = activeQueued(conv.turns).map((q) => q.text);
  });

  // Load on mount / when the source changes.
  // `error == null` keeps a 401/404/500 from retrying forever (Retry is manual).
  $effect(() => {
    const c = conv;
    if (c.transcript == null && !c.loading && c.error == null) void c.load();
  });
  // Board-task nudges for the composer status line.
  $effect(() => {
    if (sessionId && workspaceId) void activity.load(workspaceId, sessionId);
  });

  const t = $derived(conv.transcript);
  // Mounted window: at most MAX_MOUNTED turns in the DOM. Turns are variable
  // height (VirtualList assumes uniform rows), so instead of pixel windowing
  // the list keeps a bounded slice — following the tail by default; after
  // "Load earlier" the window pins to the oldest loaded turns and a "Load
  // later" affordance walks it back toward the tail.
  const MAX_MOUNTED = 300;
  const STEP = 60;
  let followTail = $state(true);
  let manualStart = $state(0);
  const winStart = $derived(
    followTail
      ? Math.max(0, conv.turns.length - MAX_MOUNTED)
      : Math.min(manualStart, Math.max(0, conv.turns.length - MAX_MOUNTED)),
  );
  const winEnd = $derived(Math.min(conv.turns.length, winStart + MAX_MOUNTED));
  const hasLater = $derived(winEnd < conv.turns.length);
  const items = $derived(groupTurns(conv.turns.slice(winStart, winEnd)));
  function loadLater(): void {
    const next = manualStart + STEP;
    if (next + MAX_MOUNTED >= conv.turns.length) followTail = true;
    else manualStart = next;
  }
  const status = $derived<SessionStatus>(
    sessionId ? (ws.statusMap[sessionId] ?? ws.sessions.find((s) => s.id === sessionId)?.status ?? 'idle') : 'exited',
  );
  const live = $derived(!!sessionId && (status === 'working' || status === 'running'));
  const canCompose = $derived(!!sessionId && !readonly && ws.myRole !== 'viewer');
  const showSystem = $derived(transcript.showSystem);

  // ---- scroll: follow the tail unless the reader scrolled up ------------------
  let listEl = $state<HTMLDivElement | null>(null);
  let atBottom = $state(true);
  let unseen = $state(0);
  function onScroll(): void {
    const el = listEl;
    if (!el) return;
    atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 48;
    if (atBottom) unseen = 0;
    // Infinite "Load earlier" when the reader reaches the top.
    if (el.scrollTop < 40 && t?.has_earlier && !conv.loadingEarlier) void loadEarlier();
  }
  function scrollToBottom(): void {
    const el = listEl;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
    atBottom = true;
    unseen = 0;
  }
  // First paint of a conversation → bottom.
  let paintedFor = '';
  $effect(() => {
    if (conv.loading || !t || paintedFor === conv.key) return;
    paintedFor = conv.key;
    void tick().then(scrollToBottom);
  });
  // Live appends: follow when at the bottom, else count them for the pill.
  $effect(() => {
    void conv.tailTick;
    if (untrack(() => atBottom)) void tick().then(scrollToBottom);
    else unseen += 1;
  });
  async function loadEarlier(): Promise<void> {
    const el = listEl;
    const beforeH = el?.scrollHeight ?? 0;
    const beforeTop = el?.scrollTop ?? 0;
    await conv.loadEarlier();
    // Pin the window to the oldest loaded turns so the new page is what shows.
    followTail = false;
    manualStart = 0;
    await tick();
    if (el) el.scrollTop = el.scrollHeight - beforeH + beforeTop; // anchor
  }
  function scrollToBottomAll(): void {
    followTail = true;
    void tick().then(scrollToBottom);
  }

  async function resume(): Promise<void> {
    if (!sessionId) return;
    try {
      await ws.restartSession(sessionId);
    } catch (e) {
      toasts.error('Resume failed', e instanceof Error ? e.message : String(e));
    }
  }

  const UNAVAILABLE: Record<TranscriptUnavailableReason, { title: string; body: string }> = {
    no_provider_session_id: {
      title: 'No transcript yet',
      body: 'The agent has not written a transcript for this session (it appears after the first prompt). The terminal has everything so far.',
    },
    transcript_missing: {
      title: 'Transcript not found on disk',
      body: 'The provider transcript file for this session is gone or was never created here. Use the terminal view.',
    },
    provider_unsupported: {
      title: 'Chat view not available for this provider',
      body: 'This agent does not keep a readable transcript. Use the terminal view.',
    },
    codex_rollout_unresolved: {
      title: 'Codex rollout not matched',
      body: 'Otto could not match this session to a Codex rollout file yet. It usually resolves after the next turn; the terminal is complete meanwhile.',
    },
  };
  const unavailable = $derived(t?.unavailable_reason ? UNAVAILABLE[t.unavailable_reason] : null);
</script>

<div class="conv" data-session={sessionId} data-path={transcriptPath} data-ws={workspaceId} data-readonly={ctx.readonly} data-loaded={t != null}>
  <header class="conv-head">
    {#if t?.provider && hasProviderIcon(t.provider)}<ProviderIcon provider={t.provider} size={13} />{/if}
    <span class="conv-title" title={t?.title ?? ''}>{t?.title ?? (conv.loading ? 'Loading…' : 'Conversation')}</span>
    {#if t?.model}<span class="chip mono">{t.model}</span>{/if}
    {#if t && !t.unavailable_reason}
      <span class="stats dim" title="turns · tool calls · cost · tokens in/out · duration">
        {t.stats.turns} turns · {t.stats.tool_calls} tools
        {#if t.stats.cost_usd != null} · {fmtCost(t.stats.cost_usd)}{/if}
        {#if t.stats.input_tokens != null || t.stats.output_tokens != null} · {fmtTokens(t.stats.input_tokens)}↑ {fmtTokens(t.stats.output_tokens)}↓{/if}
        {#if t.stats.duration_ms != null} · {fmtDuration(t.stats.duration_ms)}{/if}
      </span>
    {/if}
    <span class="grow"></span>
    <label class="sys-toggle" title="Reveal system reminders, hooks, attachments and injected queue items">
      <input type="checkbox" checked={showSystem} onchange={(e) => transcript.setShowSystem((e.currentTarget as HTMLInputElement).checked)} />
      Show system
    </label>
    <button class="icon-btn" title="Reload transcript" aria-label="Reload transcript" onclick={() => void conv.load()}><Icon name="refresh" size={12} /></button>
  </header>

  <div class="conv-list" bind:this={listEl} onscroll={onScroll} dir="auto">
    {#if conv.error && !t}
      <div class="empty">
        <div class="empty-title">Could not load the conversation</div>
        <div class="dim">{conv.error}</div>
        <button class="btn small" onclick={() => void conv.load()}>Retry</button>
      </div>
    {:else if conv.loading && !t}
      <div class="empty dim">Loading conversation…</div>
    {:else if unavailable}
      <div class="empty" data-unavailable={t?.unavailable_reason}>
        <div class="empty-title">{unavailable.title}</div>
        <div class="dim">{unavailable.body}</div>
      </div>
    {:else if t && !items.length}
      <div class="empty dim">No turns recorded yet.</div>
    {:else if t}
      {#if t.has_earlier && winStart === 0}
        <div class="earlier">
          <button class="btn small ghost" disabled={conv.loadingEarlier} onclick={() => void loadEarlier()}>
            {conv.loadingEarlier ? 'Loading…' : 'Load earlier'}
          </button>
        </div>
      {/if}
      {#each items as item, i (item.id)}
        <TurnItem {item} live={live && !hasLater && i === items.length - 1 && item.role === 'assistant'} />
      {/each}
      {#if hasLater}
        <div class="earlier">
          <button class="btn small ghost" onclick={loadLater} data-later={conv.turns.length - winEnd}>
            Load later ({conv.turns.length - winEnd} more)
          </button>
        </div>
      {/if}
      {#if conv.liveArtifacts.length}
        <div class="live-artifacts">
          {#each conv.liveArtifacts as a (a.id)}
            {#if a.url}
              <a class="chip" href={a.url} target="_blank" rel="noopener noreferrer" title={a.path ?? a.url}><Icon name="link" size={11} /> {a.label}</a>
            {:else}
              <span class="chip" title={a.path ?? ''}><Icon name="file" size={11} /> {a.label}</span>
            {/if}
          {/each}
        </div>
      {/if}
      {#if conv.error}<div class="inline-err">{conv.error}</div>{/if}
    {/if}
  </div>

  {#if (unseen > 0 && !atBottom) || hasLater}
    <button class="new-pill" onclick={scrollToBottomAll}>↓ {hasLater ? 'latest' : 'new'}</button>
  {/if}

  {#if canCompose && sessionId}
    <Composer {sessionId} {status} onresume={() => void resume()} />
  {/if}
</div>

<style>
  .conv {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    min-width: 0;
    position: relative;
    background: var(--bg);
    color: var(--text);
  }
  .conv-head {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 30px;
    padding: 0 10px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
    flex-shrink: 0;
    font-size: 12px;
    min-width: 0;
  }
  .conv-title {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 40%;
  }
  .stats {
    font-size: 11px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }
  .sys-toggle {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    color: var(--text-dim);
    cursor: pointer;
    white-space: nowrap;
  }
  .conv-list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overflow-x: hidden;
    padding: 8px 0 12px;
    overflow-anchor: none;
  }
  .earlier {
    display: flex;
    justify-content: center;
    padding: 4px 0 8px;
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    text-align: center;
    padding: 48px 24px;
    font-size: 13px;
    max-width: 480px;
    margin: 0 auto;
  }
  .empty-title {
    font-weight: 600;
    font-size: 14px;
  }
  .live-artifacts {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    padding: 6px 16px;
  }
  .live-artifacts .chip {
    gap: 5px;
    text-decoration: none;
    color: var(--text);
  }
  .inline-err {
    color: var(--status-exited, #e5534b);
    font-size: 11.5px;
    padding: 4px 16px;
  }
  .new-pill {
    position: absolute;
    bottom: 96px;
    inset-inline-end: 50%;
    transform: translateX(50%);
    background: var(--accent);
    color: var(--accent-contrast, #fff);
    border: 0;
    border-radius: 99px;
    padding: 4px 12px;
    font-size: 11.5px;
    font-weight: 600;
    cursor: pointer;
    box-shadow: var(--shadow);
  }
  @media (max-width: 640px) {
    .stats,
    .conv-head .chip {
      display: none;
    }
  }
</style>
