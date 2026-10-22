<script lang="ts">
  // http_request and api_run: method, URL and an optional JSON body.
  import type { WorkflowNode } from '../../../lib/api/types';
  import JsonLabel from './JsonLabel.svelte';
  import { jsonValue, paramJson, paramStr, type SetParam } from './util';

  let { node, set, onzoom }: { node: WorkflowNode; set: SetParam; onzoom: (field: string, label: string) => void } = $props();
</script>

{#if node.kind === 'api_run'}
  <label for="np-method">Method</label>
  <select
    id="np-method"
    value={paramStr(node, 'method') || 'GET'}
    onchange={(e) => set('method', e.currentTarget.value)}
  >
    {#each ['GET','POST','PUT','PATCH','DELETE'] as m (m)}
      <option value={m}>{m}</option>
    {/each}
  </select>
  <label for="np-url">URL</label>
  <input dir="ltr"
    id="np-url"
    type="url"
    placeholder="https://api.example.com/endpoint"
    value={paramStr(node, 'url')}
    oninput={(e) => set('url', e.currentTarget.value)}
  />
  <JsonLabel forId="np-body" label="Body (JSON, optional)" onzoom={() => onzoom('body', 'Body (JSON, optional)')} />
  <textarea dir="ltr"
    id="np-body"
    rows="3"
    placeholder="&#123;&#125;"
    value={paramJson(node, 'body')}
    oninput={(e) => set('body', jsonValue(e.currentTarget.value))}
  ></textarea>
{:else}
  <label for="np-method">Method</label>
  <select
    id="np-method"
    value={paramStr(node, 'method') || 'GET'}
    onchange={(e) => set('method', e.currentTarget.value)}
  >
    {#each ['GET','POST','PUT','PATCH','DELETE','HEAD'] as m (m)}
      <option value={m}>{m}</option>
    {/each}
  </select>
  <label for="np-url">URL</label>
  <input dir="ltr"
    id="np-url"
    type="url"
    placeholder="https://example.com/api"
    value={paramStr(node, 'url')}
    oninput={(e) => set('url', e.currentTarget.value)}
  />
  <JsonLabel forId="np-body" label="Body (JSON, optional)" onzoom={() => onzoom('body', 'Body (JSON, optional)')} />
  <textarea dir="ltr"
    id="np-body"
    rows="3"
    placeholder="&#123;&#125;"
    value={paramJson(node, 'body')}
    oninput={(e) => set('body', jsonValue(e.currentTarget.value))}
  ></textarea>
{/if}
