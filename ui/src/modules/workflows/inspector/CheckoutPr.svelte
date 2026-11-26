<script lang="ts">
  // git_checkout — "Check out PR / branch" (engine.md §7.1). The "never guess"
  // rules are code, so they are shown, not edited; the one choice is the mode.
  import type { WorkflowNode } from '../../../lib/api/types';
  import { paramStr, type SetParam } from './util';

  let { node, set }: { node: WorkflowNode; set: SetParam } = $props();
  const ids = $props.id();
  const mode = $derived(paramStr(node, 'mode') === 'branch' ? 'branch' : 'pr');
</script>

<label for="{ids}-mode">Check out</label>
<select id="{ids}-mode" value={mode} onchange={(e) => set('mode', e.currentTarget.value === 'branch' ? 'branch' : undefined)}>
  <option value="pr">The pull request</option>
  <option value="branch">A branch (no pull request)</option>
</select>
{#if mode === 'pr'}
  <p class="insp-note">Finds the PR from: 1. the <code>pr</code> input · 2. the <code>pr_branch</code> input · 3. a branch starting with the Jira key.
    If there is not exactly one open PR, the step fails.</p>
{:else}
  <p class="insp-note">The branch from: 1. the <code>pr_branch</code> input · 2. the repository’s declared source · 3. <code>feature/&lt;Jira key&gt;</code>.
    The base is the <code>base</code> input, else the repository’s default branch.</p>
{/if}
<p class="insp-note">For each repository in the run’s <code>repos.json</code>. Writes <code>repos.json</code> (source, base, base branch); an unresolved repository keeps its reason there.</p>
