<script module lang="ts">
  import { lazyComponent } from '../lazy-component.svelte';

  // ONE shared loader for every LazyTerminal: the xterm + WebGL chunk (~0.5 MB)
  // is fetched the first time any embed renders, then every later mount draws
  // in the same frame (lazy-component.svelte.ts).
  const TerminalLazy = lazyComponent(() => import('./Terminal.svelte'));
</script>

<script lang="ts">
  // Terminal.svelte, code-split. Pages that only show a terminal INSIDE a
  // running-agent section (Docs agents, Refine drawer, Product analysis, mockup
  // assist…) use this so xterm stays out of the page's static import set — a
  // first visit pays for it only when an agent pane actually opens. Terminal-
  // first hosts (SessionView, the share page, k8s exec) keep the static import.
  //
  // Same props as Terminal (forwarded verbatim) and the same exported methods
  // (no-ops until the chunk has loaded). Every state is designed: a compact
  // skeleton while loading, an inline error + Retry when the chunk fails.
  import type { ComponentProps } from 'svelte';
  import type Terminal from './Terminal.svelte';
  import LoadState from './LoadState.svelte';

  type Props = ComponentProps<typeof Terminal>;
  let props: Props = $props();

  let inner = $state<ReturnType<typeof Terminal> | undefined>();

  export function focus(): void {
    inner?.focus();
  }
  export function redraw(): void {
    inner?.redraw();
  }
  export function openFind(): void {
    inner?.openFind();
  }
</script>

{#if TerminalLazy.component}
  {@const T = TerminalLazy.component}
  <T bind:this={inner} {...props} />
{:else}
  <LoadState
    what="the terminal"
    variant="panel"
    empty
    loading={!TerminalLazy.error}
    error={TerminalLazy.error}
    onretry={() => TerminalLazy.retry()}
  />
{/if}
