<script lang="ts">
  // Collapsible JSON tree for the response viewer. Objects/arrays fold; the
  // first two levels start open. A search query highlights matching keys and
  // values and opens their ancestors. Right-click (or the row's menu key) a
  // node to copy its JSONPath or value. Large containers render in pages.
  import Icon from '../../lib/components/Icon.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { copyTextOrThrow } from '../../lib/clipboard';
  import { toasts } from '../../lib/toast.svelte';
  import { childPath, preview, searchTree } from '../../lib/api/jsonTree';

  interface Props {
    value: unknown;
    query?: string;
  }
  let { value, query = '' }: Props = $props();

  const PAGE = 200;
  const found = $derived(searchTree(value, query));
  let toggled = $state<Record<string, boolean>>({});
  let shown = $state<Record<string, number>>({});

  function isOpen(path: string, depth: number): boolean {
    if (path in toggled) return toggled[path];
    if (query.trim()) return found.open.has(path);
    return depth < 2;
  }
  function toggle(path: string, depth: number): void {
    toggled[path] = !isOpen(path, depth);
  }
  function entries(v: unknown): [string | number, unknown][] {
    return Array.isArray(v) ? v.map((x, i) => [i, x]) : Object.entries(v as Record<string, unknown>);
  }
  function kind(v: unknown): string {
    return v === null ? 'null' : Array.isArray(v) ? 'array' : typeof v;
  }
  async function copy(text: string, what: string): Promise<void> {
    try {
      await copyTextOrThrow(text);
      toasts.success(`Copied ${what}`);
    } catch {
      toasts.error('Couldn’t copy', 'The clipboard isn’t available.');
    }
  }
  function menu(e: MouseEvent | KeyboardEvent, path: string, v: unknown): void {
    ctxMenu.show(e, [
      { label: 'Copy JSONPath', icon: 'link', action: () => void copy(path, 'JSONPath') },
      { label: 'Copy value', icon: 'copy', action: () => void copy(typeof v === 'string' ? v : JSON.stringify(v, null, 2), 'value') },
    ]);
  }
</script>

<div class="jtree mono" dir="ltr">
  {@render node(value, '$', null, 0)}
</div>

{#snippet node(v: unknown, path: string, key: string | number | null, depth: number)}
  {@const container = v !== null && typeof v === 'object'}
  {@const open = container && isOpen(path, depth)}
  <div class="row" class:hit={found.hits.has(path)} style:padding-inline-start="{depth * 16}px">
    {#if container}
      <button class="tog" onclick={() => toggle(path, depth)} oncontextmenu={(e) => menu(e, path, v)} aria-expanded={open} title={path}>
        <Icon name={open ? 'chevronDown' : 'chevronRight'} size={12} />
        {#if key !== null}<span class="k">{typeof key === 'number' ? key : `"${key}"`}</span><span class="p">:</span>{/if}
        <span class="p">{Array.isArray(v) ? '[' : '{'}</span>
        {#if !open}<span class="pv">{preview(v)}</span><span class="p">{Array.isArray(v) ? ']' : '}'}</span>{/if}
      </button>
    {:else}
      <button class="leaf" oncontextmenu={(e) => menu(e, path, v)} onkeydown={(e) => { if (e.key === 'ContextMenu' || (e.shiftKey && e.key === 'F10')) menu(e, path, v); }} title={path}>
        {#if key !== null}<span class="k">{typeof key === 'number' ? key : `"${key}"`}</span><span class="p">:</span>{/if}
        <span class="v {kind(v)}">{typeof v === 'string' ? JSON.stringify(v) : String(v)}</span>
      </button>
    {/if}
  </div>
  {#if open}
    {@const all = entries(v)}
    {@const limit = shown[path] ?? PAGE}
    {#each all.slice(0, limit) as [k, child] (k)}
      {@render node(child, childPath(path, k), k, depth + 1)}
    {/each}
    {#if all.length > limit}
      <div class="row" style:padding-inline-start="{(depth + 1) * 16}px">
        <button class="more" onclick={() => (shown[path] = limit + PAGE)}>Show {Math.min(PAGE, all.length - limit)} more of {all.length - limit}</button>
      </div>
    {/if}
    <div class="row close" style:padding-inline-start="{depth * 16 + 14}px"><span class="p">{Array.isArray(v) ? ']' : '}'}</span></div>
  {/if}
{/snippet}

<style>
  .jtree {
    font-size: var(--fs-s);
    line-height: 1.6;
    padding: 6px 8px;
    user-select: text;
  }
  .row {
    display: flex;
    align-items: center;
    min-height: 20px;
    border-radius: var(--radius-s);
  }
  .row.hit {
    background: var(--warning-soft);
  }
  .tog,
  .leaf {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
    max-width: 100%;
    padding: 0 4px;
    border: none;
    background: transparent;
    color: var(--text);
    font: inherit;
    cursor: pointer;
    text-align: start;
    border-radius: var(--radius-s);
  }
  .leaf {
    cursor: text;
    padding-inline-start: 20px;
  }
  .tog:hover,
  .leaf:hover {
    background: var(--hover);
  }
  .k {
    color: var(--accent-text);
    flex-shrink: 0;
  }
  .p,
  .pv {
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .v {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .v.string { color: var(--success); }
  .v.number { color: var(--info); }
  .v.boolean { color: var(--warning); }
  .v.null { color: var(--text-dim); font-style: italic; }
  .more {
    border: none;
    background: transparent;
    color: var(--accent-text);
    font: inherit;
    cursor: pointer;
    padding: 0 4px 0 20px;
  }
</style>
