<script lang="ts">
  // What a route that can't render shows (shell/pages.svelte.ts pageKeyOf):
  // an unknown module ("Page not found"), one the person's role or a
  // disabled feature locks — instead of silently showing Agents, or a page
  // that fails with 403s — or, in the phone's light version, a module it
  // leaves out (lib/lightMode.svelte.ts), with the way back to the full app.
  // Mirrors Settings' own unknown / locked sections.
  import PageHeader from '../lib/components/PageHeader.svelte';
  import PageBody from '../lib/components/PageBody.svelte';
  import EmptyState from '../lib/components/EmptyState.svelte';
  import { router } from '../lib/router.svelte';
  import { knownRoute } from '../lib/routeKeys';
  import { moduleLabel } from '../lib/sidebar';
  import { LIGHT_HOME, lightMode, lightRouteAllowed } from '../lib/lightMode.svelte';

  const mod = $derived(router.parts[0] ?? '');
  const locked = $derived(knownRoute(mod));
  const label = $derived(moduleLabel(mod));
  // Light hides it (whatever the role says: the full app decides that).
  const lightHidden = $derived(lightMode.active && locked && !lightRouteAllowed(mod));
  // Where the gate's way out goes: the light version's home, else Agents.
  const home = $derived(lightMode.active ? LIGHT_HOME : 'agents');
  const homeLabel = $derived(`Go to ${moduleLabel(home)}`);
  // A link to one terminal session (e.g. "Continue on phone…" from Agents):
  // open it in the full app for this visit rather than dead-end on the gate.
  const sessionLink = $derived(mod === 'agents' && !!router.parts[1]);
</script>

<PageHeader title={locked ? label : 'Page not found'} />
<PageBody>
  {#if lightHidden}
    <!-- Switching light on from a hidden page navigates to Threads: while
         that page loads, stay blank rather than flash this notice. -->
    {#if router.pendingTarget}
      <!-- blank while the switch navigates -->
    {:else if sessionLink}
      <EmptyState
        variant="page"
        icon="terminal"
        title="This session runs in a terminal"
        body="The light version leaves out Agents, where terminal sessions live. Open this one in the full app just for now; the light version comes back when you move on."
        actionLabel="Open this session"
        onaction={() => lightMode.openFullOnce(router.parts.join('/'))}
      >
        <button class="btn ghost" onclick={() => router.go(LIGHT_HOME)}>Go to Threads</button>
      </EmptyState>
    {:else}
      <EmptyState
        variant="page"
        icon="phone"
        title={`${label} isn’t in the light version`}
        body="The light version shows Threads, Connections, Git, API and Usage. The full app has everything."
        actionLabel="Open the full app"
        onaction={() => lightMode.set(false)}
      >
        <button class="btn ghost" onclick={() => router.go(LIGHT_HOME)}>Go to Threads</button>
      </EmptyState>
    {/if}
  {:else if locked}
    <EmptyState
      variant="page"
      icon="lock"
      title={`You don’t have access to ${label}`}
      body="Your role, or a feature switched off for this Otto, hides it. Ask an admin for access."
      actionLabel={homeLabel}
      onaction={() => router.go(home)}
    >
      {#if router.canBack}<button class="btn ghost" onclick={() => router.back()}>Go back</button>{/if}
    </EmptyState>
  {:else}
    <EmptyState
      variant="page"
      icon="compass"
      title="Page not found"
      body={`Otto has no page at “#/${router.parts.join('/')}”. The link may be out of date.`}
      actionLabel={homeLabel}
      onaction={() => router.go(home)}
    >
      {#if router.canBack}<button class="btn ghost" onclick={() => router.back()}>Go back</button>{/if}
    </EmptyState>
  {/if}
</PageBody>
