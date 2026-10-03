<script lang="ts">
  // JSON tree view: the DB Explorer's lazily-rendered collapsible tree
  // (database/JsonTree.svelte, standalone mode — a collapsed container renders
  // one summary line, long arrays chunk) so a multi-MB document stays cheap.
  // Parse errors show inline with line/column.
  import JsonTree from '../../database/JsonTree.svelte';
  import { parseJsonWithPos } from './kinds';
  import PreviewError from './PreviewError.svelte';

  interface Props {
    content: string;
  }
  let { content }: Props = $props();

  const parsed = $derived(parseJsonWithPos(content));
</script>

<div class="json">
  {#if parsed.ok}
    <div class="tree"><JsonTree value={parsed.value} /></div>
  {:else if !content.trim()}
    <div class="state">Empty document — nothing to show yet.</div>
  {:else}
    <PreviewError title="Invalid JSON" message={parsed.error ?? 'Parse error'} line={parsed.line} col={parsed.col} />
  {/if}
</div>

<style>
  .json {
    padding: 8px 12px;
    font-family: var(--font-mono);
    font-size: var(--fs-s);
  }
  .tree {
    overflow-x: auto;
  }
  .state {
    padding: 12px;
    color: var(--text-dim);
    font-family: inherit;
    font-size: var(--fs-m);
  }
</style>
