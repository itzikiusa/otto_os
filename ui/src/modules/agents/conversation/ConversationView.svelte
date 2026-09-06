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
  import LiveDraft from './LiveDraft.svelte';
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
  // The session is alive (has a PTY) — the tail is worth keeping warm.
  const alive = $derived(!!sessionId && status !== 'exited' && status !== 'reconnectable');
  /** Suspended (PTY freed, still resumable via the provider id) — the state a
   *  session lands in ~8 min after its chat was left (idle-suspend sweep). */
  const suspended = $derived(
    !!sessionId &&
      status === 'reconnectable' &&
      ws.sessions.find((s) => s.id === sessionId)?.provider_session_id != null,
  );

  // ---- parity with the terminal: opening the chat RESUMES a suspended session --
  // Reopening a terminal auto-resumes (the WS attach calls `ensure_live`);
  // a chat never attached, so a session you stepped away from for a few
  // minutes came back as a dead "Resume" banner. The per-session touch now
  // does the same `ensure_live`, so send one on mount while suspended (once —
  // the status flip to working/idle re-runs nothing here; the `alive` effect
  // below takes over and keeps the tail armed).
  $effect(() => {
    if (!suspended) return;
    const c = conv;
    untrack(() => void c.touch());
  });
  // Remount over an already-loaded conversation: the tail may have lapsed
  // while the view was away (a fresh tail starts at the file's END, so records
  // written in the gap never arrive as deltas) — re-read the newest page.
  $effect(() => {
    const c = conv;
    untrack(() => {
      if (c.transcript != null && !c.loading) void c.resync();
    });
  });

  // ---- liveness: this VIEW keeps the server tail armed -----------------------
  // The tail stops a few minutes after the last touch, so an open chat pings
  // once a minute (only while mounted and the session is alive — closing the
  // view lets it die; nothing else in the app arms tails).
  const TOUCH_EVERY_MS = 60_000;
  $effect(() => {
    if (!alive) return;
    const c = conv;
    void c.touch();
    const id = setInterval(() => void c.touch(), TOUCH_EVERY_MS);
    // Back from a hidden tab / sleep: catch up on what the socket missed and
    // re-arm — the reader should never have to press Reload.
    const onVis = (): void => {
      if (!document.hidden) void c.resync();
    };
    document.addEventListener('visibilitychange', onVis);
    return () => {
      clearInterval(id);
      document.removeEventListener('visibilitychange', onVis);
    };
  });
  // No transcript yet (first prompt not sent, provider id not captured, Codex
  // rollout not matched): nothing will push an event, so retry the read every
  // few seconds while the session is alive instead of leaving a dead page.
  const RETRY_EVERY_MS = 5_000;
  $effect(() => {
    if (!alive || !t?.unavailable_reason || conv.loading) return;
    const c = conv;
    const id = setTimeout(() => void c.load(), RETRY_EVERY_MS);
    return () => clearTimeout(id);
  });

  // ---- live draft (sub-turn streaming off the terminal screen) --------------
  // Shown only while the agent is WORKING and the draft is not already folded.
  const lastAssistantText = $derived.by(() => {
    for (let i = conv.turns.length - 1; i >= 0; i--) {
      const turn = conv.turns[i];
      if (turn.role !== 'assistant') continue;
      const texts = turn.blocks.filter((b) => b.kind === 'text').map((b) => (b as { md: string }).md);
      return texts.join('\n');
    }
    return '';
  });
  const draft = $derived(sessionId && status === 'working' ? conv.liveDraft : '');
  // Follow the tail only when the draft gains LINES — a same-height text change
  // must not scroll (it reads as a jump), and neither must it shrink away.
  let draftLines = 0;
  $effect(() => {
    const n = draft ? draft.split('\n').length : 0;
    const grew = n > draftLines;
    draftLines = n;
    if (grew && untrack(() => atBottom)) void tick().then(scrollToBottom);
  });

  // ---- search within the loaded conversation ---------------------------------
  let searchOpen = $state(false);
  let query = $state('');
  let searchEl = $state<HTMLInputElement | null>(null);
  let hitIdx = $state(0);
  function turnText(turn: (typeof conv.turns)[number]): string {
    const parts: string[] = [];
    for (const b of turn.blocks) {
      if (b.kind === 'text') parts.push(b.md);
      else if (b.kind === 'tool_call') parts.push(b.title, b.name, b.result?.text ?? '');
      else if (b.kind === 'queued') parts.push(b.text);
    }
    return parts.join('\n').toLowerCase();
  }
  const hits = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (!q) return [] as string[];
    return conv.turns.filter((turn) => turnText(turn).includes(q)).map((turn) => turn.id);
  });
  $effect(() => {
    void hits.length;
    hitIdx = 0;
  });
  function jumpTo(i: number): void {
    if (!hits.length) return;
    hitIdx = ((i % hits.length) + hits.length) % hits.length;
    const id = hits[hitIdx];
    // Make sure the turn is in the mounted window, then scroll it into view.
    const at = conv.turns.findIndex((turn) => turn.id === id);
    if (at >= 0 && (at < winStart || at >= winEnd)) {
      followTail = false;
      manualStart = Math.max(0, at - Math.floor(MAX_MOUNTED / 2));
    }
    void tick().then(() => {
      const el = listEl?.querySelector(`[data-turn-id="${CSS.escape(id)}"]`);
      el?.scrollIntoView({ block: 'center' });
      atBottom = false;
    });
  }
  function openSearch(): void {
    searchOpen = true;
    void tick().then(() => searchEl?.select());
  }
  function closeSearch(): void {
    searchOpen = false;
    query = '';
  }
  function onSearchKey(e: KeyboardEvent): void {
    if (e.key === 'Escape') {
      e.preventDefault();
      closeSearch();
    } else if (e.key === 'Enter') {
      e.preventDefault();
      jumpTo(hitIdx + (e.shiftKey ? -1 : 1));
    }
  }
  function onConvKey(e: KeyboardEvent): void {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'f' && !e.shiftKey && !e.altKey) {
      e.preventDefault();
      e.stopPropagation();
      openSearch();
    }
  }
  const currentHit = $derived(hits[hitIdx] ?? null);

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

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="conv" data-session={sessionId} data-path={transcriptPath} data-ws={workspaceId} data-readonly={ctx.readonly} data-loaded={t != null} onkeydown={onConvKey}>
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
    {#if searchOpen}
      <div class="search" role="search">
        <Icon name="search" size={12} />
        <input
          bind:this={searchEl}
          bind:value={query}
          class="search-in"
          placeholder="Search this conversation"
          aria-label="Search this conversation"
          onkeydown={onSearchKey}
        />
        <span class="search-n dim" data-search-hits={hits.length}>{hits.length ? `${hitIdx + 1}/${hits.length}` : query ? '0' : ''}</span>
        <button class="icon-btn" title="Previous match (⇧⏎)" aria-label="Previous match" disabled={!hits.length} onclick={() => jumpTo(hitIdx - 1)}><Icon name="chevronUp" size={11} /></button>
        <button class="icon-btn" title="Next match (⏎)" aria-label="Next match" disabled={!hits.length} onclick={() => jumpTo(hitIdx + 1)}><Icon name="chevronDown" size={11} /></button>
        <button class="icon-btn" title="Close (Esc)" aria-label="Close search" onclick={closeSearch}><Icon name="x" size={11} /></button>
      </div>
    {:else}
      <button class="icon-btn" title="Search this conversation (⌘F)" aria-label="Search this conversation" onclick={openSearch}><Icon name="search" size={12} /></button>
    {/if}
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
        <TurnItem
          {item}
          live={live && !hasLater && i === items.length - 1 && item.role === 'assistant'}
          hit={!!query && hits.includes(item.id)}
          current={item.id === currentHit}
        />
      {/each}
      {#if draft && !hasLater}
        <LiveDraft text={draft} lastText={lastAssistantText} />
      {/if}
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
    <Composer
      {sessionId}
      {status}
      onresume={() => void resume()}
      cwd={ws.sessions.find((s) => s.id === sessionId)?.cwd ?? ''}
      branch={conv.liveBranch}
      model={t?.model ?? null}
      termStatus={conv.liveStatus}
      termInput={conv.liveInput}
    />
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
  .search {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    border: 1px solid color-mix(in srgb, var(--accent) 45%, var(--border));
    border-radius: var(--radius-s);
    background: var(--bg);
    padding: 0 4px 0 6px;
    height: 22px;
    color: var(--text-dim);
    min-width: 0;
  }
  .search-in {
    border: 0;
    outline: 0;
    background: none;
    color: var(--text);
    font: inherit;
    font-size: 11.5px;
    width: 180px;
    min-width: 0;
  }
  .search-n {
    font-size: 10.5px;
    min-width: 28px;
    text-align: center;
    white-space: nowrap;
  }
  @media (max-width: 640px) {
    .search-in {
      width: 110px;
    }
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
