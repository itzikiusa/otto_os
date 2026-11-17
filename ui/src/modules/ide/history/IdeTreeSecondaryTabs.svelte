<script lang="ts">
  // Secondary Project Explorer tabs (`Structure` symbol outline and the one
  // `Breakpoints` list, `debug/IdeBreakpointsList.svelte`) extracted from
  // `IdeProjectTree.svelte` to keep every file < 460 LOC.
  import Badge from '../../../lib/components/Badge.svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import { ctxMenu } from '../../../lib/contextmenu.svelte';
  import { rowMenu } from '../../../lib/rowMenu';
  import type { BadgeTone } from '../../../lib/status';
  import {
    ide,
    type IdeSymbolNode,
  } from '../ideStore.svelte';
  import { ideAgent } from '../store/ideAgentController.svelte';
  import IdeBreakpointsList from '../debug/IdeBreakpointsList.svelte';

  export interface FlatSymbolNode {
    sym: IdeSymbolNode;
    depth: number;
  }

  interface Props {
    tab: 'structure' | 'breakpoints';
    flatSymbols: FlatSymbolNode[];
  }

  let { tab, flatSymbols }: Props = $props();

  function symbolTone(kind: string): BadgeTone {
    const k = kind.toLowerCase();
    if (k.includes('func') || k.includes('method')) return 'info';
    if (k.includes('class') || k.includes('struct') || k.includes('interface') || k.includes('type')) {
      return 'accent';
    }
    if (k.includes('const') || k.includes('enum')) return 'ok';
    return 'neutral';
  }

  function shortSymbolKind(kind: string): string {
    const k = kind.toLowerCase();
    if (k.includes('method') || k.includes('func')) return 'fn';
    if (k.includes('struct')) return 'struct';
    if (k.includes('interface')) return 'iface';
    if (k.includes('class')) return 'class';
    if (k.includes('type')) return 'type';
    if (k.includes('const')) return 'const';
    if (k.includes('field') || k.includes('prop') || k.includes('key') || k.includes('col')) return 'prop';
    return kind.slice(0, 5);
  }

  function openSymbolMenu(e: MouseEvent, sym: IdeSymbolNode): void {
    const path = ide.activePath;
    if (!path) return;
    ctxMenu.show(e, [
      { label: 'Jump to symbol', icon: 'target', action: () => ide.jumpTo(path, sym.line, 1) },
      {
        label: 'Add to Agent context',
        icon: 'assistant',
        action: () => ideAgent.attachSymbol(path, sym, { openPanel: true }),
      },
    ]);
  }
</script>

{#if tab === 'structure'}
  <div class="sec-panel" role="tabpanel" id="ide-explorer-panel-structure" aria-labelledby="ide-explorer-tab-structure">
    <div class="sec-scroll">
      {#if !ide.activeTab}
        <EmptyState headingLevel={2} icon="layers" title="No file open" body="Select a source file to inspect its classes, functions, keys, and columns." />
      {:else}
        <LoadState what="file outline" variant="compact" loading={ide.symbolsLoading} empty={flatSymbols.length === 0}>
          {#snippet emptyView()}
            <EmptyState headingLevel={2} icon="layers" title="No outline symbols" body="Symbols appear automatically for Go, TypeScript, Java, Rust, Python, Shell, JSON, and CSV files." />
          {/snippet}
          <div class="sec-list">
            {#each flatSymbols as item, idx (`${item.sym.name}:${item.sym.line}:${idx}`)}
              {@const isCurrent = ide.cursorInfo.line >= item.sym.line && ide.cursorInfo.line <= Math.max(item.sym.line, item.sym.end_line)}
              <div class="struct-line-wrap" class:active={isCurrent}>
                <button
                  type="button"
                  class="struct-row"
                  aria-current={isCurrent ? 'true' : undefined}
                  dir="ltr"
                  style="padding-inline-start: calc(10px + {item.depth * 12}px)"
                  onclick={() => ide.activePath && ide.jumpTo(ide.activePath, item.sym.line, 1)}
                  oncontextmenu={(e) => openSymbolMenu(e, item.sym)}
                  use:rowMenu
                >
                  <Badge variant="outline" tone={symbolTone(item.sym.kind)} label={shortSymbolKind(item.sym.kind)} />
                  <span class="sec-name">{item.sym.name}</span>
                  <span class="struct-line">:{item.sym.line}</span>
                </button>
                <button
                  type="button"
                  class="sec-icon-btn struct-agent-btn"
                  aria-label="Add {item.sym.name} to Agent context"
                  title="Add to Agent context"
                  onclick={() => ide.activePath && ideAgent.attachSymbol(ide.activePath, item.sym, { openPanel: true })}
                  data-testid="ide-symbol-add-agent"
                >
                  <Icon name="assistant" size={12} />
                </button>
              </div>
            {/each}
          </div>
        </LoadState>
      {/if}
    </div>
  </div>
{:else}
  <div class="sec-panel" role="tabpanel" id="ide-explorer-panel-breakpoints" aria-labelledby="ide-explorer-tab-breakpoints">
    <IdeBreakpointsList />
  </div>
{/if}

<style>
  .sec-panel { display: flex; flex-direction: column; flex: 1; min-block-size: 0; min-inline-size: 0; overflow: hidden; }
  .sec-scroll { flex: 1; min-block-size: 0; overflow-y: auto; overflow-x: hidden; padding-block: 4px; }
  .sec-list { display: flex; flex-direction: column; gap: 2px; }
  .sec-name { flex: 1; min-inline-size: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sec-icon-btn {
    display: inline-flex; align-items: center; justify-content: center;
    inline-size: 20px; block-size: 20px; border: 0; border-radius: var(--radius-s);
    background: transparent; color: var(--text-dim); cursor: pointer; flex-shrink: 0;
  }
  .sec-icon-btn:hover { background: var(--hover); color: var(--text); }
  .struct-agent-btn { opacity: 0; }
  .struct-line-wrap:hover .struct-agent-btn, .struct-line-wrap:focus-within .struct-agent-btn { opacity: 1; }
  .struct-line-wrap { display: flex; align-items: center; padding-inline-end: 4px; min-inline-size: 0; overflow: hidden; }
  .struct-line-wrap:hover, .struct-line-wrap.active { background: var(--surface-2); }
  .struct-row { display: flex; align-items: center; gap: 6px; flex: 1; min-inline-size: 0; padding-block: 4px; padding-inline-end: 4px; background: transparent; border: none; color: var(--text); font-size: var(--fs-xs); text-align: start; cursor: pointer; }
  .struct-line { font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-dim); flex-shrink: 0; }
  @media (hover: none) { .struct-agent-btn { opacity: 1; } }
</style>
