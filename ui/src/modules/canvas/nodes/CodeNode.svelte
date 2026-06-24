<script lang="ts">
  // Code node — a fenced code block with lazy syntax highlighting (hljs is loaded
  // on demand by lib/hl). Double-click to edit the source in a textarea; the lang
  // is set from the Inspector. Highlight runs line-by-line so a fallback (escaped
  // plain text) shows instantly before hljs arrives.
  import { Handle, Position } from '@xyflow/svelte';
  import type { CanvasNode } from '../types';
  import { canvas } from '../../../lib/stores/canvas.svelte';
  import { ensureHljs, highlightLine } from '../../../lib/hl';

  interface Props {
    id: string;
    data: { node: CanvasNode };
    selected?: boolean;
  }
  let { id, data, selected }: Props = $props();

  const node = $derived(data.node);
  const value = $derived(node.code?.value ?? '');
  const lang = $derived(node.code?.lang ?? null);

  // Bump when hljs finishes loading so the highlighted lines re-derive.
  let hlReady = $state(false);
  $effect(() => {
    // Touch lang so a lang change re-checks readiness.
    void lang;
    void ensureHljs().then(() => {
      hlReady = true;
    });
  });

  // Per-line highlighted HTML (trusted: hl escapes on fallback).
  const lines = $derived.by(() => {
    void hlReady; // re-run once hljs is available
    return value.split('\n').map((ln) => highlightLine(ln, lang));
  });

  let editing = $state(false);
  let draft = $state('');

  function startEdit(): void {
    draft = value;
    editing = true;
  }
  function commit(): void {
    editing = false;
    if (!canvas.scene) return;
    const patched: CanvasNode = { ...node, code: { ...node.code, value: draft } };
    canvas.setScene({
      ...canvas.scene,
      nodes: canvas.scene.nodes.map((n) => (n.id === id ? patched : n)),
    });
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="code" class:selected ondblclick={startEdit}>
  <Handle type="target" position={Position.Left} />
  <div class="bar">
    <span class="lang">{lang ?? 'text'}</span>
  </div>
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
  {:else}
    <pre class="src"><code
        >{#each lines as ln}<span class="ln">{@html ln || ' '}</span>{/each}</code
      ></pre>
  {/if}
  <Handle type="source" position={Position.Right} />
</div>

<style>
  .code {
    width: 100%;
    height: 100%;
    display: flex;
    flex-direction: column;
    background: var(--term-bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
    font-family: var(--font-mono);
  }
  .code.selected {
    outline: 1px solid var(--accent);
  }
  .bar {
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    padding: 4px 10px;
    background: var(--surface-2);
    border-bottom: 1px solid var(--border);
  }
  .lang {
    font-size: 10.5px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
  }
  .src {
    flex: 1 1 auto;
    margin: 0;
    padding: 8px 10px;
    overflow: auto;
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
    flex: 1 1 auto;
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
