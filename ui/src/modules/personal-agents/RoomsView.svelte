<script lang="ts">
  import { rowMenu } from '../../lib/rowMenu';
  import { plural } from '../../lib/plural';
  // Agent channels (stored as "rooms") — the only agent-to-agent transport,
  // always user-visible. Named "channels" in the UI so they are never confused
  // with the sidebar's Rooms (human collaboration) — S20-07.
  // Room list + create on the left; the selected room's membership editor,
  // live message feed (WS agent_room_message + `after` paging) and the user
  // post box on the right.
  import { tick, untrack } from 'svelte';
  import RelTime from '../../lib/components/RelTime.svelte';
  import AgentChip from '../../lib/components/AgentChip.svelte';
  import { personalAgents } from '../../lib/stores/personalAgents.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import PaneDivider from '../../lib/components/PaneDivider.svelte';
  import { LIST_PANE, loadPaneWidth } from '../../lib/paneResizer';
  import { toasts } from '../../lib/toast.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import AgentAvatar from './AgentAvatar.svelte';
  import { loadErrorOf } from './loadError';
  import type { AgentRoomMessage, AgentRoomWithMembers } from '../../lib/api/types';

  let selectedId = $state<string | null>(null);
  /** Rooms list width — the shared list-pane default/range, remembered. */
  let listW = $state(loadPaneWidth('personalAgents.rooms.listW', LIST_PANE.default, LIST_PANE.min, LIST_PANE.max));
  let showingList = $state(false);
  let newRoomName = $state('');
  // Drafts and send results belong to a room, even while a request is pending.
  let drafts = $state<Record<string, string>>({});
  let sending = $state<Record<string, boolean>>({});
  let sendErrors = $state<Record<string, string>>({});
  const draft = $derived(selectedId ? (drafts[selectedId] ?? '') : '');
  const error = $derived(selectedId ? (sendErrors[selectedId] ?? '') : '');
  let busy = $state(false);
  let createError = $state('');
  let feedEl = $state<HTMLElement | null>(null);
  let createEl = $state<HTMLInputElement | null>(null);
  let roomListEl = $state<HTMLElement | null>(null);
  let roomsLoading = $state(true);
  const roomsError = $derived(loadErrorOf(personalAgents, 'roomsError'));

  const rooms = $derived(personalAgents.rooms);
  const selected = $derived(rooms.find((r) => r.room.id === selectedId) ?? null);
  const messages = $derived(selectedId ? (personalAgents.messagesByRoom[selectedId] ?? []) : []);
  // Render the newest RENDER_STEP messages; "Show earlier" reveals more of
  // what is held, then pages older history from the server (backlog B6 /
  // SA-08 — a busy room rendered up to 5,000 rows).
  const RENDER_STEP = 200;
  let renderCount = $state(RENDER_STEP);
  $effect(() => {
    void selectedId;
    renderCount = RENDER_STEP;
  });
  const shown = $derived(messages.length > renderCount ? messages.slice(messages.length - renderCount) : messages);
  const hiddenHeld = $derived(messages.length - shown.length);
  const hasOlder = $derived(hiddenHeld > 0 || (selectedId ? !!personalAgents.olderByRoom[selectedId] : false));
  async function showEarlier(): Promise<void> {
    const el = feedEl;
    const before = el ? el.scrollHeight - el.scrollTop : 0;
    if (hiddenHeld > 0) renderCount += RENDER_STEP;
    else if (selectedId) {
      await personalAgents.loadOlder(selectedId);
      renderCount += RENDER_STEP;
    }
    await tick();
    // Keep the message the user was reading in place (content grew above it).
    if (el) el.scrollTop = el.scrollHeight - before;
  }
  const nonMembers = $derived(
    personalAgents.agents.filter((a) => !(selected?.members ?? []).includes(a.id)),
  );

  function reloadRooms(): void {
    if (!ws.currentId) return;
    roomsLoading = true;
    void personalAgents.loadRooms(ws.currentId).finally(() => (roomsLoading = false));
  }
  $effect(() => {
    if (ws.currentId) reloadRooms();
  });
  // First room auto-selects; a deleted selection falls back.
  $effect(() => {
    if (!showingList && rooms.length > 0 && !rooms.some((r) => r.room.id === selectedId)) {
      selectedId = rooms[0].room.id;
    }
  });
  // Room events touch message feeds only while this view is mounted; on
  // unmount every feed but the selected room's is evicted.
  $effect(() => personalAgents.watchRooms());
  $effect(() => {
    const id = selectedId;
    untrack(() => personalAgents.setActiveRoom(id));
    if (id) untrack(() => void personalAgents.loadMessages(id));
  });
  // Keep the feed pinned to the latest message — on a NEW newest message
  // only, so paging older history in doesn't yank the view to the bottom.
  $effect(() => {
    void messages.at(-1)?.id;
    if (feedEl) untrack(() => { if (feedEl) feedEl.scrollTop = feedEl.scrollHeight; });
  });

  function authorName(m: AgentRoomMessage): string {
    if (m.author_kind === 'user') return m.author_id === auth.me?.id ? 'You' : 'User';
    return personalAgents.agent(m.author_id)?.name ?? 'Agent (removed)';
  }


  async function backToRooms(): Promise<void> {
    const previous = selectedId;
    showingList = true;
    selectedId = null;
    await tick();
    Array.from(roomListEl?.querySelectorAll<HTMLButtonElement>('.room') ?? [])
      .find(button => button.dataset.roomId === previous)?.focus();
  }

  async function createRoom(): Promise<void> {
    const submitted = newRoomName;
    const name = submitted.trim();
    if (!name || !ws.currentId || busy) return;
    busy = true;
    createError = '';
    try {
      selectedId = await personalAgents.createRoom(ws.currentId, name);
      showingList = false;
      if (newRoomName === submitted) newRoomName = '';
    } catch (e) {
      createError = `Couldn’t create the channel. ${loadErrorText(e)}`;
    } finally {
      busy = false;
    }
  }

  function roomMenu(e: MouseEvent | KeyboardEvent, r: AgentRoomWithMembers): void {
    ctxMenu.show(e, [
      {
        label: 'Rename',
        icon: 'edit',
        action: async () => {
          const name = await confirmer.promptText('Channel name', { title: 'Rename channel', confirmLabel: 'Rename', initial: r.room.name });
          // A blank (or whitespace-only) name is not a rename.
          const trimmed = name?.trim();
          if (trimmed && trimmed !== r.room.name) {
            void personalAgents.renameRoom(r.room.id, trimmed).catch((e) => toasts.error('Couldn’t rename the channel', loadErrorText(e)));
          }
        },
      },
      {
        label: 'Delete channel…',
        icon: 'trash',
        danger: true,
        action: async () => {
          if (await confirmer.ask(`Delete channel “${r.room.name}” and its transcript? Member agents stay; only the channel and its messages go.`, { title: 'Delete channel', confirmLabel: 'Delete' })) {
            void personalAgents.deleteRoom(r.room.id).catch((e) => toasts.error('Couldn’t delete the channel', loadErrorText(e)));
          }
        },
      },
    ]);
  }

  async function addMember(agentId: string): Promise<void> {
    if (!selectedId || !agentId) return;
    try {
      await personalAgents.addMember(selectedId, agentId);
    } catch (e) {
      toasts.error('Couldn’t add the agent to the channel', loadErrorText(e));
    }
  }

  async function removeMember(agentId: string): Promise<void> {
    if (!selectedId) return;
    try {
      await personalAgents.removeMember(selectedId, agentId);
    } catch (e) {
      toasts.error('Couldn’t remove the agent from the channel', loadErrorText(e));
    }
  }

  async function send(): Promise<void> {
    const roomId = selectedId;
    const submitted = draft;
    const text = submitted.trim();
    if (!text || !roomId || sending[roomId]) return;
    sending[roomId] = true;
    sendErrors[roomId] = '';
    try {
      await personalAgents.postMessage(roomId, text);
      // Keep anything typed after Send, including a draft in another room.
      if (drafts[roomId] === submitted) drafts[roomId] = '';
    } catch (e) {
      sendErrors[roomId] = `Couldn’t post the message. ${loadErrorText(e)}`;
    } finally {
      sending[roomId] = false;
    }
  }
</script>

<div class="rooms">
  {#if !viewport.isPhone || !selectedId}
  <aside class="list" aria-label="Agent channels" bind:this={roomListEl} style="--list-pane-w:{listW}px">
    <div class="create">
      <input dir="auto"
        bind:this={createEl}
        bind:value={newRoomName}
        placeholder="New channel name"
        aria-label="New channel name"
        onkeydown={(e) => { if (e.key === 'Enter') void createRoom(); }}
      />
      <button class="btn small" disabled={busy || !newRoomName.trim()} onclick={createRoom}>Create</button>
    </div>
    {#if createError}<div class="err" role="alert">{createError}</div>{/if}
    <ul>
      {#each rooms as r (r.room.id)}
        <li>
          <button use:rowMenu
            class="room"
            data-room-id={r.room.id}
            class:active={r.room.id === selectedId}
            aria-current={r.room.id === selectedId ? 'true' : undefined}
            onclick={() => { showingList = false; selectedId = r.room.id; }}
            oncontextmenu={(e) => roomMenu(e, r)}
          >
            <span class="room-name" title={r.room.name}>{r.room.name}</span>
            <span class="meta">
              {plural(r.members.length, 'agent')}
              · {#if r.last_message_at}<RelTime iso={r.last_message_at} />{:else}no messages yet{/if}
            </span>
          </button>
        </li>
      {/each}
    </ul>
  </aside>
  {#if !viewport.isPhone}
    <PaneDivider bind:width={listW} storageKey="personalAgents.rooms.listW" label="Resize the channels list" />
  {/if}
  {/if}

  {#if !viewport.isPhone || selectedId || rooms.length === 0}
  <section class="detail">
    <LoadState
      what="agent channels"
      loading={roomsLoading}
      error={roomsError}
      empty={!selected}
      onretry={reloadRooms}
    >
      {#snippet emptyView()}
        <EmptyState
          icon="comment"
          title="No agent channels yet"
          body="Channels are how personal agents talk to each other. Every message is kept and shown here, and you can post into any channel."
          actionLabel="Create a channel"
          actionIcon="plus"
          onaction={() => createEl?.focus()}
        />
      {/snippet}
      {#if selected}
        <header class="detail-head">
          {#if viewport.isPhone}
            <button class="icon-btn" aria-label="Back to channels" title="Back to channels"
              onclick={backToRooms}><Icon name="chevronLeft" size={16} /></button>
          {/if}
          <strong class="detail-title" title={selected.room.name}>{selected.room.name}</strong>
          <button
            class="icon-btn"
            aria-label="Channel actions for {selected.room.name}"
            title="Channel actions"
            onclick={(e) => roomMenu(e, selected)}><Icon name="more" size={14} /></button>
        </header>

        <div class="members" role="group" aria-label="Member agents">
          {#each selected.members as mid (mid)}
            {@const a = personalAgents.agent(mid)}
            <span class="member">
              <AgentAvatar avatar={a?.avatar} name={a?.name ?? '?'} size={18} round />
              <span class="member-name" title={a?.name ?? 'Removed agent'}>{a?.name ?? 'Removed agent'}</span>
              <button
                class="unlink"
                aria-label="Remove {a?.name ?? 'this agent'} from the channel"
                title="Remove from channel"
                onclick={() => removeMember(mid)}><Icon name="x" size={12} /></button>
            </span>
          {/each}
          {#if nonMembers.length > 0}
            <select
              class="add-member"
              value=""
              aria-label="Add an agent to this channel"
              onchange={(e) => {
                void addMember((e.currentTarget as HTMLSelectElement).value);
                (e.currentTarget as HTMLSelectElement).value = '';
              }}
            >
              <option value="" disabled>Add agent…</option>
              {#each nonMembers as a (a.id)}<option value={a.id}>{a.name}</option>{/each}
            </select>
          {/if}
          {#if selected.members.length === 0}
            <span class="meta">No member agents yet. Add some so they can post here.</span>
          {/if}
        </div>
        {#if selected.members.length > 0}
          <p class="meta how">
            Members find this channel in their instructions from their next run or new chat, then read
            and post here with the channel tools.
          </p>
        {/if}

        <div class="feed" bind:this={feedEl} role="log" aria-label="Channel messages" aria-live="polite">
          <LoadState what="channel messages" loading={personalAgents.messagesLoading[selectedId ?? '']}
            error={personalAgents.messagesError[selectedId ?? '']} empty={messages.length === 0}
            onretry={() => selectedId && void personalAgents.loadMessages(selectedId)}>
            {#snippet emptyView()}
              <div class="meta pad">No messages yet. Member agents read and post here when they run; anything you send is visible to all of them.</div>
            {/snippet}
          {#if hasOlder && messages.length > 0}
            <button class="btn small ghost earlier" onclick={() => void showEarlier()}
              disabled={!!personalAgents.olderLoading[selectedId ?? '']}>
              {personalAgents.olderLoading[selectedId ?? ''] ? 'Loading earlier messages…' : 'Show earlier messages'}
            </button>
          {/if}
          {#each shown as m (m.id)}
            {@const agentMsg = m.author_kind !== 'user'}
            {@const a = agentMsg ? personalAgents.agent(m.author_id) : undefined}
            <div class="msg" class:agent={agentMsg}>
              {#if agentMsg}
                <AgentAvatar avatar={a?.avatar} name={a?.name ?? 'Agent'} size={26} round />
              {:else}
                <span class="me-avatar" aria-hidden="true"><Icon name="user" size={14} /></span>
              {/if}
              <div class="msg-body">
                <div class="msg-head">
                  <strong>{authorName(m)}</strong>
                  {#if agentMsg}<AgentChip />{/if}
                  <span class="meta"><RelTime iso={m.created_at} /></span>
                </div>
                <p class="msg-text">{m.text}</p>
              </div>
            </div>
          {/each}
          </LoadState>
        </div>

        {#if error}<div class="err" role="alert">{error}</div>{/if}
        <div class="composer">
          <textarea dir="auto"
            value={draft}
            oninput={(e) => { if (selectedId) drafts[selectedId] = e.currentTarget.value; }}
            rows="2"
            aria-label="Message to the channel"
            placeholder="Post into the channel (visible to all member agents)…"
            onkeydown={(e) => {
              if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) void send();
            }}
          ></textarea>
          <button class="btn primary" disabled={sending[selectedId ?? ''] || !draft.trim()} onclick={send} title="Send (⌘↩)">Send</button>
        </div>
      {/if}
    </LoadState>
  </section>
  {/if}
</div>

<style>
  .rooms { display: flex; gap: 12px; min-height: 0; flex: 1; align-items: stretch; }
  .list { width: var(--list-pane-w, 280px); flex: 0 0 auto; display: flex; flex-direction: column; gap: 8px; min-height: 0; }
  .create { display: flex; gap: 6px; }
  .create input {
    flex: 1; min-width: 0; background: var(--bg); color: var(--text);
    border: 1px solid var(--border); border-radius: var(--radius-s); padding: 4px 8px; font: inherit; font-size: var(--fs-m);
  }
  .list ul { list-style: none; margin: 0; padding: 2px; display: flex; flex-direction: column; gap: 4px; overflow-y: auto; }
  .room {
    width: 100%; text-align: start; display: flex; flex-direction: column; gap: 2px; min-width: 0;
    background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius-s);
    color: var(--text); padding: 6px 10px; font: inherit; font-size: var(--fs-m); cursor: pointer;
  }
  .room:hover { background: var(--hover); }
  .room.active { background: var(--accent-soft); border-color: transparent; font-weight: 600; }
  .room.active .meta { font-weight: 400; }
  .room-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .detail { flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column; gap: 8px; }
  .detail-head { display: flex; align-items: center; justify-content: space-between; gap: 8px; color: var(--text); min-width: 0; }
  .detail-title { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: var(--fs-l); }
  .members { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .member {
    display: inline-flex; align-items: center; gap: 4px; font-size: var(--fs-s); color: var(--text);
    border: 1px solid var(--border); border-radius: 999px; padding-block: 2px; padding-inline: 2px 4px;
    background: var(--surface); max-width: 220px; min-width: 0;
  }
  .member-name { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .unlink {
    background: none; border: none; color: var(--text-dim); cursor: pointer; padding: 2px;
    display: inline-flex; align-items: center; border-radius: 50%;
  }
  .unlink:hover { color: var(--danger); background: var(--hover); }
  .add-member {
    background: var(--bg); color: var(--text-dim); border: 1px dashed var(--border);
    border-radius: 999px; padding: 2px 8px; font: inherit; font-size: var(--fs-s);
  }
  .feed {
    flex: 1; min-height: 0; overflow-y: auto; display: flex; flex-direction: column; gap: 10px;
    border: 1px solid var(--border); border-radius: var(--radius-m); background: var(--surface); padding: 10px;
  }
  .msg { display: flex; gap: 8px; align-items: flex-start; }
  .earlier { align-self: center; }
  /* Agent-authored: a 2px inline-start rule + an Agent label (patterns.md §2). */
  .msg.agent .msg-body { border-inline-start: 2px solid var(--border-strong); padding-inline-start: 10px; }
  .me-avatar {
    width: 26px; height: 26px; flex: 0 0 auto; border-radius: 50%; display: inline-grid; place-items: center;
    background: var(--surface-2); border: 1px solid var(--border); color: var(--text-dim); box-sizing: border-box;
  }
  .msg-body { min-width: 0; }
  .msg-head { display: flex; gap: 8px; align-items: center; font-size: var(--fs-s); color: var(--text); }
  .msg-text { margin: 2px 0 0; font-size: var(--fs-m); color: var(--text); white-space: pre-wrap; word-break: break-word; }
  .meta { color: var(--text-dim); font-size: var(--fs-s); }
  .how { margin: 0; }
  .pad { padding: 8px; }
  .composer { display: flex; gap: 8px; align-items: flex-end; }
  .composer textarea {
    flex: 1; resize: vertical; background: var(--bg); color: var(--text);
    border: 1px solid var(--border); border-radius: var(--radius-s); padding: 6px 8px; font: inherit; font-size: var(--fs-m);
  }
  .composer textarea:focus-visible, .create input:focus-visible {
    outline: 2px solid var(--accent-text); outline-offset: 1px;
  }
  .err {
    background: var(--danger-soft); color: var(--danger); padding: 8px 12px;
    border-radius: var(--radius-s); font-size: var(--fs-s);
  }
  @media (max-width: 640px) {
    .rooms { flex-direction: column; }
    .list { width: 100%; flex: 1; }
    .detail-head :global(.icon-btn:first-child) { flex: none; }
  }
</style>
