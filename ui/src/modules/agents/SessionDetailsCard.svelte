<script lang="ts">
  // The Agents page's Side panel tab "Session" (A5; registered by AgentsPage):
  // Threads' ThreadDetailsCard for the focused session. Two-step lazy on
  // purpose — the panel loads this tiny chunk only when the tab is shown, and
  // the card (with its own preload list) loads from here, so neither weighs on
  // the page. A chat pane (`cwd` null) takes its folder from its thread row.
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import { runtimeThreads } from '../threads/runtime/runtimeThreads.svelte'; // ui-guards: allow — the chat pane's own row (one store with Threads)

  let { cwd, threadId }: { cwd: string | null; threadId: string } = $props();
  const Card = lazyComponent(() => import('../threads/ThreadDetailsCard.svelte')); // ui-guards: allow — one details card for both pages
  const folder = $derived(cwd ?? runtimeThreads.get(threadId)?.cwd ?? null);
</script>

{#if folder}
  <div class="sdc" data-testid="agents-details-card">
    <LazyMount lazy={Card} what="the session details" variant="compact" props={{ cwd: folder, threadId, agents: true }} />
  </div>
{/if}

<style>
  .sdc {
    padding: var(--sp-3) var(--sp-4);
  }
</style>
