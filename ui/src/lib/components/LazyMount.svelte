<script lang="ts">
  // Renders a `lazyComponent` (lib/lazy-component.svelte.ts) with every state
  // designed: a skeleton while its chunk loads, an inline error + Retry when the
  // load fails, the component once loaded (synchronously on every later mount).
  //
  //   <LazyMount lazy={Structure} what="the structure view" />
  //   <LazyMount lazy={Sftp} what="the SFTP browser" quiet props={{ conn, onclose }} />
  //
  // `quiet` is for modals/overlays: nothing renders while loading (the chunk is
  // a few ms away and a skeleton at the end of the page would flash), and a
  // failed load reports through a toast instead of an inline block.
  import type { Component } from 'svelte';
  import LoadState from './LoadState.svelte';
  import type { LazyComponent } from '../lazy-component.svelte';
  import { toasts } from '../toast.svelte';

  interface Props {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    lazy: LazyComponent<Component<any>>;
    /** Noun for the error headline: "Couldn't load {what}". */
    what: string;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    props?: Record<string, any>;
    quiet?: boolean;
    variant?: 'page' | 'panel' | 'compact';
  }
  let { lazy, what, props = {}, quiet = false, variant = 'page' }: Props = $props();

  let toasted: string | null = null;
  $effect(() => {
    const e = lazy.error;
    if (quiet && e && e !== toasted) {
      toasted = e;
      toasts.error(`Couldn’t load ${what}`, e);
    }
  });
</script>

{#if lazy.component}
  {@const C = lazy.component}
  <C {...props} />
{:else if !quiet}
  <LoadState {what} {variant} empty loading={!lazy.error} error={lazy.error} onretry={() => lazy.retry()} />
{/if}
