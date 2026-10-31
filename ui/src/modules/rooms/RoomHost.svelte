<script lang="ts">
  import { onMount } from 'svelte';
  import type { RoomCredential } from '../../lib/api/room-types';
  import { isTauri } from '../../lib/desktop';
  import { rememberRoom, forgetRoom } from './room-access';
  import RoomPage from './RoomPage.svelte';
  import { requestRoomContext } from './room-window';
  import LoadState from '../../lib/components/LoadState.svelte';
  import ConfirmDialog from '../../lib/components/ConfirmDialog.svelte';
  import ContextMenu from '../../lib/components/ContextMenu.svelte';
  import { loadErrorText } from '../../lib/loadError';

  let { roomId }: { roomId: string } = $props();
  let ready = $state(false), error = $state(''), loading = $state(true);
  let disposed = false;
  async function load() {
    loading = true; error = '';
    try {
      const context = isTauri
        ? await (await import('@tauri-apps/api/core')).invoke<{ origin: string; credential: RoomCredential }>('get_host_room_context')
        : await requestRoomContext(roomId);
      if (disposed) return;
      if (context.credential.room_id !== roomId) throw new Error('This window belongs to another room. Reopen it from Rooms.');
      rememberRoom(context.origin, context.credential);
      ready = true;
    } catch (e) { if (!disposed) error = loadErrorText(e); }
    finally { if (!disposed) loading = false; }
  }
  onMount(() => { void load(); return () => { disposed = true; forgetRoom(roomId); }; });
</script>

{#if ready}
  <RoomPage {roomId} />
{:else}
  <div class="room-loading">
    <LoadState what="this room" variant="page" {loading} {error} empty={true} onretry={load} />
  </div>
{/if}
<ConfirmDialog />
<ContextMenu />

<style>
  .room-loading { display: grid; place-content: center; justify-items: center; gap: 12px; height: 100%; padding: 24px; }
</style>
