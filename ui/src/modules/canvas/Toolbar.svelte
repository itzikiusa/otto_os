<script lang="ts">
  // Canvas top bar: scene title (inline edit), an autosave indicator, undo/redo,
  // and the hero actions — Ask Otto (accent), Present, Export JSON, Zoom-fit. The
  // left vertical tool rail is a sibling (ToolRail); this is only the top strip.
  import Icon from '../../lib/components/Icon.svelte';
  import { focusOnMount } from '../../lib/focusOnMount';
  import { canvas } from '../../lib/stores/canvas.svelte';

  interface Props {
    onaskai: () => void;
    onpresent: () => void;
    onfit: () => void;
    readonly?: boolean;
  }
  let { onaskai, onpresent, onfit, readonly = false }: Props = $props();

  let editingTitle = $state(false);
  let titleDraft = $state('');

  function startEditTitle(): void {
    if (readonly) return;
    titleDraft = canvas.scene?.title ?? '';
    editingTitle = true;
  }
  function commitTitle(): void {
    const t = titleDraft.trim();
    if (t && t !== canvas.scene?.title) canvas.rename(t);
    editingTitle = false;
  }

  // "Saved · 14:22" relative-free clock, kept tiny.
  const savedLabel = $derived.by((): string => {
    if (canvas.saving) return 'Saving…';
    if (canvas.dirty) return 'Unsaved';
    if (canvas.savedAt) {
      const d = new Date(canvas.savedAt);
      const hh = String(d.getHours()).padStart(2, '0');
      const mm = String(d.getMinutes()).padStart(2, '0');
      return `Saved · ${hh}:${mm}`;
    }
    return '';
  });

  function exportJson(): void {
    if (!canvas.scene) return;
    const blob = new Blob([JSON.stringify(canvas.scene, null, 2)], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `${(canvas.scene.title || 'scene').replace(/[^a-z0-9-_]+/gi, '-')}.json`;
    a.click();
    URL.revokeObjectURL(url);
  }
</script>

<div class="toolbar">
  <div class="left">
    {#if editingTitle}
      <input dir="auto" aria-label="Scene title"
        class="title-input"
        bind:value={titleDraft}
        use:focusOnMount
        onblur={commitTitle}
        onkeydown={(e) => {
          if (e.key === 'Enter') commitTitle();
          else if (e.key === 'Escape') editingTitle = false;
        }}
      />
    {:else}
      <button class="title" onclick={startEditTitle} title="Rename scene" disabled={readonly}>
        {canvas.scene?.title || 'Untitled scene'}
      </button>
    {/if}
    <span class="saved" class:active={canvas.saving}>{savedLabel}</span>
  </div>

  <div class="right">
    {#if !readonly}
      <button class="icon-btn" title="Undo (⌘Z)" aria-label="Undo (⌘Z)" disabled={!canvas.canUndo} onclick={() => canvas.undo()}>
        <Icon name="arrowUp" />
      </button>
      <button class="icon-btn" title="Redo (⌘⇧Z)" aria-label="Redo (⌘⇧Z)" disabled={!canvas.canRedo} onclick={() => canvas.redo()}>
        <Icon name="arrowDown" />
      </button>
      <span class="sep"></span>
    {/if}
    <button class="icon-btn" title="Zoom to fit" aria-label="Zoom to fit" onclick={onfit}><Icon name="maximize" /></button>
    <button class="icon-btn" title="Export JSON" aria-label="Export JSON" onclick={exportJson}><Icon name="file" /></button>
    <button class="btn" title="Present" onclick={onpresent}>
      <Icon name="play" /> Present
    </button>
    {#if !readonly}
      <button class="btn primary" title="Ask Otto (⌘↵)" onclick={onaskai}>
        <Icon name="zap" /> Ask Otto
      </button>
    {/if}
  </div>
</div>

<style>
  .toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
    min-height: 42px;
  }
  .left {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }
  .title {
    font-weight: 600;
    font-size: var(--fs-m);
    color: var(--text);
    background: none;
    border: none;
    padding: 4px 6px;
    border-radius: var(--radius-s);
    cursor: text;
    max-width: 40vw;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .title:hover:not(:disabled) {
    background: var(--hover);
  }
  .title-input {
    font-size: var(--fs-m);
    font-weight: 600;
    padding: 4px 6px;
    border: 1px solid var(--accent);
    border-radius: var(--radius-s);
    background: var(--bg);
    color: var(--text);
    min-width: 200px;
  }
  .saved {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
  }
  .saved.active {
    color: var(--accent-text);
  }
  .right {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .sep {
    width: 1px;
    height: 20px;
    background: var(--border);
    margin: 0 4px;
  }
</style>
