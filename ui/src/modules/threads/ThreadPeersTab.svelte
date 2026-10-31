<script lang="ts">
  import { api, isAbortError } from '../../lib/api/client';
  import type { RoomSnapshot } from '../../lib/api/room-types';
  import Badge from '../../lib/components/Badge.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../lib/components/ProviderIcon.svelte';
  import { router } from '../../lib/router.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toastError } from '../../lib/toastError';
  import { recallRoom } from '../rooms/room-access'; // ui-guards: allow — shared room credential lookup
  import { openHostedRoom } from '../rooms/room-window'; // ui-guards: allow — shared native/tab room launcher
  import { delegatedParentId, isThreadSession } from './threadsHelpers';
  import { threads } from './threadsStore.svelte';

  interface Props {
    workspaceId: string | null;
    focusedSessionId: string | null;
    onstartroom: (sessionId: string) => void;
  }
  let { workspaceId, focusedSessionId, onstartroom }: Props = $props();

  let relayText = $state('');
  let sending = $state(false);
  let rooms = $state<RoomSnapshot[]>([]);
  let roomsRetry = $state(0);

  const focusedSession = $derived(focusedSessionId ? (ws.getSession(focusedSessionId) ?? null) : null);
  const peerCandidates = $derived(
    ws.sessions.filter((s) => !s.archived && isThreadSession(s) && s.id !== focusedSessionId),
  );

  // Sync default target when candidates change.
  $effect(() => {
    if (
      threads.peerRelayTarget !== 'broadcast' &&
      !peerCandidates.some((c) => c.id === threads.peerRelayTarget)
    ) {
      threads.peerRelayTarget = peerCandidates[0]?.id ?? 'broadcast';
    }
  });

  // Load active rooms list.
  $effect(() => {
    void roomsRetry;
    const ctl = new AbortController();
    void api
      .get<RoomSnapshot[]>('/rooms', ctl.signal)
      .then((list) => {
        if (!ctl.signal.aborted) rooms = list;
      })
      .catch((e) => {
        if (!ctl.signal.aborted && !isAbortError(e)) rooms = [];
      });
    return () => ctl.abort();
  });

  //Delegation links across current threads.
  const delegationLinks = $derived.by(() => {
    const out: { parentId: string; parentTitle: string; childId: string; childTitle: string; ts: string }[] = [];
    for (const s of ws.sessions) {
      if (s.archived) continue;
      const pid = delegatedParentId(s);
      if (!pid) continue;
      const parent = ws.getSession(pid);
      out.push({
        parentId: pid,
        parentTitle: parent?.title ?? 'Parent thread',
        childId: s.id,
        childTitle: s.title,
        ts: s.created_at,
      });
    }
    return out;
  });

  async function sendRelay(): Promise<void> {
    if (sending || !relayText.trim()) return;
    sending = true;
    try {
      const ok = await threads.sendPeerMessage({
        fromSessionId: focusedSessionId,
        target: threads.peerRelayTarget,
        text: relayText,
        workspaceId,
      });
      if (ok) relayText = '';
    } finally {
      sending = false;
    }
  }

  async function jumpToRoom(roomId: string): Promise<void> {
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

<div class="tpe-wrap" data-testid="inspector-peers-tab">
  <section class="tpe-section" aria-label="Session-to-session message relay">
    <div class="tpe-sec-head">
      <span class="tpe-sec-title">Session-to-Session Relay</span>
    </div>

    <div class="tpe-relay-card">
      <div class="tpe-route-row">
        <span class="tpe-route-label">From</span>
        <div class="tpe-from-badge">
          {#if focusedSession && hasProviderIcon(focusedSession.provider)}
            <ProviderIcon provider={focusedSession.provider} size={12} />
          {:else}
            <Icon name="user" size={12} />
          {/if}
          <span class="ellipsis" dir="auto">{focusedSession?.title ?? 'Operator'}</span>
        </div>
      </div>

      <div class="tpe-route-row">
        <label class="tpe-route-label" for="peers-target-select">To</label>
        <select
          id="peers-target-select"
          class="input tpe-select"
          aria-label="Target thread"
          data-testid="peers-target-select"
          bind:value={threads.peerRelayTarget}
        >
          {#each peerCandidates as cand (cand.id)}
            <option value={cand.id}>{cand.title}</option>
          {/each}
          <option value="broadcast">Broadcast to all active threads</option>
        </select>
      </div>

      <textarea
        class="input tpe-textarea"
        dir="auto"
        rows={3}
        placeholder="Hand off context, ask a peer thread to verify an assumption, or coordinate work…"
        aria-label="Peer message body"
        data-testid="peers-message-input"
        bind:value={relayText}
        onkeydown={(e) => {
          if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
            e.preventDefault();
            void sendRelay();
          }
        }}
      ></textarea>

      <div class="tpe-relay-foot">
        {#if threads.peerRelayTarget !== 'broadcast' && focusedSessionId}
          <button
            type="button"
            class="btn small ghost"
            data-testid="peers-open-split-btn"
            onclick={() => threads.openPairInSplit(focusedSessionId, threads.peerRelayTarget)}
          >
            <Icon name="split" size={12} />
            Open both in Split
          </button>
        {/if}
        <button
          type="button"
          class="btn small primary tpe-send-btn"
          disabled={sending || !relayText.trim()}
          data-testid="peers-send-btn"
          onclick={() => void sendRelay()}
        >
          <Icon name="send" size={12} />
          {sending ? 'Sending…' : 'Send'}
        </button>
      </div>
    </div>
  </section>

  <section class="tpe-section" aria-label="Cross-session conversation timeline">
    <div class="tpe-sec-head">
      <span class="tpe-sec-title">Cross-Session Timeline</span>
      <span class="tpe-count">{threads.peerEvents.length + delegationLinks.length}</span>
    </div>

    {#if threads.peerEvents.length === 0 && delegationLinks.length === 0}
      <EmptyState
        variant="panel"
        icon="threads"
        title="No cross-session messages yet"
        body="Messages relayed between threads, broadcasts, and parent → worker delegations appear here in real time."
      />
    {:else}
      <div class="tpe-timeline" data-testid="peers-timeline">
        {#each threads.peerEvents as ev (ev.id)}
          <article class="tpe-event-card">
            <div class="tpe-ev-head">
              <Badge
                tone={ev.kind === 'broadcast' ? 'info' : 'neutral'}
                variant="outline"
                label={ev.kind === 'broadcast' ? 'Broadcast' : 'Direct'}
              />
              <span class="tpe-ev-pair ellipsis" dir="auto">
                <strong>{ev.fromTitle}</strong> → {ev.toTitle}
              </span>
              <span class="tpe-ev-time">{rel(ev.ts)}</span>
            </div>
            <p class="tpe-ev-body" dir="auto">{ev.summary}</p>
            {#if ev.fromSessionId && ev.toSessionId && ev.fromSessionId !== ev.toSessionId}
              <button
                type="button"
                class="btn small ghost"
                onclick={() => threads.openPairInSplit(ev.fromSessionId!, ev.toSessionId!)}
              >
                <Icon name="split" size={12} />
                Open both in Split
              </button>
            {/if}
          </article>
        {/each}

        {#each delegationLinks as link (link.childId)}
          <article class="tpe-event-card">
            <div class="tpe-ev-head">
              <Badge tone="ok" variant="outline" label="Delegated" />
              <span class="tpe-ev-pair ellipsis" dir="auto">
                <strong>{link.parentTitle}</strong> → {link.childTitle}
              </span>
              <span class="tpe-ev-time">{rel(link.ts)}</span>
            </div>
            <button
              type="button"
              class="btn small ghost"
              onclick={() => threads.openPairInSplit(link.parentId, link.childId)}
            >
              <Icon name="split" size={12} />
              Open both in Split
            </button>
          </article>
        {/each}
      </div>
    {/if}
  </section>

  <section class="tpe-section" aria-label="Session rooms">
    <div class="tpe-sec-head">
      <span class="tpe-sec-title">Session Rooms</span>
      <div class="tpe-room-btns">
        <button
          type="button"
          class="icon-btn"
          aria-label="Refresh rooms"
          title="Refresh rooms"
          onclick={() => roomsRetry++}
        >
          <Icon name="refresh" size={12} />
        </button>
        {#if focusedSessionId}
          <button
            type="button"
            class="btn small"
            onclick={() => onstartroom(focusedSessionId)}
          >
            <Icon name="people" size={12} />
            Create Room
          </button>
        {/if}
      </div>
    </div>

    {#if rooms.length === 0}
      <p class="tpe-empty-note">
        Invite teammates into a live thread with voice, chat, and controlled terminal access.
      </p>
    {:else}
      <div class="tpe-room-list">
        {#each rooms as room (room.room_id)}
          <div class="tpe-room-row">
            <div class="tpe-room-info">
              <Icon name="people" size={13} />
              <span class="ellipsis" dir="auto">{room.session_title ?? 'Session room'}</span>
              <span class="tpe-count">
                {room.members?.filter((m) => m.admission === 'admitted').length ?? 1}/4
              </span>
            </div>
            <button
              type="button"
              class="btn small"
              onclick={() => void jumpToRoom(room.room_id)}
            >
              Open Room
            </button>
          </div>
        {/each}
      </div>
    {/if}
  </section>
</div>

<style>
  .tpe-wrap {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
    min-height: 0;
    overflow-y: auto;
  }
  .tpe-section {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px;
    border-block-end: 1px solid var(--border);
  }
  .tpe-sec-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .tpe-sec-title {
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-dim);
  }
  .tpe-count {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .tpe-relay-card {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border);
    background: var(--surface);
  }
  .tpe-route-row {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .tpe-route-label {
    inline-size: 36px;
    font-size: var(--fs-xs);
    font-weight: 500;
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .tpe-from-badge {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    padding: 4px 8px;
    border-radius: var(--radius-s);
    background: var(--surface-2);
    font-size: var(--fs-xs);
    font-weight: 500;
    color: var(--text);
  }
  .tpe-select {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-xs);
    padding-block: 4px;
  }
  .tpe-textarea {
    width: 100%;
    resize: vertical;
    min-block-size: 64px;
    font-size: var(--fs-s);
    line-height: 1.45;
    unicode-bidi: plaintext;
    text-align: start;
  }
  .tpe-relay-foot {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 6px;
  }
  .tpe-send-btn {
    margin-inline-start: auto;
  }
  .tpe-timeline {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .tpe-event-card {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 6px;
    padding: 8px 10px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border);
    background: var(--surface);
  }
  .tpe-ev-head {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    min-width: 0;
  }
  .tpe-ev-pair {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-xs);
    color: var(--text);
  }
  .tpe-ev-pair strong {
    font-weight: 600;
  }
  .tpe-ev-time {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .tpe-ev-body {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text);
    line-height: 1.45;
    overflow-wrap: anywhere;
  }
  .tpe-room-btns {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .tpe-empty-note {
    margin: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    line-height: 1.5;
  }
  .tpe-room-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .tpe-room-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 8px 10px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface);
  }
  .tpe-room-info {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    font-size: var(--fs-s);
    font-weight: 500;
  }
</style>
