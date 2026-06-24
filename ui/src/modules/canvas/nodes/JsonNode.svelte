<script lang="ts">
  // JSON node — renders parsed JSON as a collapsible tree (objects/arrays expand
  // and collapse) with a "Raw" toggle that shows the source text, syntax-
  // highlighted via hljs. Invalid JSON falls back to the raw view with an error
  // chip. Double-click (in Raw mode) to edit the source.
  import { Handle, Position } from '@xyflow/svelte';
  import type { CanvasNode } from '../types';
  import { canvas } from '../../../lib/stores/canvas.svelte';
  import { ensureHljs, highlightLine } from '../../../lib/hl';
  import JsonTree from './JsonTree.svelte';

  interface Props {
    id: string;
    data: { node: CanvasNode };
    selected?: boolean;
  }
  let { id, data, selected }: Props = $props();

  const node = $derived(data.node);
  const raw = $derived(node.json?.value ?? '');

  // Parse once per source change; `error` non-null ⇒ force the Raw view.
  const parsed = $derived.by((): { value?: unknown; error?: string } => {
    try {
      return { value: JSON.parse(raw) };
    } catch (e) {
      return { error: e instanceof Error ? e.message : 'Invalid JSON' };
    }
  });

  let showRaw = $state(false);
  // Force Raw when the JSON can't parse (the tree would be meaningless).
  const effectiveRaw = $derived(showRaw || !!parsed.error);

  // Lazy hljs for the Raw view.
  let hlReady = $state(false);
  $effect(() => {
    void ensureHljs().then(() => {
      hlReady = true;
    });
  });
  const rawLines = $derived.by(() => {
    void hlReady;
    return raw.split('\n').map((ln) => highlightLine(ln, 'json'));
  });

  let editing = $state(false);
  let draft = $state('');
  function startEdit(): void {
    if (!effectiveRaw) return;
    draft = raw;
    editing = true;
  }
  function commit(): void {
    editing = false;
    if (!canvas.scene) return;
    const patched: CanvasNode = { ...node, json: { value: draft } };
    canvas.setScene({
      ...canvas.scene,
      nodes: canvas.scene.nodes.map((n) => (n.id === id ? patched : n)),
    });
  }
</script>

<div class="json" class:selected>
  <Handle type="target" position={Position.Left} />
  <div class="bar">
    <span class="title">JSON</span>
    {#if parsed.error}<span class="err">invalid</span>{/if}
    <button
      class="toggle"
      class:active={effectiveRaw}
      disabled={!!parsed.error}
      onclick={() => (showRaw = !showRaw)}
      title="Toggle raw / tree view"
    >
      Raw
    </button>
  </div>

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="body" ondblclick={startEdit}>
    {#if editing}
      <!-- svelte-ignore a11y_autofocus -->
      <textarea
        bind:value={draft}
        autofocus
        spellcheck="false"
        onblur={commit}
        onkeydown={(e) => {
          if (e.key === 'Escape') editing = false;
        }}
      ></textarea>
    {:else if effectiveRaw}
      {#if parsed.error}<div class="errline">{parsed.error}</div>{/if}
      <pre class="src"><code
          >{#each rawLines as ln}<span class="ln">{@html ln || ' '}</span>{/each}</code
        ></pre>
    {:else}
      <div class="tree">
        <JsonTree value={parsed.value} name={null} depth={0} />
      </div>
    {/if}
  </div>
  <Handle type="source" position={Position.Right} />
</div>

<style>
  .json {
    width: 100%;
    height: 100%;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
  }
  .json.selected {
    outline: 1px solid var(--accent);
  }
  .bar {
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px;
    background: var(--surface-2);
    border-bottom: 1px solid var(--border);
  }
  .title {
    font-size: 10.5px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
  }
  .err {
    font-size: 10px;
    color: var(--status-exited);
    margin-right: auto;
  }
  .toggle {
    margin-left: auto;
    font-size: 11px;
    padding: 1px 7px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
  }
  .toggle.active {
    background: var(--accent);
    color: var(--accent-contrast);
    border-color: transparent;
  }
  .toggle:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .body {
    flex: 1 1 auto;
    overflow: auto;
    min-height: 0;
  }
  .tree {
    padding: 8px 10px;
    font-family: var(--font-mono);
    font-size: 12px;
    line-height: 1.5;
    color: var(--text);
  }
  .errline {
    padding: 6px 10px;
    font-size: 11px;
    color: var(--status-exited);
    border-bottom: 1px solid var(--border);
  }
  .src {
    margin: 0;
    padding: 8px 10px;
    font-family: var(--font-mono);
    font-size: 12px;
    line-height: 1.5;
    color: var(--text);
  }
  .src code {
    display: block;
    white-space: pre;
  }
  .ln {
    display: block;
  }
  textarea {
    width: 100%;
    height: 100%;
    resize: none;
    border: none;
    outline: none;
    background: var(--term-bg);
    color: var(--text);
    font-family: var(--font-mono);
    font-size: 12px;
    line-height: 1.5;
    padding: 8px 10px;
  }
</style>
