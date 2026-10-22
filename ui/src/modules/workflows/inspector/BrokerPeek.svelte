<script lang="ts">
  // broker_peek: pick a cluster, then one of its topics (listed once the
  // cluster is a known one — never per keystroke of a typed id).
  import type { WorkflowNode } from '../../../lib/api/types';
  import IdPicker from './IdPicker.svelte';
  import { isUlid, type InspectorOptions } from './options.svelte';
  import { clusterOptions, paramNum, paramStr, topicOptions, type SetParam } from './util';

  let { node, set, wsId, pickers }: { node: WorkflowNode; set: SetParam; wsId: string; pickers: InspectorOptions } = $props();
  const cid = $derived(paramStr(node, 'cluster_id'));
  // Only a COMPLETE id: one the list knows, or (free text) a full ULID.
  const clusterKnown = $derived.by(() => {
    const rows = pickers.brokerClusters.rows(wsId);
    return rows ? rows.some((c) => c.id === cid) : isUlid(cid);
  });
</script>

<label for="np-cid">Cluster</label>
<IdPicker id="np-cid" dir="ltr" value={cid} cache={pickers.brokerClusters} key={wsId} toOptions={clusterOptions}
  noun="cluster" placeholder="Broker cluster id" what="broker clusters" onchange={(v) => set('cluster_id', v)} />
<label for="np-topic">Topic</label>
{#if cid && clusterKnown}
  <IdPicker id="np-topic" dir="ltr" value={paramStr(node, 'topic')} cache={pickers.brokerTopics} key={cid} toOptions={topicOptions}
    noun="topic" placeholder="my-topic" what="topics" onchange={(v) => set('topic', v)} />
{:else}
  <input dir="ltr" id="np-topic" type="text" placeholder={cid ? 'my-topic' : 'Choose a cluster first'}
    value={paramStr(node, 'topic')} oninput={(e) => set('topic', e.currentTarget.value)} />
{/if}
<label for="np-limit">Limit (max 50)</label>
<input
  id="np-limit"
  type="number"
  min="1"
  max="50"
  value={paramNum(node, 'limit', 20)}
  oninput={(e) => set('limit', Number(e.currentTarget.value))}
/>
