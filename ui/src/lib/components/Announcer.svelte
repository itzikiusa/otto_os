<script lang="ts">
  import { announcer } from '../announce.svelte';
</script>

<!-- The app's two screen-reader live regions, mounted ONCE (App.svelte, beside
     Toasts) and always present, so a text change is what VoiceOver speaks.
     Fed only through `announce()` — see lib/announce.svelte.ts. Neither carries
     role="status", so there's no extra "status" landmark. -->
<div class="sr-only" aria-live="polite" aria-atomic="true" data-testid="announcer-polite">{announcer.polite}</div>
<div class="sr-only" aria-live="assertive" aria-atomic="true" data-testid="announcer-assertive">{announcer.assertive}</div>
