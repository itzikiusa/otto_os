<script lang="ts">
  // GoLand-style Multi-Tab Bar (locked 34px height, CLS = 0): Back/Forward cursor history,
  // smart middle-ellipsis + parent-folder disambiguation, dirty dot / pin / read-only lock,
  // Local History / Git context items, overflow tabs picker, and Split Right / Down controls.
  // A WAI-ARIA tablist (roving tabindex, ←/→, Delete closes the focused tab); the active tab
  // is scrolled into view; a preview tab (italic) is kept open by a double-click.
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
  import { ideRevealDelta } from './ideCrumbs';
  import { ideLayout } from '../shell/ideLayout.svelte';
  import { ideDiskChange } from '../store/ideDiskChange.svelte';
  import { idePhoneChrome } from './idePhoneChrome.svelte';
  import IdePhoneTabRow from './IdePhoneTabRow.svelte';
  import IdePreviewModeToggle from './IdePreviewModeToggle.svelte';

  // Soft wrap + a narrow gutter whenever the IDE is stacked (phone / narrow window).
  followIdeCompactLayout();

  interface Props {
    onsave: (path: string) => void;
  }

  let { onsave }: Props = $props();
  let stripEl = $state<HTMLDivElement | null>(null);

  // Opening a file (or the 16th) must not leave its tab off-screen. Scroll only the strip,
  // never its overflow-hidden ancestors (scrollIntoView would nudge those too).
  $effect(() => {
    const path = ide.activePath;
    void ide.tabs.length;
    const strip = stripEl;
    if (!strip || !path) return;
    const raf = requestAnimationFrame(() => {
      const el = strip.querySelector<HTMLElement>('.ide-tab.active');
      if (!el) return;
      const r = el.getBoundingClientRect(), sr = strip.getBoundingClientRect();
      const d = ideRevealDelta(r.left, r.right, sr.left, sr.right);
      if (d !== 0) strip.scrollLeft += d;
    });
    return () => cancelAnimationFrame(raf);
  });

  /** Double-click keeps a preview tab open (VS Code); pinning stays in the tab's menu. */
  function keepTab(path: string): void {
    const t = ide.tabs.find((x) => x.path === path);
    if (t?.preview) t.preview = false;
  }

  function onTabKeydown(e: KeyboardEvent, tab: IdeOpenTab, pinned: boolean): void {
    if ((e.key === 'Delete' || e.key === 'Backspace') && !e.metaKey && !e.ctrlKey && !e.altKey) {
      e.preventDefault();
      if (pinned) return;
      void ide.closeTab(tab.path).then(() => {
        requestAnimationFrame(() => stripEl?.querySelector<HTMLElement>('.ide-tab.active .ide-tab-btn')?.focus());
      });
      return;
    }
    onTabKey(e);
  }

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

<!-- A phone gets one row: the active file + "N open" + the editor's ⋯ (IdePhoneTabRow);
     on its side that row lives in the page header, and typing may fold it away. -->
{#if ideLayout.stack}
  {#if idePhoneChrome.current.fileRow}<IdePhoneTabRow />{/if}
{:else}
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

  <div class="ide-tabs-strip" role="tablist" aria-label="Open files" bind:this={stripEl} data-testid="ide-tabs-strip"><!-- ui-guards: allow: open-file tabs (close ×, dirty dot, pin) are not <Tabs> page views; same keys via onTabKey -->
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
          role="tab"
          aria-selected={isActive}
          tabindex={isActive ? 0 : -1}
          onkeydown={(e) => onTabKeydown(e, tab, isPinned)}
          onclick={() => {
            if (ide.activePath && ide.activePath !== tab.path) {
              flushAutoLocalHistorySnapshot(ide.activePath);
              pushIdeNavLocation(ide.activePath, ide.cursorInfo.line, ide.cursorInfo.col);
            }
            ide.activePath = tab.path;
          }}
          ondblclick={() => keepTab(tab.path)}
          title={`${ide.relativePath(tab.path)}${tab.preview ? ' (preview — double-click to keep open)' : ''}${tab.readOnly ? ' (read-only)' : ''}${isDirty ? ' (unsaved)' : ''}`}
        >
          <Icon name={isPinned ? 'pin' : tab.readOnly ? 'lock' : 'file'} size={12} />
          <span class="ide-tab-label" dir="ltr">{displayLabel}</span>
          {#if isDirty}
            <span class="ide-dirty-dot" aria-hidden="true"></span>
          {/if}
          {#if ideDiskChange.has(tab.path)}
            <span class="ide-disk-mark" title="Changed on disk" data-testid="ide-tab-disk-changed"><Icon name="warning" size={12} /></span>
          {/if}
        </button>
        <button
          type="button"
          class="icon-btn ide-tab-close"
          tabindex={-1}
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
{/if}

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
  /* A solid sibling, never drawn over the last tab. */
  .ide-tabs-end {
    display: flex; align-items: center; gap: var(--sp-1); padding-inline: var(--sp-3); flex-shrink: 0;
    background: var(--surface); border-inline-start: 1px solid var(--border);
  }
  .ide-disk-mark { display: inline-flex; color: var(--warning); flex-shrink: 0; }
  /* A touch tablet keeps the workbench: finger-sized tabs and controls (the halo in
     app.css brings 24 px icons to the 36 px floor; tabs grow to 40). */
  @media (pointer: coarse) {
    .ide-tabs-row { block-size: 40px; }
    .ide-tab-close { min-inline-size: 32px; }
    .ide-nav-btns, .ide-tabs-end { gap: var(--sp-2); }
  }
</style>
