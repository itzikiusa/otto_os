<script lang="ts">
  // git_pr: repository / base inherit the reference the implementer or reviewer
  // used; open on pass and detect changed repos.
  import type { WorkflowNode } from '../../../lib/api/types';
  import RepoPicker from './RepoPicker.svelte';
  import type { InspectorOptions } from './options.svelte';
  import { paramBool, paramStr, type SetParam } from './util';

  let { node, set, wsId, pickers }: { node: WorkflowNode; set: SetParam; wsId: string; pickers: InspectorOptions } = $props();
</script>

<p class="insp-note">
  Leave Repository and Base empty to <strong>inherit the reference</strong> the
  implementer/reviewer used (the run’s working folder and base, or the upstream
  review). Set them only to override. A run that changed several repos opens
  <strong>one PR per repo</strong> (from fanned-in reviews, or enable “detect changed”).
</p>
<label for="np-repo">Repository (optional — inherits from reference)</label>
<RepoPicker id="np-repo" emptyLabel="Inherit from the upstream review / working folder" {node} {set} {wsId} {pickers} />
<label for="np-base">Base branch (optional — inherits from reference)</label>
<input dir="auto"
  id="np-base"
  type="text"
  placeholder="inherits (per-repo base), else main"
  value={paramStr(node, 'base')}
  oninput={(e) => set('base', e.currentTarget.value)}
/>
<label class="np-chk">
  <input
    type="checkbox"
    checked={paramBool(node, 'open')}
    onchange={(e) => set('open', e.currentTarget.checked)}
  /> Open PR automatically on pass (gate the incoming edge on the review passing)
</label>
<label class="np-chk">
  <input
    type="checkbox"
    checked={paramBool(node, 'detect_changed')}
    onchange={(e) => set('detect_changed', e.currentTarget.checked)}
  /> Detect changed repos — open a PR for every registered repo that has changes
</label>
