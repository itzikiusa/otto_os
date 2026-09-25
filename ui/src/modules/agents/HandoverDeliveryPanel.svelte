<script lang="ts">
  import { api } from '../../lib/api/client';
  import type { HandoverDelivery, Session } from '../../lib/api/types';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';

  let { session, readonly = false }: { session: Session; readonly?: boolean } = $props();
  const delivery = $derived(session.meta?.handover as HandoverDelivery | undefined);
  let busy = $state(false);
  async function act(action: 'retry' | 'acknowledge') {
    if (!delivery || busy || readonly) return;
    const targetId = session.id;
    const deliveryId = delivery.id;
    busy = true;
    try {
      const updated = await api.post<Session>(`/sessions/${targetId}/handover/${action}`, { delivery_id: deliveryId });
      ws.sessions = ws.sessions.map((s) => s.id === targetId ? updated : s);
      toasts.success(action === 'retry' ? 'Handover retry started' : 'Receipt confirmed');
    } catch (e) {
      toasts.error('Handover action failed', e instanceof Error ? e.message : String(e));
    } finally { busy = false; }
  }
</script>

{#if delivery}
  <details class="handover-delivery">
    <summary>Handover · {delivery.state.replaceAll('_', ' ')}</summary>
    {#if delivery.error}<p role="alert">{delivery.error}</p>{/if}
    {#if delivery.focus}<p><strong>Focus:</strong> {delivery.focus}</p>{/if}
    {#if delivery.brief}<pre>{delivery.brief}</pre>{:else}<p>Preparing the saved brief…</p>{/if}
    {#if delivery.state === 'sent'}
      <p>Inspect the target conversation, then confirm that it received the brief.
        {delivery.archive_source ? 'The source will be archived only after confirmation.' : ''}</p>
      <button class="btn small" disabled={readonly || busy} onclick={() => act('acknowledge')}>Confirm received</button>
    {:else if delivery.state !== 'acknowledged'}
      <p>If delivery was interrupted, inspect the target before retrying to avoid sending the context twice.</p>
      <button class="btn small" disabled={readonly || busy} onclick={() => act('retry')}>Retry delivery</button>
    {/if}
  </details>
{/if}

<style>
  .handover-delivery { padding: 6px 12px; border-bottom: 1px solid var(--border); font-size: var(--fs-s); }
  summary { cursor: pointer; }
  pre { white-space: pre-wrap; overflow-wrap: anywhere; max-height: 240px; overflow: auto; }
  p { margin: 8px 0; }
</style>
