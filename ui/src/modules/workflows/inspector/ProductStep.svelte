<script lang="ts">
  // product_analyze / product_rewrite / product_plan: the story, an extra
  // instruction and the agent; rewrite and plan can persist a product version.
  import type { WorkflowNode } from '../../../lib/api/types';
  import IdPicker from './IdPicker.svelte';
  import ProviderModel from './ProviderModel.svelte';
  import type { InspectorOptions } from './options.svelte';
  import { paramBool, paramStr, storyOptions, type SetParam } from './util';

  let { node, set, wsId, pickers }: { node: WorkflowNode; set: SetParam; wsId: string; pickers: InspectorOptions } = $props();
</script>

<label for="np-story">Product story</label>
<IdPicker id="np-story" value={paramStr(node, 'story_id')} cache={pickers.stories} key={wsId} toOptions={storyOptions}
  noun="story" placeholder="product story id" what="product stories" onchange={(v) => set('story_id', v)} />
<label for="np-instruction">Extra instruction (optional)</label>
<input dir="auto"
  id="np-instruction"
  type="text"
  placeholder="Focus on…"
  value={paramStr(node, 'instruction')}
  oninput={(e) => set('instruction', e.currentTarget.value)}
/>
<ProviderModel {node} {set} />
{#if node.kind !== 'product_analyze'}
  <label class="np-chk">
    <input
      type="checkbox"
      checked={paramBool(node, 'persist')}
      onchange={(e) => set('persist', e.currentTarget.checked)}
    /> Persist as a product version
  </label>
{/if}
