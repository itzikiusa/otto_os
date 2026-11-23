<script lang="ts">
  // db_query: a statement against one of the workspace's Database Explorer
  // connections, picked by name · engine · environment.
  import type { WorkflowNode } from '../../../lib/api/types';
  import EnvBadge from '../../../lib/components/EnvBadge.svelte';
  import IdPicker from './IdPicker.svelte';
  import type { InspectorOptions } from './options.svelte';
  import { connectionOptions, paramNum, paramStr, type SetParam } from './util';

  let { node, set, wsId, pickers }: { node: WorkflowNode; set: SetParam; wsId: string; pickers: InspectorOptions } = $props();
  const selectedConn = $derived(
    (pickers.dbConnections.rows(wsId) ?? []).find((c) => c.id === paramStr(node, 'connection_id')) ?? null,
  );
</script>

<label for="np-conn">Connection</label>
<IdPicker id="np-conn" value={paramStr(node, 'connection_id')} cache={pickers.dbConnections} key={wsId} toOptions={connectionOptions}
  noun="connection" placeholder="DB-Explorer connection id" what="database connections"
  emptyHint="No database connections in this workspace yet — add one in Database Explorer." onchange={(v) => set('connection_id', v)} />
{#if selectedConn?.environment === 'prod'}
  <div class="prod-warn">
    <EnvBadge env={selectedConn.environment} />
    <span>This step runs against a production database. Keep statements read-only and bounded.</span>
  </div>
{/if}
<label for="np-stmt">SQL / query statement</label>
<textarea dir="ltr"
  id="np-stmt"
  rows="4"
  placeholder="SELECT * FROM users LIMIT 100"
  value={paramStr(node, 'statement')}
  oninput={(e) => set('statement', e.currentTarget.value)}
></textarea>
<label for="np-maxrows">Max rows (default 100)</label>
<input
  id="np-maxrows"
  type="number"
  min="1"
  max="1000"
  value={paramNum(node, 'max_rows', 100)}
  oninput={(e) => set('max_rows', Number(e.currentTarget.value))}
/>

<style>
  .prod-warn {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-block-start: 6px;
    padding: 6px 8px;
    border: 1px solid color-mix(in srgb, var(--danger) 35%, transparent);
    border-radius: var(--radius-s);
    background: var(--danger-soft);
    color: var(--danger);
    font-size: var(--fs-xs);
  }
</style>
