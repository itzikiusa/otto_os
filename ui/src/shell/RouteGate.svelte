<script lang="ts">
  // What a route that can't render shows (shell/pages.svelte.ts pageKeyOf):
  // an unknown module ("Page not found") or one the person's role or a
  // disabled feature locks — instead of silently showing Agents, or a page
  // that fails with 403s. Mirrors Settings' own unknown / locked sections.
  import PageHeader from '../lib/components/PageHeader.svelte';
  import PageBody from '../lib/components/PageBody.svelte';
  import EmptyState from '../lib/components/EmptyState.svelte';
  import { router } from '../lib/router.svelte';
  import { knownRoute } from '../lib/routeKeys';
  import { moduleLabel } from '../lib/sidebar';

  const mod = $derived(router.parts[0] ?? '');
  const locked = $derived(knownRoute(mod));
  const label = $derived(moduleLabel(mod));
</script>

<PageHeader title={locked ? label : 'Page not found'} />
<PageBody>
  {#if locked}
    <EmptyState
      variant="page"
      icon="lock"
      title={`You don’t have access to ${label}`}
      body="Your role, or a feature switched off for this Otto, hides it. Ask an admin for access."
      actionLabel="Go to Agents"
      onaction={() => router.go('agents')}
    >
      {#if router.canBack}<button class="btn ghost" onclick={() => router.back()}>Go back</button>{/if}
    </EmptyState>
  {:else}
    <EmptyState
      variant="page"
      icon="compass"
      title="Page not found"
      body={`Otto has no page at “#/${router.parts.join('/')}”. The link may be out of date.`}
      actionLabel="Go to Agents"
      onaction={() => router.go('agents')}
    >
      {#if router.canBack}<button class="btn ghost" onclick={() => router.back()}>Go back</button>{/if}
    </EmptyState>
  {/if}
</PageBody>
