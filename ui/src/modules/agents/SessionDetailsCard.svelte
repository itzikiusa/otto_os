<script lang="ts">
  // The Agents right panel's details card (A5): Threads' ThreadDetailsCard for
  // the focused session. Two-step lazy on purpose — RightPanel loads this tiny
  // chunk only when the panel is open, and the card (with its own preload
  // list) loads from here, so neither weighs on the panel shell.
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';

  let { cwd, threadId }: { cwd: string; threadId: string } = $props();
  const Card = lazyComponent(() => import('../threads/ThreadDetailsCard.svelte')); // ui-guards: allow — one details card for both pages
</script>

<LazyMount lazy={Card} what="the session details" variant="compact" props={{ cwd, threadId }} />
