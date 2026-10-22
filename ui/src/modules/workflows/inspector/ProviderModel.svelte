<script lang="ts">
  // Shared Provider + Model editor for every agent-running node — sourced from
  // the live registry (built-ins + custom, e.g. grok) so no node is hardcoded
  // to claude. Empty provider = default.
  import ModelPicker from '../../../lib/components/ModelPicker.svelte';
  import { agentProviders, defaultAgentProvider } from '../../../lib/providers';
  import type { WorkflowNode } from '../../../lib/api/types';
  import { paramStr, type SetParam } from './util';

  let { node, set }: { node: WorkflowNode; set: SetParam } = $props();
</script>

<label for="np-provider">Provider</label>
<select id="np-provider" value={paramStr(node, 'provider') || ''} onchange={(e) => set('provider', e.currentTarget.value || undefined)}>
  <option value="">default ({defaultAgentProvider()})</option>
  {#each agentProviders() as p (p)}<option value={p}>{p}</option>{/each}
</select>
<!-- Empty provider = default → resolve so the picker lists the models that
     will actually run. Hides itself for a provider with no model-flag template.
     Wrapped so the form's input rules never restyle the picker's own controls. -->
<div class="np-mp">
  <ModelPicker provider={paramStr(node, 'provider') || defaultAgentProvider()} value={paramStr(node, 'model')} onchange={(m) => set('model', m)} />
</div>
