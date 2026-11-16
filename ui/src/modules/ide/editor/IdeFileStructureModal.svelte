<script lang="ts">
  // GoLand-style Cmd+F12 File Structure / Move-to-Function / Move-to-Key / Move-to-Column modal.
  import Badge from '../../../lib/components/Badge.svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Icon, { type IconName } from '../../../lib/components/Icon.svelte';
  import Modal from '../../../lib/components/Modal.svelte';
  import type { IdeSymbolNode } from '../store/ideTypes';

  interface Props {
    open: boolean;
    fileName: string;
    symbols: IdeSymbolNode[];
    currentLine: number;
    onjump: (line: number, column: number) => void;
    onclose: () => void;
  }

  let { open, fileName, symbols, currentLine, onjump, onclose }: Props = $props();

  let query = $state('');
  let selectedIdx = $state(0);
  let listEl = $state<HTMLDivElement | undefined>(undefined);

  interface FlatSymbol {
    node: IdeSymbolNode;
    depth: number;
  }

  const flattened = $derived.by((): FlatSymbol[] => {
    const out: FlatSymbol[] = [];
    const walk = (items: IdeSymbolNode[], depth: number) => {
      for (let i = 0; i < items.length; i++) {
        const node = items[i];
        out.push({ node, depth });
        if (node.children?.length) walk(node.children, depth + 1);
      }
    };
    walk(symbols, 0);
    return out;
  });

  const filtered = $derived.by((): FlatSymbol[] => {
    const q = query.trim().toLowerCase();
    if (!q || q.startsWith(':')) return flattened;
    return flattened.filter(
      (f) => f.node.name.toLowerCase().includes(q) || f.node.kind.toLowerCase().includes(q),
    );
  });

  const lineJumpTarget = $derived.by((): number | null => {
    const q = query.trim();
    const m = /^:?(\d+)$/.exec(q);
    if (!m) return null;
    const n = Number(m[1]);
    return Number.isFinite(n) && n >= 1 ? n : null;
  });

  let wasOpen = false;
  $effect(() => {
    if (open && !wasOpen) {
      query = '';
      const activeIdx = flattened.findIndex(
        (f) => currentLine >= f.node.line && currentLine <= Math.max(f.node.line, f.node.end_line),
      );
      selectedIdx = activeIdx >= 0 ? activeIdx : 0;
    }
    wasOpen = open;
  });

  function iconForKind(kind: string): IconName {
    const k = kind.toLowerCase();
    if (k.includes('func') || k.includes('method')) return 'function';
    if (k.includes('test')) return 'flask';
    if (k.includes('struct') || k.includes('class') || k.includes('interface') || k.includes('trait')) return 'shapes';
    if (k.includes('field') || k.includes('prop') || k.includes('var')) return 'tag';
    return 'layers';
  }

  function scrollSelectedIntoView(): void {
    queueMicrotask(() => {
      listEl?.querySelector<HTMLElement>('[aria-selected="true"]')?.scrollIntoView({ block: 'nearest' });
    });
  }

  function activateSelected(): void {
    if (lineJumpTarget !== null && (query.trim().startsWith(':') || filtered.length === 0)) {
      onjump(lineJumpTarget, 1);
      onclose();
      return;
    }
    const target = filtered[selectedIdx];
    if (!target) return;
    onjump(target.node.line, (target.node as { column?: number }).column || 1);
    onclose();
  }

  function onKeydown(e: KeyboardEvent): void {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      if (filtered.length > 0) {
        selectedIdx = (selectedIdx + 1) % filtered.length;
        scrollSelectedIntoView();
      }
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      if (filtered.length > 0) {
        selectedIdx = (selectedIdx - 1 + filtered.length) % filtered.length;
        scrollSelectedIntoView();
      }
    } else if (e.key === 'Enter') {
      e.preventDefault();
      activateSelected();
    }
  }
</script>

{#if open}
  <Modal title="File structure · {fileName}" width={540} onclose={onclose}>
    <div class="fs-wrap" data-testid="ide-file-structure-modal">
      <div class="fs-input-row">
        <Icon name="search" size={14} />
        <input
          class="fs-input"
          type="search"
          dir="ltr"
          data-autofocus
          aria-label="Filter file symbols or enter :line"
          placeholder="Filter functions, structs, keys, columns, or type :line…"
          bind:value={query}
          oninput={() => (selectedIdx = 0)}
          onkeydown={onKeydown}
          data-testid="ide-structure-input"
        />
      </div>

      {#if lineJumpTarget !== null && (query.trim().startsWith(':') || filtered.length === 0)}
        <button type="button" class="fs-row selected" onclick={activateSelected}>
          <Icon name="arrow" size={13} />
          <span class="fs-name mono" dir="ltr">Jump to line {lineJumpTarget}</span>
          <Badge tone="accent" variant="outline" label={`L${lineJumpTarget}`} />
        </button>
      {:else if filtered.length === 0}
        <EmptyState
          icon="layers"
          title="No matching structure items"
          body="Type :line (for example :42) to jump directly to any line number."
        />
      {:else}
        <div class="fs-list" role="listbox" aria-label="File symbols" bind:this={listEl}>
          {#each filtered as item, idx (`${item.node.name}:${item.node.line}:${idx}`)}
            <button
              type="button"
              class="fs-row"
              class:selected={idx === selectedIdx}
              role="option"
              aria-selected={idx === selectedIdx}
              style:padding-inline-start={`${10 + item.depth * 16}px`}
              onmouseenter={() => (selectedIdx = idx)}
              onclick={() => {
                onjump(item.node.line, (item.node as { column?: number }).column || 1);
                onclose();
              }}
            >
              <Icon name={iconForKind(item.node.kind)} size={13} />
              <span class="fs-name mono" dir="ltr">{item.node.name}</span>
              <Badge tone="neutral" variant="outline" label={item.node.kind} />
              <span class="fs-line mono" dir="ltr">:{item.node.line}</span>
            </button>
          {/each}
        </div>
      {/if}
    </div>
  </Modal>
{/if}

<style>
  .fs-wrap {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
    min-inline-size: 0;
  }
  .fs-input-row {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    padding-inline: var(--sp-4);
    background: var(--surface-2);
    border: 1px solid var(--control-border);
    border-radius: var(--radius-s);
    color: var(--text-dim);
  }
  .fs-input-row:focus-within {
    border-color: var(--accent-text);
    box-shadow: 0 0 0 2px var(--accent-soft);
  }
  .fs-input {
    flex: 1;
    min-inline-size: 0;
    block-size: 32px;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: var(--fs-s);
    outline: none;
  }
  .fs-list {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    max-block-size: 360px;
    overflow-y: auto;
  }
  .fs-row {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    inline-size: 100%;
    padding-block: var(--sp-3);
    padding-inline-end: var(--sp-4);
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font-size: var(--fs-xs);
    text-align: start;
    cursor: pointer;
  }
  .fs-row.selected,
  .fs-row:hover {
    background: var(--accent-soft);
  }
  .fs-name {
    flex: 1;
    min-inline-size: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .fs-line {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
</style>
