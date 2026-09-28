<script lang="ts">
  import { onMount } from 'svelte';
  import type { RoomCredential } from '../../lib/api/room-types';
  import { isTauri } from '../../lib/desktop';
  import { rememberRoom, forgetRoom } from './room-access';
  import RoomPage from './RoomPage.svelte';
  import ConfirmDialog from '../../lib/components/ConfirmDialog.svelte';
  import ContextMenu from '../../lib/components/ContextMenu.svelte';

  let { roomId }: { roomId: string } = $props();
  let ready = $state(false), error = $state(''), loading = $state(true);
  let disposed = false;
  async function load() {
    loading = true; error = '';
    try {
      if (!isTauri) throw new Error('Open this room from the Otto desktop app.');
      const { invoke } = await import('@tauri-apps/api/core');
      const context = await invoke<{ origin: string; credential: RoomCredential }>('get_host_room_context');
      if (disposed) return;
      if (context.credential.room_id !== roomId) throw new Error('This window belongs to another room. Reopen it from Rooms.');
      rememberRoom(context.origin, context.credential);
      ready = true;
    } catch (e) { if (!disposed) error = e instanceof Error ? e.message : String(e); }
    finally { if (!disposed) loading = false; }
  }
  onMount(() => { void load(); return () => { disposed = true; forgetRoom(roomId); }; });
</script>

{#if ready}
  <RoomPage {roomId} />
{:else}
  <div class="room-loading">
    {#if error}<p role="alert">{error}</p><button class="btn" onclick={load}>Retry</button>
    {:else if loading}<p role="status">Opening room…</p>{/if}
  </div>
{/if}
<ConfirmDialog />
<ContextMenu />

<style>
  .room-loading { display: grid; place-content: center; justify-items: center; gap: 12px; height: 100%; padding: 24px; }
  [role='alert'] { color: var(--danger); }
</style>
