<script lang="ts">
  // post_review_comments — "Post review comments" (engine.md §7.3): the review's
  // findings posted on the PR by code (no agent), each at most once.
  import type { WorkflowGraph, WorkflowNode } from '../../../lib/api/types';
  import { paramBool, paramNum, paramStr, type SetParam } from './util';
  import { ancestors } from '../editor/stepRules';

  let { node, graph, set }: { node: WorkflowNode; graph: WorkflowGraph; set: SetParam } = $props();
  const ids = $props.id();
  const before = $derived(ancestors(graph, node.id));
  const reviews = $derived(graph.nodes.filter((n) => n.kind === 'review_run' && before.has(n.id)));
  const writers = $derived(graph.nodes.filter((n) => before.has(n.id) && n.kind === 'agent_prompt'));
  const params = $derived((node.params ?? {}) as Record<string, unknown>);
  const inline = $derived(params.inline === undefined || (Array.isArray(params.inline) && params.inline.length > 0));
  const files = $derived(Array.isArray(params.summary_files) ? (params.summary_files as string[]).join(', ') : '');
</script>

<label for="{ids}-from">Review</label>
{#if reviews.length}
  <select id="{ids}-from" value={paramStr(node, 'review_from')} onchange={(e) => set('review_from', e.currentTarget.value)}>
    <option value="">The nearest review step before this one</option>
    {#each reviews as r (r.id)}<option value={r.id}>{r.name || r.id}</option>{/each}
  </select>
{:else}
  <p class="pr-err">No review step comes before this one. Connect it after a review step.</p>
{/if}
<span class="np-label np-sec">Post</span>
<label class="np-chk"><input type="checkbox" checked={inline} onchange={(e) => set('inline', e.currentTarget.checked ? undefined : [])} /> Bugs and warnings as inline comments</label>
<label class="np-chk"><input type="checkbox" checked={params.info !== 'skip'} onchange={(e) => set('info', e.currentTarget.checked ? undefined : 'skip')} /> Notes as one combined comment</label>
<label class="np-chk"><input type="checkbox" checked={paramBool(node, 'summary', true)} onchange={(e) => set('summary', e.currentTarget.checked ? undefined : false)} /> A summary comment with the score</label>
<p class="node-hint">A comment already on the PR (same file, line and first line) is skipped, so a re-run never posts it twice.</p>
<label for="{ids}-max">At most this many inline comments</label>
<input id="{ids}-max" type="number" min="1" max="200" value={paramNum(node, 'max_inline', 150)}
  oninput={(e) => { const v = Number(e.currentTarget.value); set('max_inline', v > 0 && v !== 150 ? Math.min(200, v) : undefined); }} />
<p class="node-hint">Bitbucket allows 200 per PR; the rest go into one “More findings” comment.</p>
<details class="np-sec">
  <summary>The summary’s text</summary>
  <label for="{ids}-intro">Open the summary with the answer of</label>
  <select id="{ids}-intro" value={paramStr(node, 'summary_intro_from')} onchange={(e) => set('summary_intro_from', e.currentTarget.value)}>
    <option value="">Nothing</option>
    {#each writers as w (w.id)}<option value={w.id}>{w.name || w.id}</option>{/each}
  </select>
  <label for="{ids}-files">Add these run files to it (file names, comma-separated)</label>
  <input dir="ltr" id="{ids}-files" type="text" placeholder="gates.md" value={files}
    oninput={(e) => { const v = e.currentTarget.value.split(',').map((s) => s.trim()).filter(Boolean); set('summary_files', v.length ? v : undefined); }} />
</details>

<style>
  .pr-err {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--danger);
  }
</style>
