<script lang="ts">
  // swarm_task: pick the swarm / project instead of pasting raw ids. Loaded on
  // demand (null = not loaded / failed — the inputs fall back to free text so a
  // saved id is still editable); each lookup is tried ONCE per workspace / swarm.
  import type { WorkflowNode } from '../../../lib/api/types';
  import { untrack } from 'svelte';
  import { isUlid, type InspectorOptions } from './options.svelte';
  import { paramStr, type SetParam } from './util';

  let { node, set, wsId, pickers }: { node: WorkflowNode; set: SetParam; wsId: string; pickers: InspectorOptions } = $props();
  const sid = $derived(paramStr(node, 'swarm_id'));
  const swarmOpts = $derived(pickers.swarms.rows(wsId));
  const projOpts = $derived(sid ? (pickers.swarmProjects.rows(sid) ?? undefined) : undefined);
  $effect(() => {
    const w = wsId;
    const s = sid;
    const opts = pickers.swarms.rows(w); // re-checked once the list lands
    untrack(() => {
      pickers.swarms.ensure(w);
      // Only a COMPLETE id: one the list knows, or (free text) a full ULID.
      const complete = opts ? opts.some((o) => o.id === s) : isUlid(s);
      if (s && complete) pickers.swarmProjects.ensure(s);
    });
  });
</script>

<label for="np-swarm">Swarm</label>
{#if swarmOpts}
  <select
    id="np-swarm"
    value={sid}
    onchange={(e) => { set('swarm_id', e.currentTarget.value); set('project_id', ''); }}
  >
    <option value="">Choose a swarm…</option>
    {#each swarmOpts as o (o.id)}<option value={o.id}>{o.name}</option>{/each}
    {#if sid && !swarmOpts.some((o) => o.id === sid)}<option value={sid}>Unknown swarm ({sid})</option>{/if}
  </select>
  {#if swarmOpts.length === 0}<p class="node-hint">No swarms in this workspace yet — create one on the Swarm page.</p>{/if}
{:else}
  <input dir="auto"
    id="np-swarm"
    type="text"
    placeholder="Swarm id"
    value={sid}
    oninput={(e) => set('swarm_id', e.currentTarget.value)}
  />
{/if}
<label for="np-proj">Project</label>
{#if projOpts}
  <select
    id="np-proj"
    value={paramStr(node, 'project_id')}
    onchange={(e) => set('project_id', e.currentTarget.value)}
  >
    <option value="">Choose a project…</option>
    {#each projOpts as o (o.id)}<option value={o.id}>{o.name}</option>{/each}
    {#if paramStr(node, 'project_id') && !projOpts.some((o) => o.id === paramStr(node, 'project_id'))}
      <option value={paramStr(node, 'project_id')}>Unknown project ({paramStr(node, 'project_id')})</option>
    {/if}
  </select>
{:else}
  <input dir="auto"
    id="np-proj"
    type="text"
    placeholder={sid ? 'Swarm project id' : 'Choose a swarm first'}
    value={paramStr(node, 'project_id')}
    oninput={(e) => set('project_id', e.currentTarget.value)}
  />
{/if}
<label for="np-title">Task title</label>
<input dir="auto"
  id="np-title"
  type="text"
  placeholder="Workflow-generated task title"
  value={paramStr(node, 'title')}
  oninput={(e) => set('title', e.currentTarget.value)}
/>
<label for="np-desc">Description (optional)</label>
<textarea dir="auto"
  id="np-desc"
  class="np-prompt"
  rows="6"
  placeholder="Task details…"
  value={paramStr(node, 'description')}
  oninput={(e) => set('description', e.currentTarget.value)}
></textarea>
