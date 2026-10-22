<script lang="ts">
  // "Couldn’t load your permissions" — an IN-FLOW strip at the foot of the
  // shell, above the status bar (ux-design-2-07). It used to float over the
  // unified toolbar at the toast layer with no close, covering the back button
  // and title for as long as the daemon kept failing. Dismiss holds until the
  // next failed read (`auth.capabilitiesNoticeDismissed`).
  import Icon from '../lib/components/Icon.svelte';
  import LoadState from '../lib/components/LoadState.svelte';
  import { auth } from '../lib/stores/auth.svelte';

  let retrying = $state(false);
  async function retry(): Promise<void> {
    if (retrying) return;
    retrying = true;
    try {
      await auth.retryCapabilities();
    } finally {
      retrying = false;
    }
  }
</script>

<!-- Root never depends on the map; anyone else would otherwise see a silently
     all-deny shell. -->
{#if auth.capabilitiesError && !auth.isRoot && !auth.capabilitiesNoticeDismissed}
  <div class="caps-banner" data-testid="caps-banner">
    <LoadState what="your permissions" variant="compact" empty error="Some pages may be hidden." loading={retrying} onretry={retry}>
      {#snippet errorActions()}
        <button
          class="icon-btn"
          onclick={() => auth.dismissCapabilitiesNotice()}
          title="Dismiss until the next failed retry"
          aria-label="Dismiss"
        >
          <Icon name="x" size={12} />
        </button>
      {/snippet}
    </LoadState>
  </div>
{/if}

<style>
  .caps-banner {
    flex-shrink: 0;
    padding: 6px 12px;
    border-block-start: 1px solid var(--warning);
    background: var(--warning-soft);
    color: var(--text);
    font-size: var(--fs-s);
  }
  .caps-banner > :global(*) {
    min-width: 0;
    overflow-wrap: anywhere;
  }
</style>
