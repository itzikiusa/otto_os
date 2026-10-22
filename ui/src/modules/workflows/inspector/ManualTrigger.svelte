<script lang="ts">
  // manual_trigger: the run input the workflow starts with, plus the Slack
  // message that triggers THIS workflow by name.
  import type { WorkflowNode } from '../../../lib/api/types';
  import PathField from '../../../lib/components/PathField.svelte';
  import { copyTextOrThrow } from '../../../lib/clipboard';
  import { toasts } from '../../../lib/toast.svelte';
  import IdPicker from './IdPicker.svelte';
  import RepoPicker from './RepoPicker.svelte';
  import type { InspectorOptions } from './options.svelte';
  import { linesValue, paramLines, paramStr, storyOptions, type SetParam } from './util';

  let { node, set, wsId, pickers, workflowName }: {
    node: WorkflowNode; set: SetParam; wsId: string; pickers: InspectorOptions; workflowName: string | undefined;
  } = $props();

  // Copy-paste Slack message that triggers THIS workflow by name.
  let mtCopied = $state(false);
  const mtSlackSnippet = $derived(
    `@otto\n` +
      `Action: Workflow\n` +
      `Name: ${workflowName ?? '<workflow name>'}\n` +
      `Msg: what you want done — instructions for the agents\n` +
      `Jira ticket: PROJ-1234\n` +
      `Working Directory: ~/path/to/repo\n` +
      `Relevant Info: ~/path/a, ~/path/b\n` +
      `Goals:\n  - 100% test coverage (services)\n  - under 2 minutes runtime`,
  );
  async function copyMtSlack(): Promise<void> {
    try {
      await copyTextOrThrow(mtSlackSnippet);
      mtCopied = true;
      setTimeout(() => (mtCopied = false), 1500);
    } catch {
      toasts.error('Couldn’t copy to the clipboard', 'Select the text and copy it manually.');
    }
  }
</script>

<p class="insp-note">
  These fields are the <strong>run input</strong> the workflow starts with — fill them
  in here, then press <strong>Run</strong> (top-right). A Slack trigger or the
  <strong>Run…</strong> editor override them per key.
</p>
<label for="mt-msg">Message / prompt</label>
<textarea dir="auto"
  id="mt-msg"
  class="np-prompt"
  rows="6"
  placeholder="What you want done — instructions for the agents"
  value={paramStr(node, 'msg')}
  oninput={(e) => set('msg', e.currentTarget.value)}
></textarea>
<label for="mt-wd">Working folder (where agents run)</label>
<PathField value={paramStr(node, 'working_directory')} onpick={(path) => set('working_directory', path)}><input dir="ltr"
  id="mt-wd"
  type="text"
  placeholder="~/path/to/repo (default: workspace root)"
  value={paramStr(node, 'working_directory')}
  oninput={(e) => set('working_directory', e.currentTarget.value)}
/></PathField>
<label for="mt-repo">Repository (for review / PR steps)</label>
<RepoPicker id="mt-repo" emptyLabel="None" {node} {set} {wsId} {pickers} />
<label for="mt-base">Base branch</label>
<input dir="auto"
  id="mt-base"
  type="text"
  placeholder="main"
  value={paramStr(node, 'base')}
  oninput={(e) => set('base', e.currentTarget.value)}
/>
<label for="mt-story">Product story (for product steps)</label>
<IdPicker id="mt-story" value={paramStr(node, 'story_id')} cache={pickers.stories} key={wsId} toOptions={storyOptions}
  noun="story" placeholder="product story id" what="product stories" onchange={(v) => set('story_id', v)} />
<label for="mt-jira">Jira ticket</label>
<input dir="auto"
  id="mt-jira"
  type="text"
  placeholder="PROJ-1234"
  value={paramStr(node, 'jira_ticket')}
  oninput={(e) => set('jira_ticket', e.currentTarget.value)}
/>
<label for="mt-goals">Goals (one per line)</label>
<textarea dir="auto"
  id="mt-goals"
  class="np-prompt"
  rows="6"
  placeholder={'100% test coverage (services)\nunder 2 minutes runtime'}
  value={paramLines(node, 'goals')}
  oninput={(e) => set('goals', linesValue(e.currentTarget.value))}
></textarea>
<div class="insp-slack">
  <div class="is-head">
    <strong>Or trigger from Slack</strong>
    <button class="btn small" onclick={copyMtSlack}>{mtCopied ? 'Copied' : 'Copy'}</button>
  </div>
  <p class="insp-note">Post this where the Otto bot is configured for your workspace (matched by Name):</p>
  <pre class="is-snip">{mtSlackSnippet}</pre>
</div>
