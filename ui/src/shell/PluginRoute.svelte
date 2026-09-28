<script lang="ts">
  // `#/plugin/<slug>` as a lazily loaded page (shell/pages.svelte.ts; a slug-less
  // route is Agents there). Keyed by slug: each plugin is its own iframe
  // origin + bridge, so switching plugins must remount the frame.
  import PluginFrame from '../modules/plugins/PluginFrame.svelte';
  import { router } from '../lib/router.svelte';
</script>

{#key router.parts[1]}
  <PluginFrame slug={router.parts[1] ?? ''} />
{/key}
