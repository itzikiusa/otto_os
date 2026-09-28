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
  // bottom with a "N new messages" jump pill otherwise, live tail via
  // `transcript_appended`. The column keeps a readable measure
  // (`--chat-measure`) and every piece sheds chrome by the PANE's width.
  import { setContext, tick, untrack } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../../lib/components/ProviderIcon.svelte';
  import TurnItem from './TurnItem.svelte';
  import Composer from './Composer.svelte';
  import LiveDraft from './LiveDraft.svelte';
  import LiveStatus from './LiveStatus.svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import { transcript, type TranscriptSource } from '../../../lib/stores/transcript.svelte';
  import { ws } from '../../../lib/stores/workspace.svelte';
  import { ctxMenu } from '../../../lib/contextmenu.svelte';
  import { activity } from '../../../lib/stores/activity.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { groupTurns, stableGroupTurns, activeQueued, countUnread, fmtCost, fmtDuration, fmtTokens, pendingTool, providerName } from './format';
  import { registerFindProvider } from '../../../lib/findProviders';
  import type { SessionStatus, TranscriptUnavailableReason, Turn } from '../../../lib/api/types';
  import type { RenderItem } from './format';
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
    // Compare before assigning: a fresh array on every delta would re-run
    // every mounted TurnItem's `visibleBlocks`.
    const queued = activeQueued(conv.turns).map((q) => q.text);
    const prevQueued = untrack(() => ctx.queuedLive);
    if (queued.length !== prevQueued.length || queued.some((q, i) => q !== prevQueued[i])) ctx.queuedLive = queued;
  });

  // The lease owns initial/reconnect reads and releases them on source change.
  $effect(() => {
    const source = src;
    void conv; // Reacquire after daemon/user identity invalidates the conversation.
    return untrack(() => transcript.acquireView(source));
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
  // Stable items: unchanged responses keep their object across live deltas, so
  // only the growing response re-renders (format.ts `stableGroupTurns`).
  const itemCache = new Map<string, RenderItem>();
  const items = $derived(stableGroupTurns(conv.turns.slice(winStart, winEnd), itemCache));
  // The store drops the oldest turns of a very long live chat (TURN_CAP); keep
  // a pinned window on the same turns when the head shifts under it.
  let droppedSeen = { key: '', n: 0 };
  $effect(() => {
    const key = conv.key;
    const n = conv.headDropped;
    const prev = untrack(() => droppedSeen);
    droppedSeen = { key, n };
    if (prev.key !== key || n <= prev.n) return;
    if (!untrack(() => followTail)) manualStart = Math.max(0, untrack(() => manualStart) - (n - prev.n));
  });
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

  // ---- live state at the foot of the chat -------------------------------------
  const agentName = $derived(providerName(t?.provider ?? 'claude'));
  const working = $derived(!!sessionId && status === 'working');
  const lastItem = $derived(hasLater ? undefined : items[items.length - 1]);
  /** The newest call still without a result — the current step while working,
   *  or (session alive but quiet) what the agent is blocked on: a permission
   *  prompt / question on the terminal screen. */
  const pendingCall = $derived(pendingTool(lastItem));
  const waiting = $derived(alive && !working && pendingCall != null);
  /** Your last message's time — the working line's elapsed clock. */
  const lastPromptTs = $derived.by(() => {
    for (let i = items.length - 1; i >= 0; i--) if (items[i].role === 'user') return items[i].ts;
    return null;
  });
  function openTerminal(): void {
    if (sessionId) transcript.setView(sessionId, 'terminal');
  }
  // Screen-reader announcements: only the moments that matter (working,
  // finished, needs you) — not every streamed frame or tool row.
  let announce = $state('');
  let wasWorking = false;
  $effect(() => {
    const w = working;
    if (wasWorking && !w) announce = untrack(() => waiting) ? `${untrack(() => agentName)} is waiting for you` : `${untrack(() => agentName)} finished responding`;
    else if (!wasWorking && w) announce = `${untrack(() => agentName)} is working`;
    wasWorking = w;
  });

  // ---- liveness: this VIEW keeps the server tail armed -----------------------
  // The tail stops a few minutes after the last touch, so an open chat pings
  // once a minute (only while mounted and the session is alive — closing the
  // view lets it die; nothing else in the app arms tails).
  const TOUCH_EVERY_MS = 60_000;
  $effect(() => {
    if (!alive) return;
    const c = conv;
    const id = setInterval(() => void c.touch(), TOUCH_EVERY_MS);
    // Visibility recovery is owned once per window by TranscriptLifecycle.
    return () => clearInterval(id);
  });
  // No transcript yet (first prompt not sent, provider id not captured, Codex
  // rollout not matched): nothing will push an event, so retry the read every
  // few seconds while the session is alive instead of leaving a dead page.
  const RETRY_EVERY_MS = 5_000;
  $effect(() => {
    if (!alive || !t?.unavailable_reason || conv.loading) return;
    const c = conv;
    const id = setTimeout(() => c.requestRefresh(), RETRY_EVERY_MS);
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
  // What the hits are computed against: `query` settled for SEARCH_DEBOUNCE_MS
  // (Enter flushes it), so a keystroke doesn't rescan megabytes of tool text.
  let searchQ = $state('');
  const SEARCH_DEBOUNCE_MS = 150;
  $effect(() => {
    const q = query;
    if (q === untrack(() => searchQ)) return;
    if (!q) {
      searchQ = '';
      return;
    }
    const id = setTimeout(() => (searchQ = q), SEARCH_DEBOUNCE_MS);
    return () => clearTimeout(id);
  });
  let searchEl = $state<HTMLInputElement | null>(null);
  let hitIdx = $state(0);
  // Lowercased search text per Turn object. Turns are replaced (never mutated)
  // when they change, so an unchanged turn is lowercased once per session.
  const turnTextCache = new WeakMap<Turn, string>();
  function turnText(turn: Turn): string {
    const hit = turnTextCache.get(turn);
    if (hit !== undefined) return hit;
    const parts: string[] = [];
    for (const b of turn.blocks) {
      if (b.kind === 'text') parts.push(b.md);
      else if (b.kind === 'tool_call') parts.push(b.title, b.name, b.result?.text ?? '');
      else if (b.kind === 'queued') parts.push(b.text);
    }
    const text = parts.join('\n').toLowerCase();
    turnTextCache.set(turn, text);
    return text;
  }
  const hits = $derived.by(() => {
    const q = searchQ.trim().toLowerCase();
    if (!q) return [] as string[];
    return conv.turns.filter((turn) => turnText(turn).includes(q)).map((turn) => turn.id);
  });
  const hitSet = $derived(new Set(hits));
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
  // ⌘F over the whole loaded chat (lib/findProviders.ts). Only MAX_MOUNTED
  // turns are in the DOM, so a long chat's older turns were invisible to
  // find-in-page. Past that size the provider searches every loaded turn
  // (grouped as rendered: one row per TurnItem) and reveals a hit by moving
  // the mounted window, like the chat's own search does. Below it every turn
  // is mounted and the plain DOM walk is exact.
  let findGroups: { turns: Turn[]; items: RenderItem[] } = { turns: [], items: [] };
  function allGroups(): RenderItem[] {
    const turns = conv.turns;
    if (findGroups.turns !== turns) findGroups = { turns, items: groupTurns(turns) };
    return findGroups.items;
  }
  $effect(() =>
    registerFindProvider({
      root: () => listEl,
      active: () => conv.turns.length > MAX_MOUNTED,
      count: () => allGroups().length,
      text: (i) => allGroups()[i]?.turns.map(turnText).join('\n') ?? '',
      reveal: async (i) => {
        const head = allGroups()[i]?.turns[0];
        const at = head ? conv.turns.indexOf(head) : -1;
        if (at >= 0 && (at < winStart || at >= winEnd)) {
          followTail = false;
          manualStart = Math.max(0, at - Math.floor(MAX_MOUNTED / 2));
        }
        await tick();
        atBottom = false;
      },
      rowElement: (i) => {
        const id = allGroups()[i]?.id;
        return id ? (listEl?.querySelector(`[data-turn-id="${CSS.escape(id)}"]`) ?? null) : null;
      },
    }),
  );
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
      if (searchQ !== query) {
        // Typed faster than the debounce: search now, land on the first hit.
        searchQ = query;
        jumpTo(0);
        return;
      }
      jumpTo(hitIdx + (e.shiftKey ? -1 : 1));
    }
  }
  function onConvKey(e: KeyboardEvent): void {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'f' && !e.shiftKey && !e.altKey) {
      e.preventDefault();
      e.stopPropagation();
      openSearch();
    } else if ((e.metaKey || e.ctrlKey) && e.key === 'ArrowDown' && !(e.target instanceof HTMLTextAreaElement || e.target instanceof HTMLInputElement)) {
      // ⌘↓ — jump to the latest message (the pill's shortcut).
      e.preventDefault();
      scrollToBottomAll();
    }
  }
  const currentHit = $derived(hits[hitIdx] ?? null);

  // ---- scroll: follow the tail unless the reader scrolled up ------------------
  let listEl = $state<HTMLDivElement | null>(null);
  let colEl = $state<HTMLDivElement | null>(null);
  let atBottom = $state(true);
  /** Scrolled up by more than a screen — the pill offers the way back even
   *  with nothing new. */
  let farUp = $state(false);
  let unseen = $state(0);
  // Unread = render items that arrived after the last one seen at the bottom.
  let lastSeenId = $state<string | null>(null);
  $effect(() => {
    if (atBottom && !hasLater) lastSeenId = items[items.length - 1]?.id ?? null;
  });
  const unread = $derived(atBottom ? 0 : countUnread(items.map((i) => i.id), lastSeenId));
  const showPill = $derived(hasLater || (!atBottom && (unread > 0 || unseen > 0 || farUp)));
  const pillText = $derived(
    hasLater ? 'Jump to latest' : unread > 0 ? `${unread} new ${unread === 1 ? 'message' : 'messages'}` : unseen > 0 ? 'New activity' : 'Jump to latest',
  );
  function onScroll(): void {
    const el = listEl;
    if (!el) return;
    const gap = el.scrollHeight - el.scrollTop - el.clientHeight;
    atBottom = gap < 48;
    farUp = gap > el.clientHeight;
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
  // (`unseen += 1` would READ `unseen` inside the effect and re-trigger it —
  // an effect loop the app answers with a reload, exactly when you had
  // scrolled up to read while the agent streamed.)
  $effect(() => {
    void conv.tailTick;
    if (untrack(() => atBottom)) void tick().then(scrollToBottom);
    else unseen = untrack(() => unseen) + 1;
  });
  // Stay pinned while the tail grows for any other reason (a highlighted code
  // block, an image loading, the live line appearing) — unless the reader just
  // clicked something (expanding a step at the bottom must not yank the view).
  let lastPointer = 0;
  $effect(() => {
    const col = colEl;
    if (!col || typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(() => {
      if (untrack(() => atBottom) && performance.now() - lastPointer > 600) scrollToBottom();
    });
    ro.observe(col);
    return () => ro.disconnect();
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

  // ── Header chrome by PANE width, not window width ────────────────────────
  // A chat can live in a 200px tile or a full-window tab; `.conv` is the sized
  // flex child, so its inline size is the only honest measure. The
  // `@container` blocks at the bottom use the SAME numbers.
  let convW = $state(0);
  /** ≤260px: the search button folds into the ⋯ menu, and an open search box
   *  takes the whole row. */
  const narrowHead = $derived(convW > 0 && convW <= 260);

  /** ⋯ — the chat's secondary controls: system notes, reload (and search when
   *  the row is too narrow for its button). */
  function openHeadMenu(e: MouseEvent | KeyboardEvent): void {
    ctxMenu.show(e, [
      ...(narrowHead ? [{ label: 'Search…', icon: 'search', hint: '⌘F', action: () => openSearch() }] : []),
      {
        label: 'Show system notes',
        icon: showSystem ? 'eye' : 'eyeOff',
        checked: showSystem,
        title: 'Reveal system reminders, hooks, attachments and injected queue items',
        action: () => transcript.setShowSystem(!showSystem),
      },
      { label: 'Reload transcript', icon: 'refresh', action: () => void conv.load() },
    ]);
  }
  const statsText = $derived.by(() => {
    if (!t || t.unavailable_reason) return '';
    const st = t.stats;
    const parts = [`${st.turns} turns`, `${st.tool_calls} tools`];
    if (st.cost_usd != null) parts.push(fmtCost(st.cost_usd));
    if (st.input_tokens != null || st.output_tokens != null) parts.push(`${fmtTokens(st.input_tokens)}↑ ${fmtTokens(st.output_tokens)}↓`);
    if (st.duration_ms != null) parts.push(fmtDuration(st.duration_ms));
    return parts.join(' · ');
  });
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="conv" bind:clientWidth={convW} data-session={sessionId} data-path={transcriptPath} data-ws={workspaceId} data-readonly={ctx.readonly} data-loaded={t != null} onkeydown={onConvKey}>
  <header class="conv-head" class:folded={narrowHead && searchOpen}>
    {#if t?.provider && hasProviderIcon(t.provider)}<ProviderIcon provider={t.provider} size={13} />{/if}
    <span class="conv-title" title={[t?.title, t?.model, statsText].filter(Boolean).join(' · ')}>{t?.title ?? (conv.loading ? 'Loading…' : 'Conversation')}</span>
    {#if statsText}
      <span class="stats" title="turns · tool calls · cost · tokens in/out · duration">{statsText}</span>
    {/if}
    <span class="grow"></span>
    {#if searchOpen}
      <div class="search" class:wide={narrowHead} role="search">
        <Icon name="search" size={12} />
        <input
          bind:this={searchEl}
          bind:value={query}
          class="search-in"
          placeholder="Search this conversation"
          aria-label="Search this conversation"
          onkeydown={onSearchKey}
        />
        <span class="search-n" data-search-hits={hits.length} aria-live="polite">{hits.length ? `${hitIdx + 1}/${hits.length}` : query && searchQ === query ? 'No matches' : ''}</span>
        <button class="icon-btn" title="Previous match (⇧⏎)" aria-label="Previous match" disabled={!hits.length} onclick={() => jumpTo(hitIdx - 1)}><Icon name="chevronUp" size={11} /></button>
        <button class="icon-btn" title="Next match (⏎)" aria-label="Next match" disabled={!hits.length} onclick={() => jumpTo(hitIdx + 1)}><Icon name="chevronDown" size={11} /></button>
        <button class="icon-btn" title="Close (Esc)" aria-label="Close search" onclick={closeSearch}><Icon name="x" size={11} /></button>
      </div>
    {:else if !narrowHead}
      <button class="icon-btn" title="Search this conversation (⌘F)" aria-label="Search this conversation" onclick={openSearch}><Icon name="search" size={12} /></button>
    {/if}
    <button class="icon-btn" aria-label="Conversation options" title="Conversation options" aria-haspopup="menu" data-conv-menu onclick={openHeadMenu}><Icon name="more" size={12} /></button>
  </header>

  <!-- The scroller's frame: the jump pill anchors to ITS bottom edge, so it
       always sits just above the composer whatever the composer's height. -->
  <div class="conv-frame">
  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <div
    class="conv-list"
    bind:this={listEl}
    onscroll={onScroll}
    onpointerdown={() => (lastPointer = performance.now())}
    dir="auto"
    tabindex="0"
    role="region"
    aria-label="Conversation with {agentName}"
  >
    <div class="conv-col" bind:this={colEl}>
    {#if conv.error && !t}
      <EmptyState icon="warning" title="Couldn't load the conversation" body={conv.error} actionLabel="Retry" actionIcon="refresh" actionKind="secondary" onaction={() => void conv.load()} />
    {:else if conv.loading && !t}
      <div class="skeleton" aria-busy="true" aria-label="Loading the conversation">
        <div class="sk sk-user"></div>
        <div class="sk sk-line"></div>
        <div class="sk sk-line short"></div>
        <div class="sk sk-steps"></div>
        <div class="sk sk-line"></div>
      </div>
    {:else if unavailable}
      <div data-unavailable={t?.unavailable_reason}>
        <EmptyState
          icon="terminal"
          title={unavailable.title}
          body={unavailable.body}
          actionLabel={sessionId ? 'Open terminal' : undefined}
          actionIcon="terminal"
          actionKind="secondary"
          onaction={sessionId ? openTerminal : undefined}
        />
      </div>
    {:else if t && !items.length}
      <EmptyState icon="comment" title="No messages yet" body={canCompose ? `Send ${agentName} a message below — the conversation shows up here as it works.` : 'Nothing was recorded in this conversation.'} />
    {:else if t}
      {#if t.has_earlier && winStart === 0}
        <div class="earlier">
          <button class="btn small ghost" disabled={conv.loadingEarlier} onclick={() => void loadEarlier()}>
            {conv.loadingEarlier ? 'Loading…' : 'Load earlier messages'}
          </button>
        </div>
      {/if}
      {#each items as item, i (item.id)}
        <TurnItem
          {item}
          live={working && !hasLater && i === items.length - 1 && item.role === 'assistant'}
          active={alive && !hasLater && i === items.length - 1 && item.role === 'assistant'}
          waiting={waiting && i === items.length - 1}
          hit={!!searchQ && hitSet.has(item.id)}
          current={item.id === currentHit}
        />
      {/each}
      {#if draft && !hasLater}
        <LiveDraft text={draft} lastText={lastAssistantText} />
      {/if}
      {#if !hasLater && working}
        <LiveStatus mode="working" {agentName} pending={pendingCall} writing={!!draft} since={lastPromptTs} />
      {:else if !hasLater && waiting}
        <LiveStatus mode="waiting" {agentName} pending={pendingCall} onterminal={canCompose ? openTerminal : null} />
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
      {#if conv.error}<div class="inline-err" role="alert">{conv.error} <button class="btn small ghost" onclick={() => void conv.load()}>Retry</button></div>{/if}
    {/if}
    </div>
  </div>

  {#if showPill}
    <button class="jump-pill new-pill" onclick={scrollToBottomAll} data-unread={unread} title="Jump to the latest message (⌘↓)">
      <Icon name="arrowDown" size={12} /> {pillText}
    </button>
  {/if}
  </div>
  <div class="sr-only" aria-live="polite">{announce}</div>

  {#if canCompose && sessionId}
    {#key sessionId}
    <Composer
      {sessionId}
      {status}
      {agentName}
      onresume={() => void resume()}
      cwd={ws.sessions.find((s) => s.id === sessionId)?.cwd ?? ''}
      branch={conv.liveBranch}
      model={t?.model ?? null}
      termStatus={conv.liveStatus}
      termInput={conv.liveInput}
    />
    {/key}
  {/if}
</div>

<style>
  .conv {
    /* The readable measure of the message column (and the composer under it). */
    --chat-measure: 780px;
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    min-width: 0;
    position: relative;
    background: var(--bg);
    color: var(--text);
    /* The chat sheds chrome by its OWN width — it renders full-window in a tab
       and 200px wide in a tiled pane, and the window said nothing about that.
       Breakpoints mirror the `@container` blocks below + `narrowHead` above. */
    container-type: inline-size;
  }
  .conv-head {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 30px;
    padding: 0 6px 0 12px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
    flex-shrink: 0;
    font-size: var(--fs-s);
    min-width: 0;
  }
  .conv-title {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
    flex: 0 1 auto;
  }
  .stats {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
    flex: 0 2 auto;
  }
  .grow {
    flex: 1;
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
    font-size: var(--fs-xs);
    width: 180px;
    min-width: 0;
  }
  .search-n {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    min-width: 28px;
    text-align: center;
    white-space: nowrap;
  }
  @container (max-width: 360px) {
    .search-in {
      width: 110px;
    }
  }
  .search.wide {
    flex: 1;
  }
  .search.wide .search-in {
    width: 100%;
    flex: 1;
  }
  .conv-head.folded .conv-title {
    display: none;
  }
  .conv-frame {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    position: relative;
  }
  .conv-list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overflow-x: hidden;
    overflow-anchor: none;
    outline: none;
  }
  .conv-list:focus-visible {
    box-shadow: inset 0 0 0 2px var(--accent);
  }
  /* The message column: a readable measure, centred, with pane-sized gutters. */
  /* Block (not flex) on purpose: a live delta grows only the LAST item, and
     block layout re-lays that item, where a 300-child flex column re-runs
     the flex algorithm over every item (measurable per delta in WebKit). */
  .conv-col {
    max-width: var(--chat-measure);
    margin-inline: auto;
    padding: 12px 20px 20px;
    display: flow-root;
    min-width: 0;
  }
  @container (max-width: 480px) {
    .conv-col {
      padding: 8px 12px 16px;
    }
  }
  .earlier {
    display: flex;
    justify-content: center;
    padding: 4px 0 8px;
  }
  .skeleton {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding-block: 8px;
  }
  .sk {
    height: 14px;
    border-radius: var(--radius-s);
    background: var(--surface-2);
  }
  .sk-user {
    align-self: flex-end;
    width: 45%;
    height: 34px;
    border-radius: 16px;
  }
  .sk-line {
    width: 92%;
  }
  .sk-line.short {
    width: 60%;
  }
  .sk-steps {
    width: 70%;
    height: 22px;
  }
  @media (prefers-reduced-motion: no-preference) {
    .sk {
      animation: sk-pulse 1.4s ease-in-out infinite;
    }
  }
  @keyframes sk-pulse {
    50% {
      opacity: 0.55;
    }
  }
  .live-artifacts {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    padding: 6px 0;
  }
  .live-artifacts .chip {
    gap: 5px;
    text-decoration: none;
    color: var(--text);
  }
  .inline-err {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--danger);
    font-size: var(--fs-xs);
    padding: 4px 0;
  }
  .jump-pill {
    position: absolute;
    bottom: 12px;
    inset-inline-start: 50%;
    transform: translateX(-50%);
    display: inline-flex;
    align-items: center;
    gap: 6px;
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--border-strong);
    border-radius: 99px;
    padding: 4px 12px 4px 10px;
    font-size: var(--fs-xs);
    font-weight: 600;
    cursor: pointer;
    box-shadow: var(--shadow);
    white-space: nowrap;
  }
  :global([dir='rtl']) .jump-pill {
    transform: translateX(50%);
  }
  .jump-pill:hover {
    background: var(--surface-2);
  }
  .jump-pill:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  /* ≤560px: the stats go (they live in the title's tooltip too). */
  @container (max-width: 560px) {
    .stats {
      display: none;
    }
  }
</style>
