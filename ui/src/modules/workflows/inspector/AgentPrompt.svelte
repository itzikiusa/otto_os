<script lang="ts">
  // agent_prompt: the prompt, provider/model and skills.
  import type { WorkflowNode } from '../../../lib/api/types';
  import ProviderModel from './ProviderModel.svelte';
  import { listValue, paramList, paramStr, type SetParam } from './util';

  let { node, set }: { node: WorkflowNode; set: SetParam } = $props();
</script>

<label for="np-prompt">Prompt</label>
<textarea dir="auto"
  id="np-prompt"
  class="np-prompt np-prompt-lg"
  rows="14"
  value={paramStr(node, 'prompt')}
  oninput={(e) => set('prompt', e.currentTarget.value)}
></textarea>
<ProviderModel {node} {set} />
<label for="np-skills">Skills (comma-separated)</label>
<input dir="auto"
  id="np-skills"
  type="text"
  placeholder="e.g. golang-testing, golang-code-review"
  value={paramList(node, 'skills')}
  oninput={(e) => set('skills', listValue(e.currentTarget.value))}
/>
