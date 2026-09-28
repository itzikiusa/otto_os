<script lang="ts">
  // Guest-only chrome stays in the room chunk: the desktop's hidden bar and
  // tray windows must not load room media, xterm, or the owner shell at boot.
  import RoomPage from './RoomPage.svelte';
  import ConfirmDialog from '../../lib/components/ConfirmDialog.svelte';
  import ContextMenu from '../../lib/components/ContextMenu.svelte';
  let { roomId }: { roomId: string } = $props();
</script>

<RoomPage {roomId} guest />
<ConfirmDialog />
<ContextMenu />
