<script lang="ts">
  // The side pane of the side-by-side split: a same-origin iframe of the app
  // at `?embed=1#/<route>` — its own router and stores (lib/sidePane.ts). It
  // has no bar of its own: its Swap / Open in main pane / Close controls sit
  // at the end of the page's own top row, inside the frame (PaneControls).
  // The frame is built once per pane and navigated in place afterwards, so
  // changing the pane's module never reloads it. Until it reports ready, a
  // cover shaped like the page (a header row + skeleton) stands in, with a
  // Close button; a pane that never boots says so, with Retry.
  import Icon from '../lib/components/Icon.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';
  import { sidePane } from '../lib/stores/sidePane.svelte';
  import { ui, isTauri } from '../lib/stores/ui.svelte';

  interface Props {
    label: string;
  }
  let { label }: Props = $props();

  let frame: HTMLIFrameElement | undefined = $state();
  $effect(() => {
    const f = frame;
    if (!f) return;
    sidePane.attach(f);
    return () => sidePane.detach(f);
  });

  // The cover's header row sits where the page's will: under the traffic
  // lights when this pane leads and the sidebar is the narrow Rail.
  const padTraffic = $derived(isTauri && sidePane.placement === 'leading' && !ui.railExpanded);
</script>

<section class="side-pane" aria-label={`${label} (side pane)`} data-testid="side-pane" data-module={sidePane.key}>
  {#key sidePane.generation}
    <iframe
      bind:this={frame}
      src={sidePane.frameSrc()}
      title={`${label} (side pane)`}
      class:ready={sidePane.status === 'ready'}
      class:dragging={sidePane.dragging}
      data-testid="side-pane-frame"
    ></iframe>
  {/key}
  {#if sidePane.status !== 'ready'}
    <div class="sp-cover" data-testid="side-pane-cover">
      <div class="sp-head chrome-material" class:tauri-pad={padTraffic}>
        <span class="sp-title">{label}</span>
        <span class="sp-grow"></span>
        <button
          class="icon-btn"
          onclick={() => sidePane.close()}
          aria-label="Close side pane"
          title="Close side pane (⌘\)"
        >
          <Icon name="x" size={14} />
        </button>
      </div>
      {#if sidePane.status === 'loading'}
        <div class="sp-skeleton" aria-busy="true" aria-label={`Loading ${label}`}>
          <Skeleton rows={6} height={28} />
        </div>
      {:else}
        <div class="sp-error" role="alert">
          <span class="sp-error-icon"><Icon name="warning" size={22} /></span>
          <h3>Couldn't load {label}</h3>
          <p>The side pane didn't start. Everything in the main pane is unaffected.</p>
          <div class="sp-error-actions">
            <button class="btn" onclick={() => sidePane.retry()}>
              <Icon name="refresh" size={13} /> Retry
            </button>
            <button class="btn" onclick={() => sidePane.close()}>Close pane</button>
          </div>
        </div>
      {/if}
    </div>
  {/if}
</section>

<style>
  .side-pane {
    position: relative;
    display: flex;
    min-width: 0;
    min-height: 0;
    background: var(--bg);
    /* The split container (App) sets --side-share, already clamped so
       neither pane drops under PANE_MIN_PX; the divider is the 1px between. */
    flex: 0 0 auto;
    width: calc(var(--side-share, 0.5) * 100%);
  }
  iframe {
    flex: 1;
    width: 100%;
    height: 100%;
    border: 0;
    display: block;
    background: var(--bg);
    opacity: 0;
    transition: opacity 160ms ease-out;
  }
  iframe.ready {
    opacity: 1;
  }
  /* A divider drag must not lose its pointer to the other document. */
  iframe.dragging {
    pointer-events: none;
  }
  .sp-cover {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg);
  }
  /* Shaped like the PageHeader row the page will draw (46px, chrome glass,
     hairline), so the pane doesn't jump when it's ready. */
  .sp-head {
    flex-shrink: 0;
    height: 46px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding-inline: 20px 16px;
    border-bottom: 1px solid var(--separator);
  }
  .sp-head.tauri-pad {
    padding-inline-start: 84px;
  }
  .sp-title {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-l);
    font-weight: 600;
    color: var(--text);
  }
  .sp-grow {
    flex: 1;
  }
  .sp-skeleton {
    padding: 18px 20px;
    max-width: 560px;
  }
  .sp-error {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 24px;
    text-align: center;
  }
  .sp-error-icon {
    display: grid;
    place-items: center;
    width: 48px;
    height: 48px;
    border-radius: var(--radius-l);
    background: var(--surface-2);
    color: var(--warning);
  }
  .sp-error h3 {
    margin: 4px 0 0;
    font-size: var(--fs-l);
    font-weight: 600;
    color: var(--text);
  }
  .sp-error p {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
    max-width: 320px;
  }
  .sp-error-actions {
    display: flex;
    gap: 8px;
    margin-top: 6px;
  }
  @media (prefers-reduced-motion: reduce) {
    iframe {
      transition: none;
    }
  }
</style>
