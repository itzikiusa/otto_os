<script lang="ts">
  // Renders a `lazyComponent` (lib/lazy-component.svelte.ts) with every state
  // designed: a skeleton while its chunk loads, an inline error + Retry when the
  // load fails, the component once loaded (synchronously on every later mount).
  //
  //   <LazyMount lazy={Structure} what="the structure view" />
  //   <LazyMount lazy={Sftp} what="the SFTP browser" quiet props={{ conn, onclose }} />
  //   <LazyMount lazy={History} what="the story history" variant="panel" />
  //
  // `quiet` is for modals/overlays: nothing renders while loading (the chunk is
  // a few ms away and a skeleton at the end of the page would flash), and a
  // failed load reports through a toast instead of an inline block. Pass
  // `onerror` to reset the overlay's open flag on a failure, so the next open
  // (a click, ⌘N) mounts again and retries; without it the toast offers
  // "Try again". `variant` sizes the inline states: 'page' fills a route,
  // 'panel' a tab or pane body, 'compact' a card.
  import { untrack, type Component } from 'svelte';
  import LoadState from './LoadState.svelte';
  import type { LazyComponent } from '../lazy-component.svelte';
  import { toasts } from '../toast.svelte';

  interface Props {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    lazy: LazyComponent<Component<any>>;
    /** Noun for the error headline: "Couldn’t load {what}". */
    what: string;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    props?: Record<string, any>;
    quiet?: boolean;
    variant?: 'page' | 'panel' | 'compact';
    /** `quiet` only: called once when the chunk fails (reset the open flag). */
    onerror?: () => void;
  }
  let { lazy, what, props = {}, quiet = false, variant = 'page', onerror }: Props = $props();

  // A fresh mount after an earlier failure is a new attempt: clear the stale
  // error and load again (declared before the toast effect, so that effect
  // sees the cleared error rather than toasting the old one a second time).
  $effect(() => {
    untrack(() => {
      if (lazy.error) lazy.retry();
    });
  });

  let toasted: string | null = null;
  $effect(() => {
    const e = lazy.error;
    if (!e) toasted = null; // a retry cleared it: the next failure toasts again
    if (!quiet || !e || e === toasted) return;
    toasted = e;
    untrack(() => {
      if (onerror) {
        toasts.error(`Couldn’t load ${what}`, e);
        onerror();
      } else {
        toasts.push('error', `Couldn’t load ${what}`, e, 12000, { action: { label: 'Try again', run: () => lazy.retry() } });
      }
    });
  });
</script>

{#if lazy.component}
  {@const C = lazy.component}
  <C {...props} />
{:else if !quiet}
  <LoadState {what} {variant} empty loading={!lazy.error} error={lazy.error} onretry={() => lazy.retry()} />
{/if}
