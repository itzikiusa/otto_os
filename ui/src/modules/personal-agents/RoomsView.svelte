<script lang="ts">
  // Agent rooms — the only agent-to-agent transport, always user-visible.
  // Room list + create on the left; the selected room's membership editor,
  // live message feed (WS agent_room_message + `after` paging) and the user
  // post box on the right.
  import { personalAgents } from '../../lib/stores/personalAgents.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import type { AgentRoomMessage, AgentRoomWithMembers } from '../../lib/api/types';

  let selectedId = $state<string | null>(null);
  let newRoomName = $state('');
  let draft = $state('');
  let busy = $state(false);
  let error = $state('');
  let feedEl = $state<HTMLElement | null>(null);

  const rooms = $derived(personalAgents.rooms);
  const selected = $derived(rooms.find((r) => r.room.id === selectedId) ?? null);
  const messages = $derived(selectedId ? (personalAgents.messagesByRoom[selectedId] ?? []) : []);
  const nonMembers = $derived(
    personalAgents.agents.filter((a) => !(selected?.members ?? []).includes(a.id)),
  );

  $effect(() => {
    if (ws.currentId) void personalAgents.loadRooms(ws.currentId);
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

  function authorAvatar(m: AgentRoomMessage): string {
    if (m.author_kind === 'user') return '🙂';
    const a = personalAgents.agent(m.author_id);
    return a?.avatar || a?.name.slice(0, 1) || '🤖';
  }

  async function createRoom(): Promise<void> {
    const name = newRoomName.trim();
    if (!name || !ws.currentId) return;
    busy = true;
    error = '';
    try {
      selectedId = await personalAgents.createRoom(ws.currentId, name);
      newRoomName = '';
    } catch (e) {
      error = e instanceof Error ? e.message : 'Create failed';
    } finally {
      busy = false;
    }
  }

  function roomMenu(e: MouseEvent | KeyboardEvent, r: AgentRoomWithMembers): void {
    ctxMenu.show(e, [
      {
        label: 'Rename',
        icon: 'edit',
        action: () => {
          const name = prompt('Room name', r.room.name)?.trim();
          if (name) void personalAgents.renameRoom(r.room.id, name).catch(() => {});
        },
      },
      {
        label: 'Delete room',
        icon: 'trash',
        danger: true,
        action: () => {
          if (confirm(`Delete room "${r.room.name}" and its transcript?`)) {
            void personalAgents.deleteRoom(r.room.id).catch(() => {});
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
      error = e instanceof Error ? e.message : 'Add member failed';
    }
  }

  async function removeMember(agentId: string): Promise<void> {
    if (!selectedId) return;
    try {
      await personalAgents.removeMember(selectedId, agentId);
    } catch (e) {
      error = e instanceof Error ? e.message : 'Remove member failed';
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
      error = e instanceof Error ? e.message : 'Post failed';
    } finally {
      busy = false;
    }
  }
</script>

<div class="rooms">
  <aside class="list">
    <div class="create">
      <input
        bind:value={newRoomName}
        placeholder="New room name"
        onkeydown={(e) => { if (e.key === 'Enter') void createRoom(); }}
      />
      <button class="btn small primary" disabled={busy || !newRoomName.trim()} onclick={createRoom}>Create</button>
    </div>
    <ul>
      {#each rooms as r (r.room.id)}
        <li>
          <button
            class="room"
            class:active={r.room.id === selectedId}
            onclick={() => (selectedId = r.room.id)}
            oncontextmenu={(e) => roomMenu(e, r)}
          >
            <span class="room-name">{r.room.name}</span>
            <span class="meta">{r.members.length} agent{r.members.length === 1 ? '' : 's'}</span>
          </button>
        </li>
      {/each}
    </ul>
  </aside>

  <section class="detail">
    {#if error}<div class="err" role="alert">{error}</div>{/if}
    {#if !selected}
      <EmptyState
        icon="comment"
        title="No rooms yet"
        body="Rooms are how personal agents talk to each other — every message is persisted and visible here, and you can post into any room."
      />
    {:else}
      <header class="detail-head">
        <strong>{selected.room.name}</strong>
        <button class="btn small" onclick={(e) => roomMenu(e, selected)}>⋯</button>
      </header>

      <div class="members">
        {#each selected.members as mid (mid)}
          {@const a = personalAgents.agent(mid)}
          <span class="member">
            <span aria-hidden="true">{a?.avatar || '🤖'}</span>
            {a?.name ?? mid}
            <button class="unlink" title="Remove from room" onclick={() => removeMember(mid)}>×</button>
          </span>
        {/each}
        {#if nonMembers.length > 0}
          <select
            class="add-member"
            value=""
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
          <span class="meta">No member agents yet — add some so they can post here.</span>
        {/if}
      </div>

      <div class="feed" bind:this={feedEl}>
        {#each messages as m (m.id)}
          <div class="msg" class:mine={m.author_kind === 'user'}>
            <span class="msg-avatar" aria-hidden="true">{authorAvatar(m)}</span>
            <div class="msg-body">
              <div class="msg-head">
                <strong>{authorName(m)}</strong>
                <span class="meta">{m.created_at}</span>
              </div>
              <p class="msg-text">{m.text}</p>
            </div>
          </div>
        {:else}
          <div class="meta pad">No messages yet.</div>
        {/each}
      </div>

      <div class="composer">
        <textarea
          bind:value={draft}
          rows="2"
          placeholder="Post into the room (visible to all member agents)…"
          onkeydown={(e) => {
            if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) void send();
          }}
        ></textarea>
        <button class="btn primary" disabled={busy || !draft.trim()} onclick={send}>Send</button>
      </div>
    {/if}
  </section>
</div>

<style>
  .rooms { display: flex; gap: 0.75rem; min-height: 0; flex: 1; align-items: stretch; }
  .list { width: 220px; flex: 0 0 auto; display: flex; flex-direction: column; gap: 0.5rem; }
  .create { display: flex; gap: 0.35rem; }
  .create input {
    flex: 1; min-width: 0; background: var(--bg); color: var(--text);
    border: 1px solid var(--border); border-radius: var(--radius-s); padding: 0.4rem 0.5rem; font: inherit; font-size: 0.85rem;
  }
  .list ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 0.25rem; overflow-y: auto; }
  .room {
    width: 100%; text-align: start; display: flex; flex-direction: column; gap: 0.1rem;
    background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius-s);
    color: var(--text); padding: 0.45rem 0.6rem; font: inherit; font-size: 0.88rem; cursor: pointer;
  }
  .room.active { border-color: var(--accent); background: color-mix(in srgb, var(--accent) 8%, var(--surface)); }
  .room:focus-visible { outline: 2px solid color-mix(in srgb, var(--accent) 70%, transparent); outline-offset: 1px; }
  .room-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .detail { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 0.5rem; }
  .detail-head { display: flex; align-items: center; justify-content: space-between; color: var(--text); }
  .members { display: flex; align-items: center; gap: 0.4rem; flex-wrap: wrap; }
  .member {
    display: inline-flex; align-items: center; gap: 0.3rem; font-size: 0.78rem; color: var(--text);
    border: 1px solid var(--border); border-radius: 999px; padding: 0.1rem 0.3rem 0.1rem 0.5rem;
    background: var(--surface);
  }
  .unlink { background: none; border: none; color: var(--text-dim); cursor: pointer; font-size: 0.85rem; padding: 0 0.2rem; }
  .unlink:hover { color: var(--status-exited); }
  .add-member {
    background: var(--bg); color: var(--text-dim); border: 1px dashed var(--border);
    border-radius: 999px; padding: 0.15rem 0.5rem; font: inherit; font-size: 0.78rem;
  }
  .feed {
    flex: 1; min-height: 200px; overflow-y: auto; display: flex; flex-direction: column; gap: 0.5rem;
    border: 1px solid var(--border); border-radius: var(--radius-m); background: var(--surface); padding: 0.6rem;
  }
  .msg { display: flex; gap: 0.5rem; align-items: flex-start; }
  .msg-avatar {
    width: 1.6rem; height: 1.6rem; flex: 0 0 auto; border-radius: 50%; display: inline-flex;
    align-items: center; justify-content: center; font-size: 0.85rem;
    background: color-mix(in srgb, var(--accent) 12%, transparent);
  }
  .msg.mine .msg-avatar { background: color-mix(in srgb, var(--status-working) 16%, transparent); }
  .msg-body { min-width: 0; }
  .msg-head { display: flex; gap: 0.5rem; align-items: baseline; font-size: 0.8rem; color: var(--text); }
  .msg-text { margin: 0.1rem 0 0; font-size: 0.85rem; color: var(--text); white-space: pre-wrap; word-break: break-word; }
  .meta { color: var(--text-dim); font-size: 0.75rem; }
  .pad { padding: 0.5rem; }
  .composer { display: flex; gap: 0.5rem; align-items: flex-end; }
  .composer textarea {
    flex: 1; resize: vertical; background: var(--bg); color: var(--text);
    border: 1px solid var(--border); border-radius: var(--radius-s); padding: 0.45rem 0.55rem; font: inherit; font-size: 0.88rem;
  }
  .composer textarea:focus-visible, .create input:focus-visible {
    outline: 2px solid color-mix(in srgb, var(--accent) 70%, transparent); outline-offset: 1px;
  }
  .err {
    background: color-mix(in srgb, var(--status-exited) 12%, transparent);
    color: var(--status-exited); padding: 0.5rem 0.75rem;
    border-radius: var(--radius-s); font-size: 0.85rem;
  }
  @media (max-width: 700px) {
    .rooms { flex-direction: column; }
    .list { width: 100%; }
  }
</style>
