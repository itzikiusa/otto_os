<script lang="ts">
  // product_publish: a dry-run preview (story, RFC or Jira story, destination),
  // or — live — the publish of exactly what the approved preview showed.
  import type { WorkflowNode } from '../../../lib/api/types';
  import IdPicker from './IdPicker.svelte';
  import type { InspectorOptions } from './options.svelte';
  import { jiraAccountOptions, paramBool, paramStr, storyOptions, type SetParam } from './util';

  let { node, set, wsId, pickers }: { node: WorkflowNode; set: SetParam; wsId: string; pickers: InspectorOptions } = $props();
</script>

<label class="np-chk">
  <input
    type="checkbox"
    checked={paramBool(node, 'dry_run', true)}
    onchange={(e) => set('dry_run', e.currentTarget.checked)}
  /> Dry run (preview only)
</label>
{#if paramBool(node, 'dry_run', true)}
<label for="np-story">Product story</label>
<IdPicker id="np-story" value={paramStr(node, 'story_id')} cache={pickers.stories} key={wsId} toOptions={storyOptions}
  noun="story" placeholder="product story id" what="product stories" onchange={(v) => set('story_id', v)} />
<label for="np-pubkind">Publish as</label>
<select
  id="np-pubkind"
  value={paramStr(node, 'kind') || 'rfc'}
  onchange={(e) => set('kind', e.currentTarget.value)}
>
  <option value="rfc">RFC (Confluence)</option>
  <option value="jira">Jira story</option>
</select>

  <label for="np-account">Account</label>
  <IdPicker id="np-account" value={paramStr(node, 'account_id')} cache={pickers.accounts} key="me" toOptions={jiraAccountOptions}
    noun="Jira account" emptyLabel="Choose a Jira / Confluence account…" placeholder="Jira/Confluence account id" what="Jira accounts"
    onchange={(v) => set('account_id', v)} />
  {#if (paramStr(node, 'kind') || 'rfc') === 'jira'}
    <label for="np-project">Project key</label>
    <input dir="auto"
      id="np-project"
      type="text"
      placeholder="e.g. PROJ"
      value={paramStr(node, 'project_key')}
      oninput={(e) => set('project_key', e.currentTarget.value)}
    />
    <label for="np-issuetype">Issue type</label>
    <input dir="auto"
      id="np-issuetype"
      type="text"
      placeholder="Story"
      value={paramStr(node, 'issue_type')}
      oninput={(e) => set('issue_type', e.currentTarget.value)}
    />
  {:else}
    <label for="np-space">Space key</label>
    <input dir="auto"
      id="np-space"
      type="text"
      placeholder="Confluence space key"
      value={paramStr(node, 'space_key')}
      oninput={(e) => set('space_key', e.currentTarget.value)}
    />
    <label for="np-parent">Parent page id (optional)</label>
    <input dir="auto"
      id="np-parent"
      type="text"
      placeholder="parent page id"
      value={paramStr(node, 'parent_id')}
      oninput={(e) => set('parent_id', e.currentTarget.value)}
    />
    <label for="np-pubtitle">Title (optional)</label>
    <input dir="auto"
      id="np-pubtitle"
      type="text"
      placeholder="page title"
      value={paramStr(node, 'title')}
      oninput={(e) => set('title', e.currentTarget.value)}
    />
  {/if}
{:else}
  <p class="insp-note">Connect a preview Product Publish node to Human Approval, then this node. It publishes the approved content and destination from that run. Existing destination overrides must match the preview; clear them or create a new live node.</p>
{/if}
