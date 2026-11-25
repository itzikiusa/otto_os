<script lang="ts">
  // Mounted once per app window (desktop only, lazy): follows the shell's
  // updater for the root account and shows the Update dialog when asked. Only
  // the main window announces how the last update went.
  import { auth } from '../lib/stores/auth.svelte';
  import { isPopout } from '../lib/desktop';
  import { updates } from '../lib/stores/updates.svelte';
  import UpdateDialog from './UpdateDialog.svelte';

  $effect(() => {
    if (auth.isRoot) updates.init(!isPopout);
  });
</script>

{#if updates.open && auth.isRoot}<UpdateDialog />{/if}
