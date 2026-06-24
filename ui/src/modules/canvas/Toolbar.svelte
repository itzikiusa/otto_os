<script lang="ts">
  // Canvas top bar: scene title (inline edit), an autosave indicator, undo/redo,
  // and the hero actions — Ask AI (accent), Present, Export JSON, Zoom-fit. The
  // left vertical tool rail is a sibling (ToolRail); this is only the top strip.
  import Icon from '../../lib/components/Icon.svelte';
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
      <!-- svelte-ignore a11y_autofocus -->
      <input
        class="title-input"
        bind:value={titleDraft}
        autofocus
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
      <button class="icon-btn" title="Undo (⌘Z)" disabled={!canvas.canUndo} onclick={() => canvas.undo()}>
        <Icon name="arrowUp" />
      </button>
      <button class="icon-btn" title="Redo (⌘⇧Z)" disabled={!canvas.canRedo} onclick={() => canvas.redo()}>
        <Icon name="arrowDown" />
      </button>
      <span class="sep"></span>
    {/if}
    <button class="icon-btn" title="Zoom to fit" onclick={onfit}><Icon name="maximize" /></button>
    <button class="icon-btn" title="Export JSON" onclick={exportJson}><Icon name="file" /></button>
    <button class="btn" title="Present" onclick={onpresent}>
      <Icon name="play" /> Present
    </button>
    {#if !readonly}
      <button class="btn accent" title="Generate with AI (⌘↵)" onclick={onaskai}>
        <Icon name="zap" /> Ask AI
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
    font-size: 14px;
    color: var(--text);
    background: none;
    border: none;
    padding: 4px 6px;
    border-radius: var(--radius-s, 6px);
    cursor: text;
    max-width: 40vw;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .title:hover:not(:disabled) {
    background: var(--surface-2);
  }
  .title-input {
    font-size: 14px;
    font-weight: 600;
    padding: 4px 6px;
    border: 1px solid var(--accent);
    border-radius: var(--radius-s, 6px);
    background: var(--bg);
    color: var(--text);
    min-width: 200px;
  }
  .saved {
    font-size: 11px;
    color: var(--text-dim, #888);
    white-space: nowrap;
  }
  .saved.active {
    color: var(--accent);
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
  .icon-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 30px;
    height: 30px;
    border: none;
    background: none;
    color: var(--text);
    border-radius: var(--radius-s, 6px);
    cursor: pointer;
  }
  .icon-btn:hover:not(:disabled) {
    background: var(--surface-2);
  }
  .icon-btn:disabled {
    opacity: 0.35;
    cursor: default;
  }
  .btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 5px 10px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
    border-radius: var(--radius-s, 6px);
    font-size: 13px;
    cursor: pointer;
  }
  .btn:hover {
    background: var(--surface-2);
  }
  .btn.accent {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }
  .btn.accent:hover {
    filter: brightness(1.08);
  }
</style>
