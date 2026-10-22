<script lang="ts">
  // canvas: a diagram prompt, the mode (Mermaid / Excalidraw) and the agent.
  import type { WorkflowNode } from '../../../lib/api/types';
  import ProviderModel from './ProviderModel.svelte';
  import { paramStr, type SetParam } from './util';

  let { node, set }: { node: WorkflowNode; set: SetParam } = $props();
</script>

<label for="np-cprompt">Prompt</label>
<textarea dir="auto"
  id="np-cprompt"
  class="np-prompt"
  rows="7"
  placeholder="Diagram the request flow described in the input…"
  value={paramStr(node, 'prompt')}
  oninput={(e) => set('prompt', e.currentTarget.value)}
></textarea>
<label for="np-cmode">Mode</label>
<select
  id="np-cmode"
  value={paramStr(node, 'mode') || 'mermaid'}
  onchange={(e) => set('mode', e.currentTarget.value)}
>
  <option value="mermaid">Mermaid</option>
  <option value="excalidraw">Excalidraw</option>
</select>
<ProviderModel {node} {set} />
