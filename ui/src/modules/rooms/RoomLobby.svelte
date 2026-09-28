<script lang="ts">
  import RoomSettingsModal from './RoomSettingsModal.svelte';
  import { onMount } from 'svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { api } from '../../lib/api/client';
  import { confirmer } from '../../lib/confirm.svelte';
  import { router } from '../../lib/router.svelte';
  import { parseRoomInvite } from './room-state';
  import { recallRoom } from './room-access';
  import { openHostedRoom } from './room-window';
  import type { RoomSnapshot } from '../../lib/api/room-types';
  let settingsOpen = $state(false);
  let rooms = $state<RoomSnapshot[]>([]);
  let loading = $state(true), error = $state(''), joinOpen = $state(false), link = $state(''), joinError = $state('');
  let opening = $state(''), openError = $state('');
  async function openRoom(id: string) {
    opening = id; openError = '';
    try { await openHostedRoom(id); }
    catch (e) { openError = e instanceof Error ? e.message : String(e); }
    finally { opening = ''; }
  }
  onMount(() => { void load(); });
  async function load() {
    loading = true; error = '';
    try { rooms = await api.get<RoomSnapshot[]>('/rooms'); }
    catch (e) { error = e instanceof Error ? e.message : 'Could not load rooms.'; }
    finally { loading = false; }
  }
  async function join() {
    joinError = '';
    try {
      const invitation = parseRoomInvite(link);
      if (!await confirmer.ask(`Connect to ${invitation.origin}? Your display name and room activity will be shared with this host and admitted participants.`, {title: 'Join room', confirmLabel: 'Continue'})) return;
      if ('__TAURI_INTERNALS__' in window) {
        const { invoke } = await import('@tauri-apps/api/core');
        await invoke<string>('open_room_window', {url: invitation.url});
        joinOpen = false; link = '';
      } else { window.location.assign(invitation.url); }
    } catch (e) { joinError = e instanceof Error ? e.message : 'Could not open this room. Try again.'; }
  }
</script>
<div class="rooms-page">
  <PageHeader title="Rooms" icon="people" subtitle="Work together in a session">
    {#snippet actions()}<button class="btn" onclick={() => router.go('rooms/recaps')}>Recap archives</button>{#if auth.isRoot}<button class="btn" onclick={() => settingsOpen = true}>Connection settings…</button>{/if}<button class="btn primary" onclick={() => joinOpen = true}>Join room…</button>{/snippet}
  </PageHeader>
  <PageBody>
    {#if openError}<p role="alert">{openError} Use Open room to retry.</p>{/if}
    {#if loading}<p role="status">Loading rooms…</p>
    {:else if error}<p role="alert">{error}</p><button class="btn" onclick={load}>Retry</button>
    {:else if !rooms.length}<EmptyState variant="page" icon="people" title="Bring someone into your session" body="Open a running session’s More menu and choose Start room. You control admission and terminal access." />
    {:else}<div class="room-list">{#each rooms as room (room.room_id)}
      <article><h2>{room.session_title ?? 'Session room'}</h2><p>{room.members?.filter(m => m.admission === 'admitted').length ?? 1} of 4 people</p>
        {#if recallRoom(room.room_id)}<button class="btn" disabled={!!opening} onclick={() => openRoom(room.room_id)}>{opening === room.room_id ? 'Opening…' : 'Open room'}</button>
        {:else}<p>Open in the window where you started the room.</p>{/if}
      </article>
    {/each}</div>{/if}
  </PageBody>
</div>
{#if settingsOpen}<RoomSettingsModal onclose={() => settingsOpen = false} />{/if}
{#if joinOpen}<Modal title="Join a room" onclose={() => joinOpen = false}>
  <label>Invitation link <input type="url" bind:value={link} placeholder="https://host/#/room/…" autocomplete="off" spellcheck="false" /></label>
  <p>You will see the destination before connecting.</p>
  {#if joinError}<p role="alert">{joinError}</p>{/if}
  {#snippet footer()}<button class="btn" onclick={() => joinOpen = false}>Cancel</button><button class="btn primary" disabled={!link.trim()} onclick={join}>Continue</button>{/snippet}
</Modal>{/if}
<style>
  .rooms-page { display: flex; flex-direction: column; height: 100%; min-width: 0; }
  label { display: grid; gap: 8px; } input { width: 100%; }
  .room-list { display: grid; gap: 12px; grid-template-columns: repeat(auto-fit, minmax(min(280px, 100%), 1fr)); }
  article { border: 1px solid var(--border); border-radius: var(--radius-m); padding: 16px; background: var(--surface); }
  h2 { font-size: var(--fs-l); margin: 0; } p { color: var(--text-dim); line-height: 1.5; }
  [role='alert'] { color: var(--danger); }
</style>
