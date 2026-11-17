<script lang="ts">
  // Share with Friends (Live Collaboration & Co-Debugging) modal for the Otto Full IDE.
  // Explicit outward-sharing disclosure (what is shared, who can access, instant revoke),
  // capability-link copy, guest join by link/token, live peer presence, Viewer <-> Editor
  // role switching, and 1-click Follow mode.
  import Modal from '../../lib/components/Modal.svelte';
  import Tabs, { type TabItem } from '../../lib/components/Tabs.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import { announceShown } from '../../lib/announce.svelte';
  import { api } from '../../lib/api/client';
  import { copyWithToast } from '../../lib/clipboard';
  import { confirmer } from '../../lib/confirm.svelte';
  import { plural } from '../../lib/plural';
  import { pollWhileVisible } from '../../lib/poll';
  import { radioKey } from '../../lib/radioKey';
  import { toasts } from '../../lib/toast.svelte';
  import {
    ide,
    ideBasename,
    type IdeCollabPeer,
    type IdeCollabRole,
    type IdeCollabRoom,
  } from './ideStore.svelte';

  interface Props {
    open?: boolean;
    onclose?: () => void;
  }

  let { open, onclose = () => (ide.shareOpen = false) }: Props = $props();
  type ShareTab = 'host' | 'join';

  const isOpen = $derived(open === undefined ? ide.shareOpen : open);

  // Least-privilege default: guests join as Viewers unless upgraded or explicitly chosen.
  let activeTab = $state<ShareTab>('host');
  let sessionTitle = $state('');
  let defaultRole = $state<IdeCollabRole>('viewer');
  let inviteInput = $state('');
  let manualRoomId = $state('');
  let manualToken = $state('');
  let guestName = $state('Teammate');
  let joinValidationError = $state('');
  let roleBusyPeer = $state<string | null>(null);

  function close(): void {
    ide.shareOpen = false;
    onclose();
  }

  $effect(() => {
    if (!isOpen) return;
    joinValidationError = '';
    if (!sessionTitle && ide.target.root) {
      sessionTitle = `${ideBasename(ide.target.root)} pair session`;
    }
    if (!ide.collabRoom && !ide.collabIsGuest) {
      void syncExistingHostRoom();
    }
  });

  $effect(() => {
    if (!isOpen || !ide.collabRoom || ide.collabIsGuest) return;
    const roomId = ide.collabRoom.id;
    const poller = pollWhileVisible(
      async (signal) => {
        const resp = await api.bg.get<{ rooms: IdeCollabRoom[] }>('/ide/collab/rooms', signal);
        const updated = (resp.rooms ?? []).find((r) => r.id === roomId);
        if (updated) ide.collabRoom = updated;
      },
      { ms: 2000, immediate: true },
    );
    return () => poller.stop();
  });

  async function syncExistingHostRoom(): Promise<void> {
    await ide.loadCollabRooms();
    if (!ide.collabRoom && ide.collabRooms.length > 0 && ide.target.root) {
      const match = ide.collabRooms.find((r) => r.target.root === ide.target.root);
      if (match) {
        ide.collabRoom = match;
        // After a reload the invite link comes back from the room list.
        if (!ide.collabShareHash && match.share_hash) ide.collabShareHash = match.share_hash;
      }
    }
  }

  const shareTabs: readonly TabItem<ShareTab>[] = [
    { id: 'host', label: 'Share workspace' },
    { id: 'join', label: 'Join friend’s room' },
  ];

  const fullShareUrl = $derived.by(() => {
    const hash = ide.collabShareHash;
    if (!hash) return '';
    if (typeof window === 'undefined') return hash;
    const cleanHash = hash.startsWith('#') ? hash : `#${hash.replace(/^\//, '')}`;
    return `${window.location.origin}/${cleanHash}`;
  });
  /** The page itself runs on this Mac (the app, Vite, 127.0.0.1): nobody else can open
   *  a link built on it. Opened over the network listener or a public domain, it works. */
  const linkIsLocalOnly = $derived(
    typeof window === 'undefined' ||
      !/^https?:$/.test(window.location.protocol) ||
      /^(localhost|127\.|\[::1\]$)/.test(window.location.hostname),
  );

  function parseShareLink(raw: string): { roomId: string; token: string } | null {
    const text = raw.trim();
    const qIdx = text.indexOf('?');
    if (qIdx < 0) return null;
    const params = new URLSearchParams(text.slice(qIdx + 1));
    const roomId = params.get('collab') ?? params.get('room') ?? '';
    const token = params.get('token') ?? '';
    return roomId && token ? { roomId, token } : null;
  }

  function handleInvitePasteInput(val: string): void {
    inviteInput = val;
    joinValidationError = '';
    const parsed = parseShareLink(val);
    if (parsed) {
      manualRoomId = parsed.roomId;
      manualToken = parsed.token;
    }
  }

  async function handleStartSharing(): Promise<void> {
    if (!ide.hasTarget) return;
    await ide.startCollabRoom(defaultRole, sessionTitle.trim() || undefined);
    if (ide.collabRoom) {
      toasts.success(
        'Live sharing started',
        defaultRole === 'editor'
          ? 'Friends with the link join as editors & co-debuggers.'
          : 'Friends with the link join as read-only viewers.',
      );
    }
  }

  async function handleJoinRoom(): Promise<void> {
    joinValidationError = '';
    const parsed = parseShareLink(inviteInput);
    const roomId = (parsed?.roomId || manualRoomId).trim();
    const token = (parsed?.token || manualToken).trim();
    const name = guestName.trim() || 'Teammate';
    if (!roomId || !token) {
      joinValidationError = 'Paste a collaboration share link or enter both room ID and invite token.';
      return;
    }
    await ide.joinCollabRoom(roomId, token, name);
    if (ide.collabRoom) {
      toasts.success(`Joined "${ide.collabRoom.title}"`, `Connected as ${name}`);
    }
  }

  async function handleRoleChange(peer: IdeCollabPeer, nextRole: IdeCollabRole): Promise<void> {
    if (peer.role === nextRole) return;
    roleBusyPeer = peer.peer_id;
    try {
      const ok = await ide.setCollabPeerRole(peer.peer_id, nextRole);
      if (ok) {
        toasts.success(
          `Updated ${peer.display_name}`,
          nextRole === 'editor' ? 'Can edit files & co-debug' : 'Read-only viewer',
        );
      }
    } finally {
      roleBusyPeer = null;
    }
  }

  async function confirmStopSharing(): Promise<void> {
    if (!ide.collabRoom) return;
    const peerCount = Math.max(0, ide.collabRoom.peers.filter((p) => !p.is_host).length);
    const ok = await confirmer.ask(
      peerCount > 0
        ? `Revoke the invite link and disconnect ${plural(peerCount, 'connected teammate')} immediately?`
        : 'Close this live room and revoke its invite link immediately?',
      { title: 'Stop sharing workspace?', confirmLabel: 'Stop sharing', danger: true },
    );
    if (!ok) return;
    const stopped = await ide.stopCollabRoom();
    if (stopped) {
      toasts.success('Stopped sharing — invite link revoked');
    }
  }

  function handleLeaveGuestRoom(): void {
    ide.leaveCollabRoom();
    toasts.success('Left shared session');
  }

  function peerLocationLabel(peer: IdeCollabPeer): string {
    const cursor = ide.peerCursors.find((c) => c.peer_id === peer.peer_id);
    const p = cursor?.path || peer.active_path;
    const ln = cursor?.active_line || peer.active_line;
    if (!p) return 'Idle in workspace';
    return ln ? `${ideBasename(p)}:${ln}` : ideBasename(p);
  }

  const joinErrorMsg = $derived(joinValidationError || ide.collabError || '');
</script>

{#if isOpen}
  <Modal title="Share with friends" width={680} dismissable={!ide.collabLoading} onclose={close}>
    <div class="sh-shell">
      {#if ide.collabRoom}
        {@const room = ide.collabRoom}
        <section class="sh-live-banner" aria-label="Active collaboration room">
          <div class="sh-live-top">
            <div class="sh-live-title-row">
              <span class="sh-room-title">{room.title}</span>
              <Badge tone="ok" live label="Live" />
              <Badge tone={ide.collabIsGuest ? 'info' : 'accent'} label={ide.collabIsGuest ? 'Guest session' : 'Hosting'} />
              <Badge tone="neutral" variant="outline" label={room.default_role === 'editor' ? 'Default: Editor' : 'Default: Viewer'} />
            </div>
            <p class="sh-sub">
              Shared folder: <code class="sh-mono" dir="ltr">{room.target.root}</code> · Keychain secrets and other workspaces remain private.
            </p>
          </div>

          {#if !ide.collabIsGuest && fullShareUrl}
            <div class="sh-link-box">
              <label class="sh-lbl" for="sh-invite-url">
                Capability invite link (anyone with this link joins as {room.default_role === 'editor' ? 'an editor' : 'a viewer'})
              </label>
              <div class="sh-link-row">
                <input id="sh-invite-url" class="input sh-mono" type="text" dir="ltr" readonly value={fullShareUrl} />
                <button type="button" class="btn primary" onclick={() => copyWithToast(fullShareUrl, 'invite link')}>
                  <Icon name="copy" size={13} />
                  Copy link
                </button>
              </div>
              {#if linkIsLocalOnly}
                <p class="sh-local-warn" role="note" data-testid="ide-share-local-only">
                  <Icon name="warning" size={12} />
                  This link only opens on this Mac. To invite someone, turn on the network listener
                  (Settings › Daemon) and open Otto from that address, then share again.
                </p>
              {/if}
            </div>
          {/if}
        </section>

        <section class="sh-peers-sec" aria-labelledby="sh-peers-heading">
          <div class="sh-sec-head">
            <h3 id="sh-peers-heading" class="sh-sec-title">Connected peers ({room.peers.length})</h3>
            {#if ide.followingPeerId}
              <button type="button" class="btn small ghost" onclick={() => ide.followPeer(null)}>Stop following</button>
            {/if}
          </div>

          {#if room.peers.length === 0}
            <div class="sh-empty-note">Waiting for friends to join with the invite link…</div>
          {:else}
            <div class="sh-peer-list">
              {#each room.peers as peer (peer.peer_id)}
                {@const isSelf = peer.peer_id === ide.collabPeerId}
                {@const isFollowing = ide.followingPeerId === peer.peer_id}
                <div class="sh-peer-item">
                  <span class="sh-peer-dot" style:--peer-dot={peer.color} aria-hidden="true"></span>
                  <div class="sh-peer-info">
                    <div class="sh-peer-name-row">
                      <span class="sh-peer-name">{peer.display_name}</span>
                      {#if peer.is_host}<Badge tone="accent" label="Host" />{/if}
                      {#if isSelf}<Badge tone="neutral" variant="outline" label="You" />{/if}
                      {#if peer.is_host || ide.collabIsGuest}
                        <Badge tone={peer.role === 'editor' ? 'info' : 'neutral'} label={peer.role === 'editor' ? 'Editor & co-debugger' : 'Viewer'} />
                      {/if}
                    </div>
                    <span class="sh-peer-loc sh-mono" dir="ltr">{peerLocationLabel(peer)}</span>
                  </div>

                  <div class="sh-peer-actions">
                    {#if !ide.collabIsGuest && !peer.is_host}
                      <select
                        class="input sh-role-select"
                        aria-label="Access role for {peer.display_name}"
                        disabled={roleBusyPeer === peer.peer_id}
                        value={peer.role}
                        onchange={(e) => handleRoleChange(peer, e.currentTarget.value as IdeCollabRole)}
                      >
                        <option value="viewer">Viewer (read &amp; follow)</option>
                        <option value="editor">Editor (edit &amp; co-debug)</option>
                      </select>
                    {/if}
                    {#if !isSelf}
                      <button
                        type="button"
                        class="btn small"
                        class:ghost={!isFollowing}
                        aria-pressed={isFollowing}
                        onclick={() => ide.followPeer(isFollowing ? null : peer.peer_id)}
                      >
                        <Icon name="eye" size={12} />
                        {isFollowing ? 'Following' : 'Follow'}
                      </button>
                    {/if}
                  </div>
                </div>
              {/each}
            </div>
          {/if}
        </section>
      {:else}
        <Tabs label="Collaboration mode" tabs={shareTabs} value={activeTab} onchange={(id) => (activeTab = id)} idBase="ide-share" />

        {#if activeTab === 'host'}
          <div class="sh-pane" role="tabpanel" id="ide-share-panel-host" aria-labelledby="ide-share-tab-host">
            {#if !ide.hasTarget}
              <EmptyState
                icon="folder"
                title="Open a workspace to share"
                body="Choose a local or SSH project first so friends can browse buffers and co-debug with you."
                actionLabel="Choose target…"
                actionKind="secondary"
                onaction={() => { close(); ide.sshOpen = true; }}
              />
            {:else}
              <LoadState what="collaboration status" loading={false} error={ide.collabError} empty={false} onretry={syncExistingHostRoom}>
                <div class="sh-disclosure">
                  <div class="sh-disc-head">
                    <Icon name="shield" size={14} />
                    <span class="sh-sec-title">What this shares &amp; who can access it</span>
                  </div>
                  <ul class="sh-disc-list">
                    <li><strong>Shared scope:</strong> Open files, live cursor selections, and active debugger state inside <code class="sh-mono" dir="ltr">{ide.target.root}</code>.</li>
                    <li><strong>Stays private:</strong> Your Otto admin token, macOS Keychain secrets, SSH credentials, and other workspaces are never sent to guests.</li>
                    <li><strong>Instant revocation:</strong> Click <strong>Stop sharing</strong> at any time to invalidate the link and disconnect all peers immediately.</li>
                  </ul>
                </div>

                <div class="field">
                  <label for="sh-room-title">Room title</label>
                  <input id="sh-room-title" class="input" type="text" dir="auto" placeholder="Pair debugging session" bind:value={sessionTitle} />
                </div>

                <div class="field">
                  <span class="field-label" id="sh-role-group-lbl">Default guest permission</span>
                  <div class="sh-role-cards" role="radiogroup" aria-labelledby="sh-role-group-lbl">
                    <button
                      type="button"
                      class="sh-role-card"
                      class:is-picked={defaultRole === 'viewer'}
                      role="radio"
                      aria-checked={defaultRole === 'viewer'}
                      tabindex={defaultRole === 'viewer' ? 0 : -1}
                      onkeydown={radioKey}
                      onclick={() => (defaultRole = 'viewer')}
                    >
                      <div class="sh-role-card-top">
                        <Icon name="eye" size={14} />
                        <span class="sh-role-card-name">Viewer — read &amp; follow</span>
                        <Badge tone="ok" label="Recommended" />
                      </div>
                      <p class="sh-sub">
                        Friends can view shared buffers, see live cursors &amp; paused stack frames, and follow you. Promote individual friends to Editor live.
                      </p>
                    </button>

                    <button
                      type="button"
                      class="sh-role-card"
                      class:is-picked={defaultRole === 'editor'}
                      role="radio"
                      aria-checked={defaultRole === 'editor'}
                      tabindex={defaultRole === 'editor' ? 0 : -1}
                      onkeydown={radioKey}
                      onclick={() => (defaultRole = 'editor')}
                    >
                      <div class="sh-role-card-top">
                        <Icon name="edit" size={14} />
                        <span class="sh-role-card-name">Editor — edit, run &amp; co-debug</span>
                      </div>
                      <p class="sh-sub">
                        Friends can edit shared files in real time and step through active debug sessions alongside you.
                      </p>
                    </button>
                  </div>
                </div>
              </LoadState>
            {/if}
          </div>
        {:else}
          <div class="sh-pane" role="tabpanel" id="ide-share-panel-join" aria-labelledby="ide-share-tab-join">
            <div class="field">
              <label for="sh-paste-link">Invite link</label>
              <input
                id="sh-paste-link"
                class="input sh-mono"
                type="url"
                dir="ltr"
                placeholder="https://…/#/ide?collab=room_…&token=…"
                value={inviteInput}
                oninput={(e) => handleInvitePasteInput(e.currentTarget.value)}
              />
              <span class="hint">Pasting a friend’s share URL fills the room ID and invite token automatically.</span>
            </div>

            <div class="sh-two-col">
              <div class="field">
                <label for="sh-join-room">Room ID</label>
                <input id="sh-join-room" class="input sh-mono" type="text" dir="ltr" placeholder="collab_…" bind:value={manualRoomId} />
              </div>
              <div class="field">
                <label for="sh-join-token">Invite token</label>
                <input id="sh-join-token" class="input sh-mono" type="password" dir="ltr" autocomplete="off" placeholder="Capability token" bind:value={manualToken} />
              </div>
            </div>

            <div class="field">
              <label for="sh-guest-name">Your display name</label>
              <input id="sh-guest-name" class="input" type="text" dir="auto" placeholder="Maya" bind:value={guestName} />
            </div>

            {#if joinErrorMsg}
              <div class="sh-err" use:announceShown={{ text: joinErrorMsg, assertive: true }}>{joinErrorMsg}</div>
            {/if}
          </div>
        {/if}
      {/if}
    </div>

    {#snippet footer()}
      {#if ide.collabRoom}
        <div class="sh-footer-bar">
          <div>
            {#if ide.collabIsGuest}
              <button type="button" class="btn danger" onclick={handleLeaveGuestRoom}>Leave room</button>
            {:else}
              <button type="button" class="btn danger" onclick={confirmStopSharing}>
                <Icon name="shieldOff" size={13} />
                Stop sharing
              </button>
            {/if}
          </div>
          <button type="button" class="btn ghost" onclick={close}>Done</button>
        </div>
      {:else if activeTab === 'join'}
        <button type="button" class="btn ghost" disabled={ide.collabLoading} onclick={close}>Cancel</button>
        <button type="button" class="btn primary" disabled={ide.collabLoading} onclick={handleJoinRoom}>
          {ide.collabLoading ? 'Joining…' : 'Join shared session'}
        </button>
      {:else}
        <button type="button" class="btn ghost" disabled={ide.collabLoading} onclick={close}>Cancel</button>
        {#if ide.hasTarget}
          <button type="button" class="btn primary" disabled={ide.collabLoading} onclick={handleStartSharing}>
            <Icon name="share" size={13} />
            {ide.collabLoading ? 'Starting…' : 'Start sharing'}
          </button>
        {/if}
      {/if}
    {/snippet}
  </Modal>
{/if}

<style>
  .sh-shell { display: flex; flex-direction: column; gap: 14px; min-inline-size: 0; }
  .sh-pane { display: flex; flex-direction: column; gap: 6px; padding-block-start: 4px; min-inline-size: 0; }

  .sh-live-banner,
  .sh-peers-sec,
  .sh-disclosure {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px;
    border-radius: var(--radius-m);
    background: var(--surface-2);
    border: 1px solid var(--border);
  }

  .sh-live-top { display: flex; flex-direction: column; gap: 4px; }
  .sh-live-title-row, .sh-sec-head, .sh-disc-head { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
  .sh-sec-head { justify-content: space-between; }
  .sh-room-title { font-size: var(--fs-l); font-weight: 600; color: var(--text); }
  .sh-sec-title { margin: 0; font-size: var(--fs-s); font-weight: 600; color: var(--text); }
  .sh-sub, .sh-empty-note { margin: 0; font-size: var(--fs-xs); color: var(--text-dim); }

  .sh-disc-list {
    margin: 0;
    padding-inline-start: 18px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .sh-disc-list strong { color: var(--text); font-weight: 600; }

  .sh-link-box { display: flex; flex-direction: column; gap: 4px; }
  .sh-lbl { font-size: var(--fs-xs); font-weight: 500; color: var(--text-dim); }
  .sh-link-row { display: flex; align-items: center; gap: 8px; }
  .sh-link-row input { flex: 1; min-inline-size: 0; }
  .sh-local-warn { display: flex; align-items: flex-start; gap: 4px; margin: 0; font-size: var(--fs-xs); color: var(--warning); line-height: 1.45; }

  .sh-role-cards, .sh-two-col { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 10px; }

  .sh-role-card {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
    text-align: start;
    cursor: pointer;
    transition: background var(--dur-fast) var(--ease-out), border-color var(--dur-fast) var(--ease-out);
  }
  .sh-role-card:hover { background: var(--hover); }
  .sh-role-card.is-picked { background: var(--accent-soft); border-color: var(--accent-line); }
  .sh-role-card-top { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; }
  .sh-role-card-name { font-size: var(--fs-s); font-weight: 600; }

  .sh-peer-list { display: flex; flex-direction: column; gap: 6px; }
  .sh-peer-item {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 10px;
    border-radius: var(--radius-s);
    background: var(--surface);
    border: 1px solid var(--border);
    min-inline-size: 0;
  }
  .sh-peer-dot {
    --peer-dot: var(--accent);
    inline-size: 10px;
    block-size: 10px;
    border-radius: 50%;
    background: var(--peer-dot);
    flex-shrink: 0;
  }
  .sh-peer-info { display: flex; flex-direction: column; gap: 2px; min-inline-size: 0; flex: 1; }
  .sh-peer-name-row { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; }
  .sh-peer-name { font-size: var(--fs-s); font-weight: 600; color: var(--text); }
  .sh-peer-loc { color: var(--text-dim); }
  .sh-peer-actions { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; flex-shrink: 0; }
  .sh-role-select { inline-size: auto; font-size: var(--fs-xs); }
  .sh-mono { font-family: var(--font-mono); font-size: var(--fs-xs); }
  .sh-err { font-size: var(--fs-xs); color: var(--danger); }
  .sh-footer-bar { display: flex; align-items: center; justify-content: space-between; gap: 12px; inline-size: 100%; }

  @media (max-width: 640px) {
    .sh-role-cards, .sh-two-col { grid-template-columns: 1fr; }
    .sh-peer-item { flex-wrap: wrap; }
  }
</style>
