<script lang="ts">
  // Bottom symbol trail & quick-access bar (JetBrains GoLand / IntelliJ parity):
  // Clickable path + symbol hierarchy chain, Local History clock, and Editor Settings gear.
  import Icon from '../../../lib/components/Icon.svelte';
  import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
  import { IDE_PREFS_EVENT, readIdeEditorPreferences } from '../../../lib/components/cm-ide-theme';
  import type { IdeSymbolNode } from '../ideStore.svelte';
  import { flushAutoLocalHistorySnapshot, revealInIdeProjectTree } from './ideEditorStoreBridge.svelte';

  interface Props {
    filePath: string;
    pathSegments: string[];
    symbols: IdeSymbolNode[];
    cursorLine: number;
    onjumpsymbol: (line: number, col: number) => void;
    onopenstructure: () => void;
  }

  let { filePath, pathSegments, symbols, cursorLine, onjumpsymbol, onopenstructure }: Props = $props();
  let visible = $state(readIdeEditorPreferences().showBreadcrumbs);

  $effect(() => {
    if (typeof window === 'undefined') return;
    const onPrefs = () => { visible = readIdeEditorPreferences().showBreadcrumbs; };
    window.addEventListener(IDE_PREFS_EVENT, onPrefs);
    return () => window.removeEventListener(IDE_PREFS_EVENT, onPrefs);
  });

  const symbolChain = $derived.by((): IdeSymbolNode[] => {
    const chain: IdeSymbolNode[] = [];
    const walk = (nodes: IdeSymbolNode[]): void => {
      for (let i = 0; i < nodes.length; i++) {
        const s = nodes[i];
        if (cursorLine >= s.line && cursorLine <= Math.max(s.line, s.end_line)) {
          chain.push(s);
          if (s.children?.length) walk(s.children);
          break;
        }
      }
    };
    walk(symbols);
    return chain;
  });

  function openSiblingMenu(e: MouseEvent, levelIdx: number): void {
    const anchor = e.currentTarget as HTMLElement | null;
    const siblings = levelIdx === 0 ? symbols : (symbolChain[levelIdx - 1]?.children ?? symbols);
    if (!siblings.length) { onopenstructure(); return; }
    const activeSym = symbolChain[levelIdx];
    const items: MenuItem[] = siblings.slice(0, 50).map((s) => ({
      label: s.name,
      icon: activeSym?.line === s.line ? 'check' : 'function',
      hint: `L${s.line}`,
      action: () => onjumpsymbol(s.line, 1),
    }));
    items.push({ separator: true }, { label: 'File Structure…', icon: 'book', hint: '⌘F12', action: onopenstructure });
    ctxMenu.showAt(anchor, items, { align: 'start' });
  }

  function openLocalHistory(): void {
    flushAutoLocalHistorySnapshot(filePath);
    window.dispatchEvent(new CustomEvent('otto:ide-open-local-history', { detail: { path: filePath } }));
  }

  function openEditorSettings(): void {
    window.dispatchEvent(new CustomEvent('otto:ide-open-settings'));
  }
</script>

{#if visible}
  <div class="ide-breadcrumbs-bar" dir="ltr" data-testid="ide-breadcrumbs-bar">
    <div class="ide-bc-trail">
      {#each pathSegments.slice(-3) as seg, idx (`${seg}:${idx}`)}
        {#if idx > 0}<span class="ide-bc-sep" aria-hidden="true"><Icon name="chevronRight" size={12} noflip /></span>{/if}
        <button type="button" class="ide-bc-item" onclick={() => void revealInIdeProjectTree(filePath)} title="Reveal in project tree">{seg}</button>
      {/each}
      {#each symbolChain as sym, idx (`${sym.name}:${sym.line}`)}
        <span class="ide-bc-sep" aria-hidden="true"><Icon name="chevronRight" size={12} noflip /></span>
        <button type="button" class="ide-bc-item is-sym" onclick={(e) => openSiblingMenu(e, idx)} title="Jump to symbol in scope">
          <Icon name="function" size={12} /><span>{sym.name}</span>
        </button>
      {/each}
    </div>

    <div class="ide-bc-actions">
      <button
        type="button"
        class="icon-btn"
        aria-label="Local History"
        title="Show Local History"
        onclick={openLocalHistory}
        data-testid="ide-local-history-btn"
      >
        <Icon name="clock" size={13} />
      </button>
      <button
        type="button"
        class="icon-btn"
        aria-label="Editor Settings"
        title="Editor Settings & Color Scheme"
        onclick={openEditorSettings}
        data-testid="ide-editor-settings-btn"
      >
        <Icon name="gear" size={13} />
      </button>
    </div>
  </div>
{/if}

<style>
  .ide-breadcrumbs-bar {
    display: flex; align-items: center; justify-content: space-between; gap: var(--sp-3);
    block-size: 26px; flex-shrink: 0; padding-inline: 10px var(--sp-3);
    background: var(--surface); border-block-start: 1px solid var(--border); min-inline-size: 0; overflow: hidden;
  }
  .ide-bc-trail { display: flex; align-items: center; gap: var(--sp-1); min-inline-size: 0; overflow: hidden; }
  .ide-bc-sep { display: inline-flex; opacity: var(--disabled-opacity); flex-shrink: 0; }
  .ide-bc-item {
    display: inline-flex; align-items: center; gap: var(--sp-1); padding: 1px var(--sp-2);
    background: transparent; border: none; border-radius: var(--radius-s);
    color: var(--text-dim); font-size: var(--fs-xs); cursor: pointer;
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-inline-size: 200px;
  }
  .ide-bc-item:hover { background: var(--surface-2); color: var(--text); }
  .ide-bc-item.is-sym { color: var(--accent-text); font-family: var(--font-mono); }
  .ide-bc-actions { display: inline-flex; align-items: center; gap: var(--sp-1); flex-shrink: 0; }
</style>
