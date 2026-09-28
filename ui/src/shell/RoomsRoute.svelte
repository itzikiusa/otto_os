<script lang="ts">
  // `#/rooms…` as one lazily loaded page (shell/pages.svelte.ts): the lobby, the
  // recaps list, or one room. Keyed by room id — a room's call/socket state
  // belongs to that room, so switching rooms must remount it.
  import RoomPage from '../modules/rooms/RoomPage.svelte';
  import RoomRecapsPage from '../modules/rooms/RoomRecapsPage.svelte';
  import RoomLobby from '../modules/rooms/RoomLobby.svelte';
  import { router } from '../lib/router.svelte';
</script>

{#if router.parts[1] === 'recaps'}<RoomRecapsPage />{:else if router.parts[1]}{#key router.parts[1]}<RoomPage roomId={router.parts[1]} />{/key}{:else}<RoomLobby />{/if}
