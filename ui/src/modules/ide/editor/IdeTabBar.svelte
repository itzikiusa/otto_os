<script lang="ts">
  // GoLand-style Multi-Tab Bar (locked 34px height, CLS = 0): Back/Forward cursor history,
  // smart middle-ellipsis + parent-folder disambiguation, dirty dot / pin / read-only lock,
  // Local History / Git context items, overflow tabs picker, and Split Right / Down controls.
  import Icon from '../../../lib/components/Icon.svelte';
  import { rowMenu } from '../../../lib/rowMenu';
  import { onTabKey } from '../../../lib/tabKeys';
  import { ide, type IdeOpenTab } from '../ideStore.svelte';
  import {
    canIdeNavigateBack,
    canIdeNavigateForward,
    flushAutoLocalHistorySnapshot,
    formatIdeTabDisplayName,
    isIdeTabPinned,
    navigateIdeBack,
    navigateIdeForward,
    pushIdeNavLocation,
    toggleIdeTabPinned,
  } from './ideEditorStoreBridge.svelte';
  import { followIdeCompactLayout } from './ideEditorChrome.svelte';
  import IdePreviewModeToggle from './IdePreviewModeToggle.svelte';

  // Soft wrap + a narrow gutter whenever the IDE is stacked (phone / narrow window).
  followIdeCompactLayout();

  interface Props {
    onsave: (path: string) => void;
  }

  let { onsave }: Props = $props();

  // A tab activated from outside the strip (Find file, ⌘E, a jump) scrolls into view.
  let stripEl = $state<HTMLDivElement | undefined>(undefined);
  $effect(() => {
    if (!ide.activePath || !stripEl) return;
    void ide.tabs.length;
    stripEl.querySelector('.ide-tab.active')?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  });

  function openTabMenu(e: MouseEvent, tab: IdeOpenTab): void {
    e.preventDefault();
    const el = e.currentTarget instanceof HTMLElement ? e.currentTarget : null;
    const anchor =
      el && (e.detail === 0 || e.type === 'click')
        ? el
        : new MouseEvent('contextmenu', { clientX: e.clientX, clientY: e.clientY });
    void import('./ideEditorActionsLazy').then((m) => m.openIdeTabContextMenu(anchor, tab, onsave));
  }

  function openOverflowTabsMenu(e: MouseEvent): void {
    const anchor = e.currentTarget as HTMLElement | null;
    void import('./ideEditorActionsLazy').then((m) => m.openIdeOverflowTabsMenu(anchor));
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

  <div class="ide-tabs-strip" role="toolbar" aria-label="Open files" data-testid="ide-tabs-strip" bind:this={stripEl}>
    {#each ide.tabs as tab (tab.path)}
      {@const isDirty = tab.dirty ?? (tab.content !== tab.savedContent)}
      {@const isActive = tab.path === ide.activePath}
      {@const isPinned = isIdeTabPinned(tab)}
      {@const displayLabel = formatIdeTabDisplayName(tab, ide.tabs)}
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
              flushAutoLocalHistorySnapshot(ide.activePath);
              pushIdeNavLocation(ide.activePath, ide.cursorInfo.line, ide.cursorInfo.col);
            }
            ide.activePath = tab.path;
          }}
          ondblclick={() => ide.pinTab(tab.path)}
          title={`${ide.relativePath(tab.path)}${tab.readOnly ? ' (read-only)' : ''}${isDirty ? ' (unsaved)' : ''}`}
        >
          <Icon name={isPinned ? 'pin' : tab.readOnly ? 'lock' : 'file'} size={12} />
          <span class="ide-tab-label" dir="ltr">{displayLabel}</span>
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
    <IdePreviewModeToggle />
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
  .ide-tabs-row {
    display: flex; align-items: stretch; block-size: var(--ide-tab-h, 34px); flex-shrink: 0;
    background: var(--surface); border-block-end: 1px solid var(--border); min-inline-size: 0; overflow: hidden;
  }
  .ide-nav-btns { display: inline-flex; align-items: center; gap: var(--sp-1); padding-inline: var(--sp-2); border-inline-end: 1px solid var(--border); flex-shrink: 0; }
  .ide-tabs-strip { flex: 1; min-inline-size: 0; display: flex; align-items: stretch; overflow-x: auto; scrollbar-width: none; }
  .ide-tabs-strip::-webkit-scrollbar { display: none; }
  .ide-tab {
    display: inline-flex; align-items: center; padding-inline-end: var(--sp-2);
    border-inline-end: 1px solid var(--border); background: var(--surface); flex-shrink: 0; max-inline-size: 240px;
  }
  .ide-tab.active { background: var(--bg); box-shadow: inset 0 -2px 0 var(--accent-solid); }
  .ide-tab.preview .ide-tab-label { font-style: italic; }
  .ide-tab-btn {
    display: inline-flex; align-items: center; gap: var(--sp-2); block-size: 100%; padding-inline: 10px var(--sp-2);
    background: transparent; border: none; color: var(--text-dim); font-size: var(--fs-xs); cursor: pointer; min-inline-size: 0;
  }
  .ide-tab.active .ide-tab-btn { color: var(--text); font-weight: 500; }
  .ide-tab-label { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ide-dirty-dot { inline-size: 6px; block-size: 6px; border-radius: 50%; background: var(--accent-solid); flex-shrink: 0; }
  .ide-tab-close { opacity: var(--disabled-opacity); }
  .ide-tab:hover .ide-tab-close, .ide-tab.active .ide-tab-close { opacity: 1; }
  .ide-tabs-end { display: flex; align-items: center; gap: var(--sp-1); padding-inline: var(--sp-3); flex-shrink: 0; }
</style>
