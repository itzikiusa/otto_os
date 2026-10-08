<script lang="ts">
  // Keep each destination independent: Games must not fetch the live terminal,
  // room call and recap dependency graphs before it can render its chooser.
  import LazyMount from '../lib/components/LazyMount.svelte';
  import { lazyComponent } from '../lib/lazy-component.svelte';
  import { router } from '../lib/router.svelte';
  const games = lazyComponent(() => import('../modules/rooms/games/GamesHub.svelte'));
  const room = lazyComponent(() => import('../modules/rooms/RoomPage.svelte'));
  const recaps = lazyComponent(() => import('../modules/rooms/RoomRecapsPage.svelte'));
  const lobby = lazyComponent(() => import('../modules/rooms/RoomLobby.svelte'));
</script>

{#if router.parts[1] === 'games'}
  <LazyMount lazy={games} what="games" />
{:else if router.parts[1] === 'recaps'}
  <LazyMount lazy={recaps} what="room recaps" />
{:else if router.parts[1]}
  {#key router.parts[1]}<LazyMount lazy={room} what="room" props={{roomId:router.parts[1]}} />{/key}
{:else}
  <LazyMount lazy={lobby} what="rooms" />
{/if}
