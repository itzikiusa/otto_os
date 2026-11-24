<script lang="ts">
  // The details of "Can't connect" (the workbench header's one line carries the
  // word, a short reason and Retry; its Details shows this): the server's full
  // reason with its hint, and the ways to fix it.
  import Icon from '../../lib/components/Icon.svelte';
  import { database } from '../../lib/stores/database.svelte';
  import type { Connection } from '../../lib/api/types';
  import DiagnoseButton from './DiagnoseButton.svelte';

  interface Props {
    /** Absent when the viewer may not edit the selected profile. */
    onedit?: (c: Connection) => void;
    /** A phone's header has no room for Retry: it sits here. */
    onretry?: () => void;
    retrying?: boolean;
  }
  let { onedit, onretry, retrying = false }: Props = $props();

  const status = $derived(database.activeConnStatus);
  // `hint` is optional and newer than ConnStatus: read it defensively.
  const hint = $derived(status && 'hint' in status && typeof status.hint === 'string' ? status.hint : null);
</script>

{#if status?.phase === 'error'}
  <div class="conn-error" data-testid="db-conn-error-banner">
    <p class="conn-error-text">{status.error || 'The server gave no reason.'}{hint ? ` ${hint}` : ''}</p>
    <div class="conn-error-actions">
      {#if onretry}<button class="btn small" disabled={retrying} onclick={onretry}>{retrying ? 'Retrying…' : 'Retry'}</button>{/if}<!-- ui-guards: allow — the phone's one Retry for "Can't connect" (the header has no room) -->
      {#if database.selectedConn}
        <DiagnoseButton conn={database.selectedConn} probe={{ message: status.error ?? '', hint }} />
      {/if}
      {#if onedit && database.selectedConn}
        {@const c = database.selectedConn}
        <button class="btn small" onclick={() => onedit(c)}>
          <Icon name="edit" size={12} />Edit connection
        </button>
      {/if}
    </div>
  </div>
{/if}

<style>
  .conn-error {
    flex: none;
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-block-end: 10px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    overflow-wrap: anywhere;
  }
  .conn-error-text {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
    user-select: text;
  }
  .conn-error-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
</style>
