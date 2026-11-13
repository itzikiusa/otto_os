<script lang="ts">
  // GoLand-style Multi-Tab Bar: Back/Forward cursor history, persistent file tabs
  // with dirty dot / pin / read-only lock / middle-click close / right-click context menu,
  // overflow tabs picker, and Split Right / Split Down / Close Split controls.
  import { copyWithToast } from '../../../lib/clipboard';
  import Icon from '../../../lib/components/Icon.svelte';
  import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
  import { rowMenu } from '../../../lib/rowMenu';
  import { onTabKey } from '../../../lib/tabKeys';
  import { ide, type IdeOpenTab } from '../ideStore.svelte';
  import { ideAgent } from '../store/ideAgentController.svelte';
  import {
    canIdeNavigateBack,
    canIdeNavigateForward,
    closeAllIdeTabs,
    closeUnmodifiedIdeTabs,
    isIdeTabPinned,
    navigateIdeBack,
    navigateIdeForward,
    openIdeTerminalForPath,
    pushIdeNavLocation,
    revealInIdeProjectTree,
    toggleIdeTabPinned,
  } from './ideEditorStoreBridge.svelte';

  interface Props {
    onsave: (path: string) => void;
  }

  let { onsave }: Props = $props();

  function openTabMenu(e: MouseEvent, tab: IdeOpenTab): void {
    const tabDirty = tab.dirty ?? (tab.content !== tab.savedContent);
    const pinned = isIdeTabPinned(tab);
    const items: MenuItem[] = [
      {
        label: 'Save',
        icon: 'download',
        hint: '⌘S',
        disabled: tab.readOnly || !tabDirty,
        action: () => onsave(tab.path),
      },
      {
        label: pinned ? 'Unpin tab' : 'Pin tab',
        icon: 'pin',
        action: () => toggleIdeTabPinned(tab.path),
      },
      { separator: true },
      { label: 'Split right', icon: 'split', action: () => ide.openInSplit(tab.path, 'right') },
      { label: 'Split down', icon: 'unified', action: () => ide.openInSplit(tab.path, 'down') },
      { separator: true },
      { label: 'Close', icon: 'x', disabled: pinned, action: () => void ide.closeTab(tab.path) },
      {
        label: 'Close others',
        disabled: ide.tabs.length <= 1,
        action: () => ide.closeOtherTabs(tab.path),
      },
      { label: 'Close unmodified', action: () => void closeUnmodifiedIdeTabs() },
      { label: 'Close all', action: () => void closeAllIdeTabs() },
      { separator: true },
      { label: 'Copy path', icon: 'copy', action: () => void copyWithToast(tab.path, 'file path') },
      {
        label: 'Copy relative path',
        icon: 'copy',
        action: () => void copyWithToast(ide.relativePath(tab.path), 'relative path'),
      },
      { label: 'Reveal in project tree', icon: 'folder', action: () => void revealInIdeProjectTree(tab.path) },
      { label: 'Open in terminal', icon: 'terminal', action: () => void openIdeTerminalForPath(tab.path) },
      { label: 'Add to Agent context', icon: 'assistant', action: () => void ideAgent.attachFile(tab.path, { openPanel: true }) },
    ];
    ctxMenu.show(e, items);
  }

  function openOverflowTabsMenu(e: MouseEvent): void {
    const anchor = e.currentTarget as HTMLElement | null;
    const items: MenuItem[] = ide.tabs.map((t) => {
      const dirty = t.dirty ?? (t.content !== t.savedContent);
      return {
        label: `${t.name}${dirty ? ' •' : ''}`,
        icon: t.path === ide.activePath ? 'check' : isIdeTabPinned(t) ? 'pin' : t.readOnly ? 'lock' : 'file',
        hint: ide.relativePath(t.path),
        action: () => { ide.activePath = t.path; },
      };
    });
    items.push(
      { separator: true },
      { label: 'Close unmodified tabs', action: () => void closeUnmodifiedIdeTabs() },
      { label: 'Close all tabs', icon: 'x', action: () => void closeAllIdeTabs() },
    );
    ctxMenu.showAt(anchor, items, { align: 'end' });
  }
</script>

<div class="ide-tabs-row">
  <div class="ide-nav-btns">
    <button
      type="button"
      class="icon-btn"
      aria-label="Navigate back"
      title="Back (⌘[)"
      disabled={!canIdeNavigateBack()}
      onclick={() => void navigateIdeBack()}
      data-testid="ide-nav-back"
    >
      <Icon name="chevronLeft" size={13} />
    </button>
    <button
      type="button"
      class="icon-btn"
      aria-label="Navigate forward"
      title="Forward (⌘])"
      disabled={!canIdeNavigateForward()}
      onclick={() => void navigateIdeForward()}
      data-testid="ide-nav-forward"
    >
      <Icon name="chevronRight" size={13} />
    </button>
  </div>

  <div class="ide-tabs-strip" role="toolbar" aria-label="Open files" data-testid="ide-tabs-strip">
    {#each ide.tabs as tab (tab.path)}
      {@const isDirty = tab.dirty ?? (tab.content !== tab.savedContent)}
      {@const isActive = tab.path === ide.activePath}
      {@const isPinned = isIdeTabPinned(tab)}
      <div
        class="ide-tab"
        class:active={isActive}
        class:preview={tab.preview}
        oncontextmenu={(e) => openTabMenu(e, tab)}
        onauxclick={(e) => {
          if (e.button === 1 && !isPinned) {
            e.preventDefault();
            void ide.closeTab(tab.path);
          }
        }}
        use:rowMenu
        role="presentation"
        data-testid="ide-tab-item"
      >
        <button
          type="button"
          class="ide-tab-btn"
          aria-pressed={isActive}
          onkeydown={onTabKey}
          onclick={() => {
            if (ide.activePath && ide.activePath !== tab.path) {
              pushIdeNavLocation(ide.activePath, ide.cursorInfo.line, ide.cursorInfo.col);
            }
            ide.activePath = tab.path;
          }}
          ondblclick={() => ide.pinTab(tab.path)}
          title={`${ide.relativePath(tab.path)}${tab.readOnly ? ' (read-only)' : ''}${isDirty ? ' (unsaved)' : ''}`}
        >
          <Icon name={isPinned ? 'pin' : tab.readOnly ? 'lock' : 'file'} size={12} />
          <span class="ide-tab-label" dir="ltr">{tab.name}</span>
          {#if isDirty}
            <span class="ide-dirty-dot" aria-hidden="true"></span>
          {/if}
        </button>
        <button
          type="button"
          class="icon-btn ide-tab-close"
          aria-label={isPinned ? `Unpin ${tab.name}` : `Close ${tab.name}`}
          title={isPinned ? 'Unpin tab' : 'Close tab'}
          onclick={() => (isPinned ? toggleIdeTabPinned(tab.path) : void ide.closeTab(tab.path))}
        >
          <Icon name={isPinned ? 'pin' : 'x'} size={12} />
        </button>
      </div>
    {/each}
  </div>

  <div class="ide-tabs-end">
    <button
      type="button"
      class="icon-btn"
      aria-label="All open tabs ({ide.tabs.length})"
      title="All open tabs ({ide.tabs.length})"
      onclick={openOverflowTabsMenu}
      data-testid="ide-tabs-overflow-btn"
    >
      <Icon name="chevronDown" size={13} />
    </button>
    {#if ide.activePath}
      <button
        type="button"
        class="icon-btn"
        aria-label="Split editor right"
        title="Split editor right"
        onclick={() => ide.activePath && ide.openInSplit(ide.activePath, 'right')}
        data-testid="ide-split-right-btn"
      >
        <Icon name="split" size={13} />
      </button>
      <button
        type="button"
        class="icon-btn"
        aria-label="Split editor down"
        title="Split editor down"
        onclick={() => ide.activePath && ide.openInSplit(ide.activePath, 'down')}
        data-testid="ide-split-down-btn"
      >
        <Icon name="unified" size={13} />
      </button>
    {/if}
    {#if ide.splitPath}
      <button
        type="button"
        class="icon-btn"
        aria-label="Close split editor"
        title="Close split editor"
        onclick={() => ide.closeSplit()}
      >
        <Icon name="minimize" size={13} />
      </button>
    {/if}
  </div>
</div>

<style>
  .ide-tabs-row { display: flex; align-items: center; background: var(--surface); border-block-end: 1px solid var(--border); min-inline-size: 0; }
  .ide-nav-btns { display: inline-flex; align-items: center; gap: var(--sp-1); padding-inline: var(--sp-2); border-inline-end: 1px solid var(--border); flex-shrink: 0; }
  .ide-tabs-strip { flex: 1; min-inline-size: 0; display: flex; align-items: stretch; overflow-x: auto; scrollbar-width: none; }
  .ide-tabs-strip::-webkit-scrollbar { display: none; }
  .ide-tab {
    display: inline-flex; align-items: center; padding-inline-end: var(--sp-2);
    border-inline-end: 1px solid var(--border); background: var(--surface); flex-shrink: 0; max-inline-size: 220px;
  }
  .ide-tab.active { background: var(--bg); box-shadow: inset 0 -2px 0 var(--accent-solid); }
  .ide-tab.preview .ide-tab-label { font-style: italic; }
  .ide-tab-btn {
    display: inline-flex; align-items: center; gap: var(--sp-3); padding-block: var(--sp-3); padding-inline: 10px var(--sp-3);
    background: transparent; border: none; color: var(--text-dim); font-size: var(--fs-xs); cursor: pointer; min-inline-size: 0;
  }
  .ide-tab.active .ide-tab-btn { color: var(--text); font-weight: 500; }
  .ide-tab-label { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ide-dirty-dot { inline-size: 6px; block-size: 6px; border-radius: 50%; background: var(--accent-solid); flex-shrink: 0; }
  .ide-tab-close { opacity: var(--disabled-opacity); }
  .ide-tab:hover .ide-tab-close, .ide-tab.active .ide-tab-close { opacity: 1; }
  .ide-tabs-end { display: flex; align-items: center; gap: var(--sp-1); padding-inline: var(--sp-3); flex-shrink: 0; }
</style>
