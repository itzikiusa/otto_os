<script lang="ts">
  // FilesPanel: multi-section file tree container (up to 4 sections).
  // Each section is an independent FileTree rooted at its own folder.
  import { ws } from '../../lib/stores/workspace.svelte';
  import FileTree from './FileTree.svelte';
  import Icon from '../../lib/components/Icon.svelte';

  // ── section state ─────────────────────────────────────────────────────────

  interface Section {
    id: number;
    /** undefined → FileTree falls back to ws default */
    root: string | undefined;
  }

  let nextId = $state(1);

  // Default root from workspace (read once at init; sections track their own root via FileTree).
  function defaultRoot(): string | undefined {
    const r = ws.activeSession?.cwd || ws.current?.root_path;
    return r || undefined;
  }

  let sections: Section[] = $state([{ id: nextId++, root: undefined }]);

  const MAX = 4;

  function addSection(): void {
    if (sections.length >= MAX) return;
    sections = [...sections, { id: nextId++, root: undefined }];
  }

  function removeSection(id: number): void {
    if (sections.length <= 1) return;
    sections = sections.filter((s) => s.id !== id);
  }
</script>

<div class="fp-wrap">
  <!-- Toolbar -->
  <div class="fp-toolbar">
    <span class="fp-count dim">{sections.length} / {MAX}</span>
    <button
      class="icon-btn fp-add-btn"
      disabled={sections.length >= MAX}
      title="Add file section"
      aria-label="Add file section"
      onclick={addSection}
    >
      <Icon name="plus" size={13} />
      <span class="fp-add-label">Add section</span>
    </button>
  </div>

  <!-- Sections -->
  <div class="fp-sections">
    {#each sections as section, i (section.id)}
      {#if i > 0}
        <div class="fp-divider"></div>
      {/if}
      <div class="fp-section">
        <FileTree
          root={section.root}
          primary={i === 0}
          onClose={sections.length > 1 ? () => removeSection(section.id) : undefined}
        />
      </div>
    {/each}
  </div>
</div>

<style>
  .fp-wrap {
    display: flex;
    flex-direction: column;
    height: 100%;
    overflow: hidden;
  }

  /* ── toolbar ──────────────────────────── */
  .fp-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 4px 8px;
    border-bottom: 1px solid var(--border);
    background: var(--surface-2);
    flex-shrink: 0;
  }

  .fp-count {
    font-size: 10px;
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.04em;
  }

  .fp-add-btn {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 2px 6px;
    font-size: 11px;
    border-radius: var(--radius-s);
    color: var(--text-dim);
    transition: color 120ms ease-out, background 120ms ease-out;
  }
  .fp-add-btn:hover:not(:disabled) {
    color: var(--text);
    background: var(--surface);
  }
  .fp-add-btn:disabled {
    opacity: 0.35;
    cursor: not-allowed;
  }

  .fp-add-label {
    font-size: 11px;
  }

  /* ── sections container ───────────────── */
  .fp-sections {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    min-height: 0;
  }

  .fp-section {
    /* Each section gets an equal share of the available height */
    flex: 1 1 0;
    min-height: 80px;
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }

  .fp-divider {
    flex-shrink: 0;
    height: 3px;
    background: var(--border);
  }
</style>
