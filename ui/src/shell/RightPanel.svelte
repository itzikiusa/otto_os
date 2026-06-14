<script lang="ts">
  // Collapsible right panel (⌘J): Git / Files / Notes / Info / Browser tabs ⇄ 36px icon strip.
  import Icon from '../lib/components/Icon.svelte';
  import EmptyState from '../lib/components/EmptyState.svelte';
  import GitPanel from '../modules/git/GitPanel.svelte';
  import InfoPanel from '../modules/panels/InfoPanel.svelte';
  import BrowserPanel from '../modules/panels/BrowserPanel.svelte';
  import FilesPanel from '../modules/panels/FilesPanel.svelte';
  import ApiPanel from '../modules/api/ApiPanel.svelte';
  import { ui, type RightTab } from '../lib/stores/ui.svelte';
  import { ws } from '../lib/stores/workspace.svelte';

  // Drag-to-resize: the panel is anchored right, so dragging the left edge
  // leftwards (smaller clientX) widens it.
  let resizing = $state(false);
  function startResize(e: MouseEvent): void {
    e.preventDefault();
    resizing = true;
    const startX = e.clientX;
    const startW = ui.rightWidth;
    const onMove = (ev: MouseEvent) => ui.setRightWidth(startW + (startX - ev.clientX));
    const onUp = () => {
      resizing = false;
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
      document.body.style.cursor = '';
      document.body.style.userSelect = '';
    };
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
    document.body.style.cursor = 'col-resize';
    document.body.style.userSelect = 'none';
  }

  const tabs: { id: RightTab; icon: string; label: string }[] = [
    { id: 'git', icon: 'branch', label: 'Git' },
    { id: 'files', icon: 'file', label: 'Files' },
    { id: 'notes', icon: 'note', label: 'Notes' },
    { id: 'info', icon: 'info', label: 'Info' },
    { id: 'browser', icon: 'globe', label: 'Browser' },
    { id: 'api', icon: 'send', label: 'API' },
  ];

  let notes = $state('');
  let notesLoadedFor: string | null = $state(null);
  let saveTimer: ReturnType<typeof setTimeout> | null = null;
  let saveState: 'idle' | 'saving' | 'saved' = $state('idle');

  $effect(() => {
    // (re)load notes when workspace changes
    const w = ws.current;
    if (w && notesLoadedFor !== w.id) {
      notesLoadedFor = w.id;
      notes = typeof w.settings?.notes === 'string' ? (w.settings.notes as string) : '';
      saveState = 'idle';
    }
  });

  function onNotesInput(): void {
    if (saveTimer) clearTimeout(saveTimer);
    saveState = 'saving';
    saveTimer = setTimeout(async () => {
      try {
        await ws.saveNotes(notes);
        saveState = 'saved';
        setTimeout(() => (saveState = 'idle'), 1500);
      } catch {
        saveState = 'idle';
      }
    }, 600);
  }
</script>

{#if ui.rightOpen}
  <aside class="rpanel" class:resizing style="width:{ui.rightWidth}px">
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="resize-handle"
      onmousedown={startResize}
      ondblclick={() => ui.setRightWidth(300)}
      title="Drag to resize · double-click to reset"
    ></div>
    <header class="rpanel-head">
      <div class="rpanel-tabs" role="tablist">
        {#each tabs as t (t.id)}
          <button
            class="rtab"
            class:active={ui.rightTab === t.id}
            role="tab"
            aria-selected={ui.rightTab === t.id}
            onclick={() => (ui.rightTab = t.id)}
          >
            {t.label}
          </button>
        {/each}
      </div>
      <button
        class="icon-btn"
        onclick={() => ui.toggleRight()}
        title="Collapse panel (⌘J)"
        aria-label="Collapse panel"
      >
        <Icon name="panel" size={13} />
      </button>
    </header>

    <div class="rpanel-body">
      {#if ui.rightTab === 'git'}
        <GitPanel />
      {:else if ui.rightTab === 'files'}
        <FilesPanel />
      {:else if ui.rightTab === 'info'}
        <InfoPanel />
      {:else if ui.rightTab === 'browser'}
        <BrowserPanel />
      {:else if ui.rightTab === 'api'}
        <ApiPanel />
      {:else}
        <div class="notes-wrap">
          <textarea
            class="notes"
            bind:value={notes}
            oninput={onNotesInput}
            placeholder="Workspace notes (markdown)…"
            spellcheck="false"
          ></textarea>
          <div class="notes-foot">
            {#if saveState === 'saving'}<span class="dim">saving…</span>
            {:else if saveState === 'saved'}<span class="dim">saved</span>
            {:else}<span class="dim">autosaves to workspace</span>{/if}
          </div>
        </div>
      {/if}
    </div>
  </aside>
{:else}
  <aside class="rstrip">
    {#each tabs as t (t.id)}
      <button
        class="icon-btn strip-btn"
        onclick={() => ui.openRight(t.id)}
        title="{t.label} (⌘J)"
        aria-label={t.label}
      >
        <Icon name={t.icon} size={15} />
      </button>
    {/each}
  </aside>
{/if}

<style>
  .rpanel {
    /* width is set inline from ui.rightWidth (drag-resizable) */
    height: 100%;
    display: flex;
    flex-direction: column;
    border-left: 1px solid var(--border);
    background: var(--bg);
    flex-shrink: 0;
    position: relative;
  }
  .rpanel.resizing {
    /* no transition while dragging for 1:1 tracking */
    user-select: none;
  }
  .resize-handle {
    position: absolute;
    left: -3px;
    top: 0;
    bottom: 0;
    width: 7px;
    cursor: col-resize;
    z-index: 5;
  }
  .resize-handle:hover,
  .rpanel.resizing .resize-handle {
    background: linear-gradient(
      to right,
      transparent 0,
      color-mix(in srgb, var(--accent) 40%, transparent) 45%,
      color-mix(in srgb, var(--accent) 40%, transparent) 55%,
      transparent 100%
    );
  }
  .rpanel-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 7px 8px 6px;
    border-bottom: 1px solid var(--border);
  }
  .rpanel-tabs {
    display: flex;
    gap: 2px;
  }
  .rtab {
    height: 24px;
    padding: 0 10px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    transition: background 120ms ease-out, color 120ms ease-out;
  }
  .rtab:hover {
    background: var(--surface-2);
  }
  .rtab.active {
    background: var(--surface-2);
    color: var(--text);
  }
  .rpanel-body {
    flex: 1;
    overflow-y: auto;
    min-height: 0;
  }
  .rstrip {
    width: 36px;
    height: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    padding-top: 10px;
    border-left: 1px solid var(--border);
    background: var(--bg);
    flex-shrink: 0;
  }
  .strip-btn {
    width: 28px;
    height: 28px;
  }
  .notes-wrap {
    display: flex;
    flex-direction: column;
    height: 100%;
  }
  .notes {
    flex: 1;
    border: none;
    resize: none;
    background: transparent;
    padding: 12px;
    font-family: var(--font-mono);
    font-size: 12px;
    line-height: 1.6;
    color: var(--text);
    outline: none;
  }
  .notes-foot {
    padding: 4px 12px 8px;
    font-size: 10.5px;
  }
</style>
