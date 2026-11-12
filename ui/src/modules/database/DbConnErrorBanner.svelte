<script lang="ts">
  // Inline "couldn't connect" banner over the DB workbench. The failure used
  // to live only in tooltips (the tab's red dot, the header "Disconnected"
  // chip) and in the schema tree, which is collapsed on phones — so the main
  // area just showed an editor that could not run anything. A failed load
  // shows inline with Retry (design guidelines), plus the way to fix it.
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { database } from '../../lib/stores/database.svelte';
  import type { Connection } from '../../lib/api/types';

  interface Props {
    /** Absent when the viewer may not edit the selected profile. */
    onedit?: (c: Connection) => void;
  }
  let { onedit }: Props = $props();

  const status = $derived(database.activeConnStatus);
  // `hint` is optional and newer than ConnStatus: read it defensively.
  const hint = $derived(status && 'hint' in status && typeof status.hint === 'string' ? status.hint : null);
  const detail = $derived(
    status?.phase === 'error' ? `${status.error || 'The server gave no reason.'}${hint ? ` ${hint}` : ''}` : null,
  );
  let retrying = $state(false);

  async function retry(): Promise<void> {
    retrying = true;
    try {
      await database.retryConnection();
    } finally {
      retrying = false;
    }
  }
</script>

{#if detail}
  <div class="conn-error" data-testid="db-conn-error-banner">
    <LoadState
      what="the connection"
      errorTitle={database.selectedConn ? `Couldn’t connect to ${database.selectedConn.name}` : 'Couldn’t connect'}
      error={detail}
      loading={retrying}
      empty
      onretry={() => void retry()}
      variant="compact"
    >
      {#snippet errorActions()}
        {#if onedit && database.selectedConn}
          {@const c = database.selectedConn}
          <button class="btn small" onclick={() => onedit(c)}>
            <Icon name="edit" size={12} />Edit connection
          </button>
        {/if}
      {/snippet}
    </LoadState>
  </div>
{/if}

<style>
  .conn-error {
    flex: none;
    margin-block-end: 10px;
    border: 1px solid var(--danger);
    border-radius: var(--radius-m);
    background: var(--danger-soft);
    overflow-wrap: anywhere;
  }
</style>
