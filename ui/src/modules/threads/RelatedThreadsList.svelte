<script lang="ts">
  // Everything a thread is connected to, in one list: linked threads, a
  // delegated thread's parent and workers, its side chat and its session
  // rooms — each with its status and its actions (open, open beside,
  // message), then "Message a thread…" and "Start a room…". Shown in the
  // header's Related threads popover; replaces the old Peers tab and the
  // pane's "To:" relay bar.
  import { api, isAbortError } from '../../lib/api/client';
  import type { RoomSnapshot } from '../../lib/api/room-types';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../lib/components/ProviderIcon.svelte';
  import { router } from '../../lib/router.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import { toastError } from '../../lib/toastError';
  import { recallRoom } from '../rooms/room-access'; // ui-guards: allow — shared room credential lookup
  import { openHostedRoom } from '../rooms/room-window'; // ui-guards: allow — shared native/tab room launcher
  import { collectRelated } from './relatedCollect';
  import { RELATED_KIND_LABEL, relatedStatus, type RelatedRow } from './relatedThreads';
  import { messageThread } from './relatedMessage';
  import { relayTargets } from './threadRelay';
  import { threads } from './threadsStore.svelte';

  interface Props {
    threadId: string;
    onstartroom: (sessionId: string) => void;
    /** Called after an action that moves focus away (the popover closes). */
    ondone?: () => void;
  }
  let { threadId, onstartroom, ondone }: Props = $props();

  const rows = $derived(collectRelated(threadId));

  let rooms = $state<RoomSnapshot[]>([]);
  $effect(() => {
    const ctl = new AbortController();
    void api
      .get<RoomSnapshot[]>('/rooms', ctl.signal)
      .then((list) => { if (!ctl.signal.aborted) rooms = list; })
      .catch((e) => { if (!ctl.signal.aborted && !isAbortError(e)) rooms = []; });
    return () => ctl.abort();
  });
  const ids = $derived(new Set([threadId, ...rows.map((r) => r.id)]));
  const myRooms = $derived(rooms.filter((r) => r.session_id && ids.has(r.session_id)));
  const recent = $derived(threads.peerEvents.filter((e) => e.fromSessionId === threadId || e.toSessionId === threadId).slice(0, 3));

  function open(r: RelatedRow, beside: boolean): void {
    if (r.kind === 'side') threads.openSideChat(threadId);
    else threads.selectThread(r.id, { split: beside });
    ondone?.();
  }

  function pickTarget(e: MouseEvent): void {
    const items: MenuItem[] = [
      { label: 'All active threads', icon: 'send', pinned: true, action: () => void messageThread(threadId, 'broadcast') },
      { separator: true, pinned: true },
      ...relayTargets(threadId).map((t) => ({ label: t.title, action: () => void messageThread(threadId, t.id) })),
    ];
    ctxMenu.show(e, items, { filter: true, filterPlaceholder: 'Find a thread…', maxVisible: 12 });
  }

  async function openRoom(roomId: string): Promise<void> {
    ondone?.();
    if (recallRoom(roomId)) {
      try {
        await openHostedRoom(roomId);
        return;
      } catch (e) {
        toastError('Couldn’t open room window', e);
      }
    }
    router.go(`rooms/${roomId}`);
  }
</script>

<div class="rl" data-testid="related-threads">
  {#if rows.length === 0 && myRooms.length === 0}
    <p class="rl-empty">No related threads yet. An agent’s linked threads, workers, side chats and rooms show here.</p>
  {/if}
  {#if rows.length > 0}
    <ul class="rl-list" aria-label="Related threads">
      {#each rows as r (r.id)}
        {@const st = relatedStatus(r.status)}
        <li class="rl-row" data-testid="related-row-{r.id}">
          <button type="button" class="rl-main" title="Open {r.title}" onclick={() => open(r, false)}>
            <span class="rl-dot" data-tone={st.tone} aria-hidden="true"></span>
            {#if hasProviderIcon(r.provider)}<ProviderIcon provider={r.provider} size={13} />{/if}
            <span class="rl-title ellipsis" dir="auto">{r.title}</span>
            <span class="rl-meta">{RELATED_KIND_LABEL[r.kind]} · {st.text}</span>
          </button>
          {#if r.kind !== 'side'}
            <button type="button" class="icon-btn" aria-label="Open {r.title} beside" title="Open beside" onclick={() => open(r, true)}>
              <Icon name="split" size={13} />
            </button>
            <button type="button" class="icon-btn" aria-label="Message {r.title}" title="Message…" data-testid="related-message-{r.id}" onclick={() => void messageThread(threadId, r.id)}>
              <Icon name="send" size={13} />
            </button>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
  {#if myRooms.length > 0}
    <p class="rl-head">Rooms</p>
    <ul class="rl-list" aria-label="Session rooms">
      {#each myRooms as room (room.room_id)}
        <li class="rl-row">
          <button type="button" class="rl-main" title="Open the room" onclick={() => void openRoom(room.room_id)}>
            <Icon name="people" size={13} />
            <span class="rl-title ellipsis" dir="auto">{room.session_title ?? 'Session room'}</span>
            <span class="rl-meta">{room.members?.filter((m) => m.admission === 'admitted').length ?? 1}/4</span>
          </button>
        </li>
      {/each}
    </ul>
  {/if}
  {#if recent.length > 0}
    <p class="rl-head">Recent messages</p>
    <ul class="rl-list" aria-label="Recent messages between threads" data-testid="related-recent">
      {#each recent as ev (ev.id)}
        <li class="rl-msg" dir="auto"><strong>{ev.fromTitle}</strong> → {ev.toTitle} · {rel(ev.ts)}<span class="rl-sum ellipsis">{ev.summary}</span></li>
      {/each}
    </ul>
  {/if}
  <div class="rl-foot">
    <button type="button" class="btn small" aria-haspopup="menu" data-testid="related-message-any" onclick={pickTarget}>
      <Icon name="send" size={12} />Message a thread…
    </button>
    <button type="button" class="btn small" data-testid="related-start-room" onclick={() => { ondone?.(); onstartroom(threadId); }}>
      <Icon name="people" size={12} />Start a room…
    </button>
  </div>
</div>

<style>
  .rl {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-inline-size: 0;
  }
  .rl-empty {
    margin: 4px 6px;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .rl-head {
    margin: 6px 6px 0;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    font-weight: 600;
  }
  .rl-list {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
  }
  .rl-row {
    display: flex;
    align-items: center;
    gap: 2px;
    min-inline-size: 0;
  }
  .rl-main {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 8px;
    min-inline-size: 0;
    min-block-size: 32px;
    padding-inline: 6px;
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    text-align: start;
    cursor: pointer;
  }
  .rl-main:hover {
    background: var(--hover);
  }
  .rl-main:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  .rl-title {
    flex: 1;
    min-inline-size: 0;
  }
  .rl-meta {
    flex: none;
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .rl-dot {
    inline-size: 7px;
    block-size: 7px;
    border-radius: 50%;
    background: var(--text-dim);
    flex: none;
  }
  .rl-dot[data-tone='live'] {
    background: var(--info);
  }
  .rl-dot[data-tone='ask'] {
    background: var(--warning);
  }
  .rl-dot[data-tone='idle'] {
    background: var(--success);
  }
  .rl-msg {
    display: flex;
    flex-direction: column;
    padding: 4px 6px;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    min-inline-size: 0;
  }
  .rl-sum {
    color: var(--text);
  }
  .rl-foot {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    padding: 6px 6px 2px;
    border-block-start: 1px solid var(--border);
    margin-block-start: 4px;
  }
  @media (pointer: coarse) {
    .rl-main {
      min-block-size: var(--hit-min);
    }
  }
</style>
