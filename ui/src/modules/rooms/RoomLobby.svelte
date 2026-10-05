<script lang="ts">
  import RoomSettingsModal from './RoomSettingsModal.svelte';
  import { onMount } from 'svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import {loadErrorText} from '../../lib/loadError';
  import Modal from '../../lib/components/Modal.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { api } from '../../lib/api/client';
  import { confirmer } from '../../lib/confirm.svelte';
  import { router } from '../../lib/router.svelte';
  import { registry } from '../../lib/commands.svelte';
  import { parseRoomInvite } from './room-state';
  import { recallRoom } from './room-access';
  import { openHostedRoom } from './room-window';
  import type { RoomSnapshot } from '../../lib/api/room-types';
  let settingsOpen = $state(false);
  let rooms = $state<RoomSnapshot[]>([]);
  let loading = $state(true), error = $state(''), joinOpen = $state(false), link = $state(''), joinError = $state('');
  let opening = $state(''), openError = $state(''), joining = $state(false), lastOpened = $state('');
  async function openRoom(id: string) {
    opening = id; lastOpened = id; openError = '';
    try { await openHostedRoom(id); }
    catch (e) { openError = loadErrorText(e); }
    finally { opening = ''; }
  }
  onMount(() => { void load(); });
  // ⌘K verbs while the lobby is open.
  $effect(() =>
    registry.register('rooms', [
      { id: 'rooms.join', title: 'Join room…', group: 'Rooms', keywords: 'invite link connect session room', run: () => (joinOpen = true) },
      { id: 'rooms.recaps', title: 'Open recap archives', group: 'Rooms', keywords: 'room recap summary history', run: () => router.go('rooms/recaps') },
      ...(auth.isRoot ? [{ id: 'rooms.settings', title: 'Room connection settings…', group: 'Rooms', keywords: 'relay host server', run: () => (settingsOpen = true) }] : []),
      ...rooms.filter((r) => recallRoom(r.room_id)).map((r) => ({
        id: `rooms.open.${r.room_id}`, title: `Open room ${r.session_title ?? 'Session room'}`, group: 'Rooms', keywords: 'room session', run: () => void openRoom(r.room_id),
      })),
    ]),
  );
  async function load() {
    loading = true; error = '';
    try { rooms = await api.get<RoomSnapshot[]>('/rooms'); }
    catch (e) { error = loadErrorText(e); }
    finally { loading = false; }
  }
  async function join() {
    joinError = ''; joining = true;
    try {
      const invitation = parseRoomInvite(link);
      if (!await confirmer.ask(`Connect to ${invitation.origin}? Your display name and room activity will be shared with this host and admitted participants.`, {title: 'Join room', confirmLabel: 'Join'})) return;
      if ('__TAURI_INTERNALS__' in window) {
        const { invoke } = await import('@tauri-apps/api/core');
        await invoke<string>('open_room_window', {url: invitation.url});
        joinOpen = false; link = '';
      } else { window.location.assign(invitation.url); }
    } catch (e) { joinError = e instanceof Error ? e.message : 'Could not open this room. Try again.'; }
    finally { joining = false; }
  }
</script>
<div class="rooms-page">
  <PageHeader title="Rooms" subtitle="Work together in a session">
    {#snippet actions()}<button class="btn" onclick={() => router.go('rooms/recaps')}>Recap archives</button>{#if auth.isRoot}<button class="btn" onclick={() => settingsOpen = true}>Connection settings…</button>{/if}<button class="btn primary" onclick={() => joinOpen = true}>Join room…</button>{/snippet}
  </PageHeader>
  <PageBody>
    {#if openError}<p role="alert">Couldn’t open the room. {openError} <button class="btn small" disabled={!!opening || !lastOpened} onclick={() => openRoom(lastOpened)}>Retry</button></p>{/if}
    <LoadState what="rooms" variant="page" {loading} {error} empty={!rooms.length} onretry={load}>
    {#snippet emptyView()}<EmptyState variant="page" icon="people" title="Bring someone into your session" body="Open a running session’s More menu and choose Start room. You control admission and terminal access." />{/snippet}
    <div class="room-list">{#each rooms as room (room.room_id)}
      <article><h2>{room.session_title ?? 'Session room'}</h2><p>{room.members?.filter(m => m.admission === 'admitted').length ?? 1} of 4 people</p>
        {#if recallRoom(room.room_id)}<button class="btn" disabled={!!opening} onclick={() => openRoom(room.room_id)}>{opening === room.room_id ? 'Opening…' : 'Open room'}</button>
        {:else}<p>Open in the window where you started the room.</p>{/if}
      </article>
    {/each}</div>
    </LoadState>
  </PageBody>
</div>
{#if settingsOpen}<RoomSettingsModal onclose={() => settingsOpen = false} />{/if}
{#if joinOpen}<Modal title="Join a room" onclose={() => joinOpen = false}>
  <label>Invitation link <input type="url" bind:value={link} placeholder="https://host/#/room/…" autocomplete="off" spellcheck="false" /></label>
  <p>You will see the destination before connecting.</p>
  {#if joinError}<p role="alert">{joinError}</p>{/if}
  {#snippet footer()}<button class="btn" onclick={() => joinOpen = false}>Cancel</button><button class="btn primary" disabled={!link.trim() || joining} onclick={join}>{joining ? 'Joining…' : 'Continue'}</button>{/snippet}
</Modal>{/if}
<style>
  .rooms-page { display: flex; flex-direction: column; height: 100%; min-width: 0; }
  label { display: grid; gap: 8px; } input { width: 100%; }
  .room-list { display: grid; gap: 12px; grid-template-columns: repeat(auto-fit, minmax(min(280px, 100%), 1fr)); }
  article { border: 1px solid var(--border); border-radius: var(--radius-m); padding: 16px; background: var(--surface); }
  h2 { font-size: var(--fs-l); margin: 0; } p { color: var(--text-dim); line-height: 1.5; }
  [role='alert'] { color: var(--danger); }
</style>
