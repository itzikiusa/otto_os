<script lang="ts">
  // FilesPanel: multi-section file tree container (up to 4 sections).
  // Each section is an independent FileTree rooted at its own folder, with
  // folder navigation; it starts at the Side panel's folder (the session's
  // cwd, or the workspace folder picked in the panel).
  import { rightPanel } from '../../lib/stores/rightPanel.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { sessionFs } from '../../lib/api/fs';
  import FileTree from './FileTree.svelte';
  import Icon from '../../lib/components/Icon.svelte';

  // ── section state ─────────────────────────────────────────────────────────

  interface Section {
    id: number;
    /** undefined → the Side panel's folder */
    root: string | undefined;
  }

  let nextId = $state(1);

  let sections: Section[] = $state([{ id: nextId++, root: undefined }]);

  const MAX = 4;

  // Inside an agent session the tree is the session's cwd. Engine-made cwds
  // (workflow worktrees, persona folders, goal-loop / scheduled / swarm work
  // dirs) live in Otto's data dir, which `/fs/*` refuses non-root callers, so
  // the session-scoped routes back it up (PC-1, access-control-4-03). One
  // adapter per session, so the tree doesn't reload on every render.
  const sessionSource = $derived.by(() => {
    const s = ws.activeSession;
    return rightPanel.session && s?.cwd ? sessionFs(s.id, s.cwd) : null;
  });

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
      title={sections.length >= MAX ? `Up to ${MAX} file sections` : 'Add file section'}
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
          root={section.root ?? (rightPanel.folder || undefined)}
          primary={i === 0}
          navigation
          lister={sessionSource?.lister}
          reader={sessionSource?.reader}
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
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
    letter-spacing: .06em;
  }

  .fp-add-btn {
    display: flex;
    align-items: center;
    gap: 4px;
    /* .icon-btn is a fixed 24px square — this one carries a label too, so let
       it size to its content or "Add section" spills past the hover box. */
    width: auto;
    height: 22px;
    padding: 2px 6px;
    font-size: var(--fs-xs);
    border-radius: var(--radius-s);
    color: var(--text-dim);
    transition: color var(--dur-fast) ease-out, background var(--dur-fast) ease-out;
  }
  .fp-add-btn:hover:not(:disabled) {
    color: var(--text);
    background: var(--surface);
  }
  .fp-add-btn:disabled {
    opacity: var(--disabled-opacity);
    cursor: not-allowed;
  }

  .fp-add-label {
    font-size: var(--fs-xs);
    white-space: nowrap;
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
