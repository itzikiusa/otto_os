<script lang="ts">
  // prepare_context: fetch the input's Jira ticket into a context file, then
  // (with a prompt) run an agent to consolidate a brief.
  import type { WorkflowNode } from '../../../lib/api/types';
  import IdPicker from './IdPicker.svelte';
  import ProviderModel from './ProviderModel.svelte';
  import type { InspectorOptions } from './options.svelte';
  import { jiraAccountOptions, listValue, paramBool, paramList, paramStr, type SetParam } from './util';

  let { node, set, pickers }: { node: WorkflowNode; set: SetParam; pickers: InspectorOptions } = $props();
</script>

<p class="insp-note">
  Fetches the Jira ticket from the input (if any) into a context file, then
  — if you give it a prompt — runs an agent to consolidate a brief. The
  Provider/Model below drive that agent phase.
</p>
<label for="np-pc-prompt">Prompt (optional — runs an agent when set)</label>
<textarea dir="auto"
  id="np-pc-prompt"
  class="np-prompt np-prompt-lg"
  rows="14"
  placeholder="e.g. Read the ticket + repos and produce a consolidated brief…"
  value={paramStr(node, 'prompt')}
  oninput={(e) => set('prompt', e.currentTarget.value)}
></textarea>
<ProviderModel {node} {set} />
<label for="np-pc-skills">Skills (comma-separated)</label>
<input dir="auto"
  id="np-pc-skills"
  type="text"
  placeholder="e.g. golang-testing"
  value={paramList(node, 'skills')}
  oninput={(e) => set('skills', listValue(e.currentTarget.value))}
/>
<label for="np-pc-account">Jira account (optional — else the default)</label>
<IdPicker id="np-pc-account" value={paramStr(node, 'account_id')} cache={pickers.accounts} key="me" toOptions={jiraAccountOptions}
  noun="Jira account" emptyLabel="The default Jira account" placeholder="inherits the default Jira account" what="Jira accounts"
  onchange={(v) => set('account_id', v || undefined)} />
<label class="np-chk">
  <input
    type="checkbox"
    checked={paramBool(node, 'require')}
    onchange={(e) => set('require', e.currentTarget.checked || undefined)}
  /> Fail the run if the Jira fetch fails
</label>
