<script lang="ts">
  // The open-files row on a phone (stacked layout): Back, the active file — a tap lists
  // every open file ("N open"), with Editor | Preview and the close actions — and the
  // editor's command menu (⋯: Go to definition, Find usages, docs, Rename, Find in file,
  // Go to line, Text size…). No split buttons: a phone never shows two editors. On a
  // phone on its side the same row sits inside the page header (`inHeader`).
  import Icon from '../../../lib/components/Icon.svelte';
  import { ide } from '../ideStore.svelte';
  import { ideDiskChange } from '../store/ideDiskChange.svelte';
  import { canIdeNavigateBack, formatIdeTabDisplayName, navigateIdeBack } from './ideEditorStoreBridge.svelte';
  import { openIdeEditorTouchMenu } from './ideEditorTouchMenu';
  import IdePreviewModeToggle from './IdePreviewModeToggle.svelte';

  interface Props {
    inHeader?: boolean;
  }
  let { inHeader = false }: Props = $props();

  const tab = $derived(ide.activeTab);
  const label = $derived(tab ? formatIdeTabDisplayName(tab, ide.tabs) : '');
  const dirty = $derived(Boolean(tab && (tab.dirty ?? tab.content !== tab.savedContent)));
  const onDisk = $derived(Boolean(tab && ideDiskChange.has(tab.path)));
  const n = $derived(ide.tabs.length);

  function openTabs(e: MouseEvent): void {
    const anchor = e.currentTarget as HTMLElement | null;
    void import('./ideEditorActionsLazy').then((m) => m.openIdeOverflowTabsMenu(anchor));
  }
</script>

{#if tab}
  <div class="ide-phone-row" class:in-header={inHeader} data-testid="ide-phone-file-row">
    <button
      type="button"
      class="icon-btn ide-phone-btn"
      aria-label="Navigate back"
      title="Back"
      disabled={!canIdeNavigateBack()}
      onclick={() => void navigateIdeBack()}
      data-testid="ide-nav-back"
    >
      <Icon name="chevronLeft" size={16} />
    </button>
    <div class="ide-tabs-strip ide-phone-strip" role="group" aria-label="Open files" data-testid="ide-tabs-strip">
      <div class="ide-tab active">
        <button
          type="button"
          class="ide-phone-file"
          aria-label={`${label}${dirty ? ', unsaved' : ''}${onDisk ? ', changed on disk' : ''} — ${n} open ${n === 1 ? 'file' : 'files'}`}
          title={`${ide.relativePath(tab.path)} — ${n} open`}
          onclick={openTabs}
          data-testid="ide-tabs-overflow-btn"
        >
          <Icon name={tab.readOnly ? 'lock' : 'file'} size={13} />
          <span class="ide-tab-label" dir="ltr">{label}</span>
          {#if dirty}<span class="ide-dirty-dot" aria-hidden="true"></span>{/if}
          {#if onDisk}<span class="ide-disk-mark" aria-hidden="true" data-testid="ide-tab-disk-changed"><Icon name="warning" size={12} /></span>{/if}
          {#if n > 1}<span class="ide-phone-count" aria-hidden="true">{n}</span>{/if}
          <Icon name="chevronDown" size={12} />
        </button>
      </div>
    </div>
    <IdePreviewModeToggle />
    <button
      type="button"
      class="icon-btn ide-phone-btn ide-phone-more"
      aria-label="Editor commands"
      title="Editor commands"
      onclick={(e) => void openIdeEditorTouchMenu(e.currentTarget)}
      data-testid="ide-editor-touch-more"
    >
      <Icon name="more" size={16} />
    </button>
  </div>
{/if}

<style>
  .ide-phone-row {
    display: flex; align-items: center; gap: var(--sp-1); block-size: 40px; flex-shrink: 0; min-inline-size: 0;
    padding-inline: var(--sp-1); background: var(--surface); border-block-end: 1px solid var(--border);
  }
  .ide-phone-row.in-header { block-size: auto; background: transparent; border: none; padding: 0; flex: 1 1 auto; }
  .ide-phone-btn { inline-size: 40px; block-size: 40px; flex-shrink: 0; }
  .ide-phone-strip { flex: 1 1 auto; min-inline-size: 0; display: flex; }
  .ide-phone-strip .ide-tab { display: flex; min-inline-size: 0; max-inline-size: 100%; }
  .ide-phone-file {
    display: inline-flex; align-items: center; gap: var(--sp-2); min-block-size: 40px; min-inline-size: 0; max-inline-size: 100%;
    padding-inline: var(--sp-3); background: transparent; border: none; border-radius: var(--radius-s);
    color: var(--text); font-size: var(--fs-s); font-weight: 500; cursor: pointer;
  }
  .ide-phone-file:hover { background: var(--surface-2); }
  .ide-tab-label { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-inline-size: 0; }
  /* In the header keep the file name legible while letting it truncate before overflowing landscape headers. */
  .in-header .ide-tab-label { flex: 0 1 auto; min-inline-size: 56px; }
  .ide-dirty-dot { inline-size: 6px; block-size: 6px; border-radius: 50%; background: var(--accent-solid); flex-shrink: 0; }
  .ide-disk-mark { display: inline-flex; color: var(--warning); flex-shrink: 0; }
  .ide-phone-count {
    flex-shrink: 0; min-inline-size: 18px; padding-inline: 4px; border-radius: 999px; background: var(--surface-2);
    color: var(--text-dim); font-size: var(--fs-xs); text-align: center; line-height: 18px;
  }
</style>
