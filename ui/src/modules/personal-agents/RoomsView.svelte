<script lang="ts">
  // Agent rooms — the only agent-to-agent transport, always user-visible.
  // Room list + create on the left; the selected room's membership editor,
  // live message feed (WS agent_room_message + `after` paging) and the user
  // post box on the right.
  import RelTime from '../../lib/components/RelTime.svelte';
  import { personalAgents } from '../../lib/stores/personalAgents.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import AgentAvatar from './AgentAvatar.svelte';
  import { loadErrorOf } from './loadError';
  import type { AgentRoomMessage, AgentRoomWithMembers } from '../../lib/api/types';

  let selectedId = $state<string | null>(null);
  let newRoomName = $state('');
  let draft = $state('');
  let busy = $state(false);
  let error = $state('');
  let createError = $state('');
  let feedEl = $state<HTMLElement | null>(null);
  let createEl = $state<HTMLInputElement | null>(null);
  let roomsLoading = $state(true);
  const roomsError = $derived(loadErrorOf(personalAgents, 'roomsError'));

  const rooms = $derived(personalAgents.rooms);
  const selected = $derived(rooms.find((r) => r.room.id === selectedId) ?? null);
  const messages = $derived(selectedId ? (personalAgents.messagesByRoom[selectedId] ?? []) : []);
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
    if (rooms.length > 0 && !rooms.some((r) => r.room.id === selectedId)) {
      selectedId = rooms[0].room.id;
    }
  });
  $effect(() => {
    if (selectedId) void personalAgents.loadMessages(selectedId);
  });
  // Keep the feed pinned to the latest message.
  $effect(() => {
    void messages.length;
    if (feedEl) feedEl.scrollTop = feedEl.scrollHeight;
  });

  function authorName(m: AgentRoomMessage): string {
    if (m.author_kind === 'user') return m.author_id === auth.me?.id ? 'You' : 'User';
    return personalAgents.agent(m.author_id)?.name ?? 'Agent (removed)';
  }


  async function createRoom(): Promise<void> {
    const name = newRoomName.trim();
    if (!name || !ws.currentId) return;
    busy = true;
    createError = '';
    try {
      selectedId = await personalAgents.createRoom(ws.currentId, name);
      newRoomName = '';
    } catch (e) {
      createError = `Couldn’t create the room. ${loadErrorText(e)}`;
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
          const name = await confirmer.promptText('Room name', { title: 'Rename room', confirmLabel: 'Rename', initial: r.room.name });
          if (name) {
            void personalAgents.renameRoom(r.room.id, name).catch((e) => toasts.error('Couldn’t rename the room', loadErrorText(e)));
          }
        },
      },
      {
        label: 'Delete room…',
        icon: 'trash',
        danger: true,
        action: async () => {
          if (await confirmer.ask(`Delete room “${r.room.name}” and its transcript? Member agents stay; only the room and its messages go.`, { title: 'Delete room', confirmLabel: 'Delete' })) {
            void personalAgents.deleteRoom(r.room.id).catch((e) => toasts.error('Couldn’t delete the room', loadErrorText(e)));
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
      toasts.error('Couldn’t add the agent to the room', loadErrorText(e));
    }
  }

  async function removeMember(agentId: string): Promise<void> {
    if (!selectedId) return;
    try {
      await personalAgents.removeMember(selectedId, agentId);
    } catch (e) {
      toasts.error('Couldn’t remove the agent from the room', loadErrorText(e));
    }
  }

  async function send(): Promise<void> {
    const text = draft.trim();
    if (!text || !selectedId) return;
    busy = true;
    error = '';
    try {
      await personalAgents.postMessage(selectedId, text);
      draft = '';
    } catch (e) {
      // Inline next to the composer: the draft is kept, so the fix is a resend.
      error = `Couldn’t post the message. ${loadErrorText(e)}`;
    } finally {
      busy = false;
    }
  }
</script>

<div class="rooms">
  <aside class="list" aria-label="Rooms">
    <div class="create">
      <input
        bind:this={createEl}
        bind:value={newRoomName}
        placeholder="New room name"
        aria-label="New room name"
        onkeydown={(e) => { if (e.key === 'Enter') void createRoom(); }}
      />
      <button class="btn small" disabled={busy || !newRoomName.trim()} onclick={createRoom}>Create</button>
    </div>
    {#if createError}<div class="err" role="alert">{createError}</div>{/if}
    <ul>
      {#each rooms as r (r.room.id)}
        <li>
          <button
            class="room"
            class:active={r.room.id === selectedId}
            aria-current={r.room.id === selectedId ? 'true' : undefined}
            onclick={() => (selectedId = r.room.id)}
            oncontextmenu={(e) => roomMenu(e, r)}
          >
            <span class="room-name" title={r.room.name}>{r.room.name}</span>
            <span class="meta">{r.members.length} agent{r.members.length === 1 ? '' : 's'}</span>
          </button>
        </li>
      {/each}
    </ul>
  </aside>

  <section class="detail">
    <LoadState
      what="rooms"
      loading={roomsLoading}
      error={roomsError}
      empty={!selected}
      onretry={reloadRooms}
    >
      {#snippet emptyView()}
        <EmptyState
          icon="comment"
          title="No rooms yet"
          body="Rooms are how personal agents talk to each other. Every message is kept and shown here, and you can post into any room."
          actionLabel="Create a room"
          actionIcon="plus"
          onaction={() => createEl?.focus()}
        />
      {/snippet}
      {#if selected}
        <header class="detail-head">
          <strong class="detail-title" title={selected.room.name}>{selected.room.name}</strong>
          <button
            class="icon-btn"
            aria-label="Room actions for {selected.room.name}"
            title="Room actions"
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
                aria-label="Remove {a?.name ?? 'this agent'} from the room"
                title="Remove from room"
                onclick={() => removeMember(mid)}><Icon name="x" size={12} /></button>
            </span>
          {/each}
          {#if nonMembers.length > 0}
            <select
              class="add-member"
              value=""
              aria-label="Add an agent to this room"
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

        <div class="feed" bind:this={feedEl} role="log" aria-label="Room messages" aria-live="polite">
          {#each messages as m (m.id)}
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
                  {#if agentMsg}<span class="chip agent-label">Agent</span>{/if}
                  <span class="meta"><RelTime iso={m.created_at} /></span>
                </div>
                <p class="msg-text">{m.text}</p>
              </div>
            </div>
          {:else}
            <div class="meta pad">No messages yet. Member agents post here while they run, and anything you send is visible to all of them.</div>
          {/each}
        </div>

        {#if error}<div class="err" role="alert">{error}</div>{/if}
        <div class="composer">
          <textarea
            bind:value={draft}
            rows="2"
            aria-label="Message to the room"
            placeholder="Post into the room (visible to all member agents)…"
            onkeydown={(e) => {
              if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) void send();
            }}
          ></textarea>
          <button class="btn primary" disabled={busy || !draft.trim()} onclick={send} title="Send (⌘↩)">Send</button>
        </div>
      {/if}
    </LoadState>
  </section>
</div>

<style>
  .rooms { display: flex; gap: 12px; min-height: 0; flex: 1; align-items: stretch; }
  .list { width: 220px; flex: 0 0 auto; display: flex; flex-direction: column; gap: 8px; min-height: 0; }
  .create { display: flex; gap: 6px; }
  .create input {
    flex: 1; min-width: 0; background: var(--bg); color: var(--text);
    border: 1px solid var(--border); border-radius: var(--radius-s); padding: 5px 8px; font: inherit; font-size: var(--fs-m);
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
  .detail { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 8px; }
  .detail-head { display: flex; align-items: center; justify-content: space-between; gap: 8px; color: var(--text); min-width: 0; }
  .detail-title { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: var(--fs-l); }
  .members { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .member {
    display: inline-flex; align-items: center; gap: 5px; font-size: var(--fs-s); color: var(--text);
    border: 1px solid var(--border); border-radius: 999px; padding: 2px 4px 2px 3px;
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
    flex: 1; min-height: 200px; overflow-y: auto; display: flex; flex-direction: column; gap: 10px;
    border: 1px solid var(--border); border-radius: var(--radius-m); background: var(--surface); padding: 10px;
  }
  .msg { display: flex; gap: 8px; align-items: flex-start; }
  /* Agent-authored: a 2px inline-start rule + an Agent label (patterns.md §2). */
  .msg.agent .msg-body { border-inline-start: 2px solid var(--border-strong); padding-inline-start: 10px; }
  .me-avatar {
    width: 26px; height: 26px; flex: 0 0 auto; border-radius: 50%; display: inline-grid; place-items: center;
    background: var(--surface-2); border: 1px solid var(--border); color: var(--text-dim); box-sizing: border-box;
  }
  .msg-body { min-width: 0; }
  .msg-head { display: flex; gap: 8px; align-items: center; font-size: var(--fs-s); color: var(--text); }
  .agent-label { height: 18px; padding: 0 6px; }
  .msg-text { margin: 2px 0 0; font-size: var(--fs-m); color: var(--text); white-space: pre-wrap; word-break: break-word; }
  .meta { color: var(--text-dim); font-size: var(--fs-s); }
  .pad { padding: 8px; }
  .composer { display: flex; gap: 8px; align-items: flex-end; }
  .composer textarea {
    flex: 1; resize: vertical; background: var(--bg); color: var(--text);
    border: 1px solid var(--border); border-radius: var(--radius-s); padding: 6px 8px; font: inherit; font-size: var(--fs-m);
  }
  .composer textarea:focus-visible, .create input:focus-visible {
    outline: 2px solid var(--accent); outline-offset: 1px;
  }
  .err {
    background: var(--danger-soft); color: var(--danger); padding: 8px 12px;
    border-radius: var(--radius-s); font-size: var(--fs-s);
  }
  @media (max-width: 640px) {
    .rooms { flex-direction: column; }
    .list { width: 100%; }
  }
</style>
