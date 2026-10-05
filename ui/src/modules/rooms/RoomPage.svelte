<script lang="ts">
  import { onMount } from 'svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Terminal from '../../lib/components/Terminal.svelte';
  import { PRIMARY_SCROLLBACK } from '../../lib/components/termFlow';
  import Modal from '../../lib/components/Modal.svelte';
  import RoomRecap from './RoomRecap.svelte';
  import RoomMedia from './RoomMedia.svelte';
  import RoomParticipants from './RoomParticipants.svelte';
  import RoomChat from './RoomChat.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { copyTextOrThrow } from '../../lib/clipboard';
  import { router } from '../../lib/router.svelte';
  import { applyRoomEvent } from './room-state';
  import { RoomClient, joinRoom, type RoomConnection } from './room-client';
  import { recallRoom, rememberRoom, forgetRoom, roomInvite } from './room-access';
  import type { RoomSnapshot, RoomAction, RoomEvent, RoomInvite } from '../../lib/api/room-types';
  let { roomId, guest = false }: {roomId: string; guest?: boolean} = $props();
  let mediaPanel: RoomMedia | undefined = $state();
  let room = $state<RoomSnapshot | null>(null), connection = $state<RoomConnection>('connecting');
  let recapPanel: RoomRecap | undefined = $state();
  let client = $state<RoomClient | null>(null), joining = $state(false), name = $state(''), error = $state(''), ended = $state('');
  let waitingForRecap = $state(false);
  let finishing = $state(false), needsName = $state(false), inviteOpen = $state(false), inviteRole = $state<'viewer' | 'editor'>('viewer');
  let invitation = $state<RoomInvite | null>(null), inviting = $state(false), copied = $state(false);
  let tab = $state<'session' | 'conversation'>('session');
  const host = $derived(room?.member_id === room?.host_member_id && !!room?.host_member_id);
  const self = $derived(room?.members?.find(m => m.id === room?.member_id));
  const driver = $derived(room?.members?.find(m => m.id === room?.driver_member_id));
  const connected = $derived(connection === 'connected');
  const canType = $derived(!finishing && connected && room?.admission === 'admitted' && room?.driver_member_id === room?.member_id);
  const origin = $derived(recallRoom(roomId)?.origin ?? location.origin);
  onMount(() => {
    if (recallRoom(roomId)) attach();
    else if (guest && roomInvite(roomId)) { needsName = true; connection = 'disconnected'; }
    else { ended = 'This room credential is no longer available in this window. Ask the host for a new invitation.'; connection = 'ended'; }
    const unguard = router.guard(async (to) => {
      if (to === `${guest ? 'room' : 'rooms'}/${roomId}` || connection === 'ended' || needsName) return true;
      return confirmer.ask('You will disconnect from this room. The underlying session keeps running.', {title: 'Leave this room view?', confirmLabel: 'Leave view'});
    });
    return () => { unguard(); client?.dispose(); };
  });
  function attach() {
    const stored = recallRoom(roomId); if (!stored) return;
    client?.dispose();
    client = new RoomClient(stored.origin, stored.credential, receive, (status) => { connection = status; if (status !== 'connected' && waitingForRecap) { waitingForRecap = false; finishing = false; } });
    client.connect();
  }
  async function join() {
    const invite = roomInvite(roomId); if (!invite || !name.trim()) return;
    joining = true; error = '';
    try { rememberRoom(location.origin, await joinRoom(location.origin, roomId, invite, name.trim())); needsName = false; attach(); }
    catch (e) { error = e instanceof Error ? e.message : 'Could not join. Ask the host for a fresh invitation.'; }
    finally { joining = false; }
  }
  function receive(event: RoomEvent) {
    mediaPanel?.handleEvent(event);
    if (event.type === 'snapshot') { room = event.room; error = ''; if (waitingForRecap && ['stopped', 'paused'].includes(room.recap?.state ?? 'stopped')) { waitingForRecap = false; if (!send({type: 'end'})) finishing = false; } }
    else if (event.type === 'ended') { ended = event.reason; room = null; forgetRoom(roomId); }
    else if (event.type === 'error') { error = event.message; finishing = false; waitingForRecap = false; }
    else if (room) room = applyRoomEvent(room, event);
  }
  function send(action: RoomAction): boolean {
    if (!client?.send(action)) { error = 'Connection interrupted. Reconnect and try again.'; return false; }
    return true;
  }
  function terminalFrame(frame: unknown): unknown | null {
    if (!frame || typeof frame !== 'object' || !('type' in frame)) return null;
    const data = frame as {type: string; data?: string; cols?: number; rows?: number; lines?: number; window?: number; bytes?: number};
    // Backpressure controls only this viewer's output stream, never PTY authority.
    if (data.type === 'pause' || data.type === 'resume') return {type: data.type};
    // Credit flow control (same window as /ws/term, r3-10-06): no grant needed.
    if (data.type === 'credit') return {type: 'credit', window: data.window};
    if (data.type === 'ack') return {type: 'ack', bytes: data.bytes};
    if (data.type === 'scrollback' || data.type === 'resync') return {type: data.type, lines: data.lines};
    if (!canType || room?.grant_epoch === undefined) return null;
    if (data.type === 'input') return {type: 'input', data: data.data, grant_epoch: room.grant_epoch};
    if (data.type === 'resize') return {type: 'resize', cols: data.cols, rows: data.rows, grant_epoch: room.grant_epoch};
    return null;
  }
  async function leave() {
    const label = host ? 'End room' : 'Leave room';
    if (!await confirmer.ask(host ? 'Everyone will lose access to this room. Your session and agent will keep running.' : 'Your terminal, chat and media access will close.', {title: `${label}?`, confirmLabel: label, danger: host})) return;
    if (host) {
      finishing = true;
      await recapPanel?.finishCapture();
      if (room?.recap?.state === 'capturing' || room?.recap?.state === 'finalizing') {
        waitingForRecap = true;
        if (room.recap.state !== 'finalizing' && !send({type: 'recap_stop'})) { waitingForRecap = false; finishing = false; }
      } else if (!send({type: 'end'})) finishing = false;
      return;
    }
    send({type: 'leave'});
    client?.dispose(); room = null; connection = 'ended'; ended = host ? 'The room has ended. The session is still running.' : 'You left the room.'; forgetRoom(roomId);
  }
  async function makeInvite() {
    inviting = true; error = ''; copied = false;
    try { const { api } = await import('../../lib/api/client'); invitation = await api.post<RoomInvite>(`/rooms/${encodeURIComponent(roomId)}/invites`, {role: inviteRole}); }
    catch (e) { error = e instanceof Error ? e.message : 'Could not create an invitation. Try again.'; }
    finally { inviting = false; }
  }
  async function copyInvite() {
    if (!invitation) return;
    try { await copyTextOrThrow(invitation.url); copied = true; } catch { error = 'Could not copy. Select and copy the invitation link below.'; }
  }
</script>
<div class="room-page">
  <PageHeader title={room?.session_title ?? 'Session room'} subtitle={room ? `${room.members?.filter(m => m.admission === 'admitted').length ?? 1}/4 people` : ''}>
    {#snippet actions()}{#if room?.admission === 'admitted'}
      <button class="btn small danger" data-overflow="-2" disabled={finishing} onclick={leave}>{finishing ? waitingForRecap ? 'Finishing recap…' : 'Ending…' : host ? 'End room…' : 'Leave room…'}</button>
      {#if host}<button class="btn small primary" disabled={!connected} onclick={() => { invitation = null; inviteOpen = true; }}>Invite someone…</button>{/if}
    {/if}{/snippet}
  </PageHeader>
  <PageBody fill padded={false}>
    {#if needsName}<div class="join-form"><h2>Join this session</h2><p>Connecting to <strong>{origin}</strong></p><p>The host will see your display name and decide whether to admit you. Admitted participants can see your messages and shared media.</p>
      <form onsubmit={(event) => { event.preventDefault(); void join(); }}><label>Your display name <input bind:value={name} maxlength="80" autocomplete="nickname" /></label>
        {#if error}<p role="alert">{error}</p>{/if}<button class="btn primary" disabled={joining || !name.trim()}>{joining ? 'Requesting entry…' : 'Request entry'}</button>
      </form></div>
    {:else if ended}<EmptyState variant="page" icon="people" title="Room closed" body={ended} />
    {:else if !room}{#if error}<p class="error" role="alert">{error}</p>{/if}<EmptyState variant="page" icon="people" title={connection === 'connecting' ? 'Connecting to the room…' : 'Connection interrupted'} body="Terminal access is paused until your room connection is restored." actionLabel="Retry" onaction={() => client?.connect()} />
    {:else if room.admission === 'pending'}<EmptyState variant="page" icon="people" title="Waiting for the host" body="Your request has been sent. Terminal content, people and messages remain private until you are admitted." actionLabel="Cancel request" onaction={leave} />
    {:else}
      {#if !connected}<div class="error" role="status">Connection interrupted. Terminal access is paused.<button class="btn small" onclick={() => client?.connect()}>Retry</button></div>{/if}
      <div class="room-status" role="status"><span>{self?.role === 'host' ? 'Host' : self?.role === 'editor' ? 'Can control' : 'View only'} · {driver?.name ?? 'Host'} controls the terminal</span>
        {#if host && !canType}<button class="btn small" onclick={() => send({type: 'grant_control', member_id: room!.member_id})}>Take back control</button>
        {:else if !host && canType}<button class="btn small" onclick={() => send({type: 'release_control'})}>Release control</button>
        {:else if self?.role === 'editor'}<button class="btn small" disabled={self.control_requested} onclick={() => send({type: 'request_control'})}>{self.control_requested ? 'Control requested' : 'Request control'}</button>{/if}
      </div>
      {#if error}<div class="error" role="alert">{error}<button class="btn small" onclick={() => error = ''}>Dismiss</button></div>{/if}
      <RoomRecap bind:this={recapPanel} {room} {connected} {send} audioSources={() => mediaPanel?.getRecapAudioSources() ?? new Map()} screenStreams={() => mediaPanel?.getPresentationStreams() ?? new Map()} />
      <nav class="mobile-tabs" aria-label="Room view"><button class="btn" aria-pressed={tab === 'session'} onclick={() => tab = 'session'}>Session</button><button class="btn" aria-pressed={tab === 'conversation'} onclick={() => tab = 'conversation'}>People & chat</button></nav>
      <div class="room-workspace"><main class:mobile-hidden={tab !== 'session'}><RoomMedia bind:this={mediaPanel} {room} {send} {connected} /><div class="terminal-pane">{#if room.session_id && client && connected}<Terminal sessionId={room.session_id} socketFactory={() => client!.terminal()} transformFrame={terminalFrame} readOnly={!canType} readOnlyReason={self?.role === 'viewer' ? 'View only. The host decides who can control the terminal.' : `${driver?.name ?? 'The host'} currently controls the terminal.`} showToolbar={false} scrollback={PRIMARY_SCROLLBACK} preferDom />{/if}</div></main>
        <aside class:mobile-hidden={tab !== 'conversation'}><RoomParticipants {room} {send} disabled={!connected} /><RoomChat messages={room.messages ?? []} {send} {connected} /></aside>
      </div>
    {/if}
  </PageBody>
</div>
{#if inviteOpen}<Modal title="Invite someone" onclose={() => inviteOpen = false}>
  <p>A single-use invitation expires after ten minutes. You still approve admission and terminal control.</p>
  <label>Maximum access <select bind:value={inviteRole}><option value="viewer">View only</option><option value="editor">Can control when granted</option></select></label>
  {#if invitation}<label>Invitation link <input readonly value={invitation.url} aria-label="Invitation link" /></label><p>Expires {new Date(invitation.expires_at).toLocaleTimeString()}.</p>{/if}
  {#if error}<p role="alert">{error}</p>{/if}
  {#snippet footer()}<button class="btn" onclick={() => inviteOpen = false}>Done</button>{#if invitation}<button class="btn primary" onclick={copyInvite}>{copied ? 'Copied' : 'Copy invitation'}</button>{:else}<button class="btn primary" disabled={inviting} onclick={makeInvite}>{inviting ? 'Creating…' : 'Create invitation'}</button>{/if}{/snippet}
</Modal>{/if}
<style>
  .room-page { height: 100%; min-width: 0; display: flex; flex-direction: column; background: var(--bg); color: var(--text); }
  .join-form { padding: 24px; max-width: 680px; overflow: auto; } h2 { font-size: var(--fs-l); } p { line-height: 1.5; }
  form, label { display: grid; gap: 8px; } form { gap: 16px; } form button { justify-self: start; } input { width: 100%; }
  .room-status, .error { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 8px; padding: 8px 16px; border-bottom: 1px solid var(--border); font-size: var(--fs-s); }
  .room-status { background: var(--surface); } .error, [role='alert'] { color: var(--danger); }
  .room-workspace { flex: 1; min-height: 0; display: grid; grid-template-columns: minmax(0, 1fr) minmax(280px, 340px); }
  main { min-width: 0; min-height: 0; display: flex; flex-direction: column; } .terminal-pane { flex: 1; min-height: 160px; }
  aside { min-width: 0; overflow-y: auto; border-inline-start: 1px solid var(--border); display: flex; flex-direction: column; background: var(--surface); }
  .mobile-tabs { display: none; }
  @media (max-width: 1024px) { .room-workspace { grid-template-columns: minmax(0, 1fr); } .mobile-tabs { display: flex; gap: 8px; padding: 8px 12px; border-bottom: 1px solid var(--border); } .mobile-hidden { display: none; } aside { border-inline-start: 0; } .join-form { padding: 16px; } }
</style>
