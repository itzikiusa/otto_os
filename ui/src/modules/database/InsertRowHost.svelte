<script lang="ts">
  // Mounts the "Add row" form (lazily) for one ResultsGrid and serves the
  // schema tree's "Insert row…" request: the store opened the table in a new
  // tab and asked THAT tab's grid to open its insert UI once the result is in
  // (`database.objects.pendingInsert` = the tab uid). The request is one-shot —
  // it opens the form, or is dropped with the reason when the result can't take
  // an insert — so it never fires on a later run.
  import { untrack } from 'svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import { database } from '../../lib/stores/database.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import type { EditFlow } from './EditFlow.svelte';
  import type { QueryResult } from '../../lib/api/types';

  const InsertRowFormLazy = lazyComponent(() => import('./InsertRowForm.svelte'));

  interface Props {
    flow: EditFlow;
    /** The main (connection-backed, full-size) grid — the only one that serves requests. */
    active: boolean;
    running: boolean;
    /** The grid's shown result (null until the tab's run lands). */
    result: QueryResult | null;
    failed: boolean;
    canModify: boolean;
  }
  let { flow, active, running, result, failed, canModify }: Props = $props();

  $effect(() => {
    const req = database.objects.pendingInsert;
    if (!req || !active || running || database.tab?.uid !== req) return;
    // Settle only on THIS run's outcome, once the flow has been fed it (never
    // on the previous tab's result still in the flow).
    if (!failed && (!result || flow.result !== result)) return;
    const target = flow.insert.target;
    untrack(() => {
      database.objects.pendingInsert = null;
      if (target) void flow.insert.open();
      else if (!failed)
        toasts.info('Can’t add a row here', canModify ? 'This result can’t be attributed to one table.' : 'You don’t have edit access to this connection.');
    });
  });
</script>

{#if flow.insert.form}
  <LazyMount lazy={InsertRowFormLazy} what="the new-row form" quiet props={{ flow }} onerror={() => flow.insert.close()} />
{/if}
