<script lang="ts">
  // Scene list — the workspace's canvas scenes grouped into collapsible SECTIONS
  // (a folder path like "Platform/Staging" → sections + sub-sections), with
  // search, New, click-to-open, inline RENAME, MOVE-to-section, and delete.
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { canvas } from '../../lib/stores/canvas.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { api } from '../../lib/api/client';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import type { CanvasSceneSummary, CanvasScene } from './types';


  let filter = $state('');
  let collapsed = $state<Record<string, boolean>>({});


  const rows = $derived.by((): CanvasSceneSummary[] => {
    const q = filter.trim().toLowerCase();
    const list = q
      ? canvas.scenes.filter(
          (s) => s.title.toLowerCase().includes(q) || (s.section ?? '').toLowerCase().includes(q),
        )
      : canvas.scenes;
    return [...list].sort((a, b) => b.updated_at.localeCompare(a.updated_at));
  });

  // Group by section path. Root (no section) first, then sections alphabetically.
  const groups = $derived.by((): [string, CanvasSceneSummary[]][] => {
    const map = new Map<string, CanvasSceneSummary[]>();
    for (const s of rows) {
      const key = (s.section ?? '').trim();
      if (!map.has(key)) map.set(key, []);
      map.get(key)!.push(s);
    }
    return [...map.entries()].sort((a, b) => {
      if (a[0] === '') return -1;
      if (b[0] === '') return 1;
      return a[0].localeCompare(b[0]);
    });
  });

  /** One ⋯ / right-click menu per row instead of four always-on icons
   *  (the shared clamped ctxMenu). */
  function rowMenu(e: MouseEvent | KeyboardEvent, s: CanvasSceneSummary): void {
    e.preventDefault();
    e.stopPropagation();
    ctxMenu.show(e, [
      { label: 'Rename…', icon: 'edit', action: () => void rename(null, s) },
      { label: 'Move to section…', icon: 'folder', action: () => void move(null, s) },
      { label: 'Duplicate', icon: 'copy', action: () => void duplicate(null, s) },
      { separator: true },
      { label: 'Delete…', icon: 'trash', danger: true, action: () => void remove(null, s) },
    ]);
  }

  async function rename(e: MouseEvent | null, s: CanvasSceneSummary): Promise<void> {
    e?.stopPropagation();
    const t = await confirmer.promptText('', {
      title: 'Rename scene',
      initial: s.title,
      confirmLabel: 'Rename',
    });
    if (t == null || t.trim() === '' || t === s.title) return;
    try {
      await canvas.updateMeta(s.id, { title: t.trim() });
    } catch (err) {
      toasts.error('Rename failed', err instanceof Error ? err.message : String(err));
    }
  }

  async function move(e: MouseEvent | null, s: CanvasSceneSummary): Promise<void> {
    e?.stopPropagation();
    const sec = await confirmer.promptText('Sections group scenes in the list. Use / to nest them.', {
      title: 'Move to section',
      initial: s.section ?? '',
      placeholder: 'e.g. Platform/Staging — empty = no section',
      confirmLabel: 'Move',
    });
    if (sec == null) return;
    try {
      await canvas.updateMeta(s.id, { section: sec.trim() || null });
    } catch (err) {
      toasts.error('Move failed', err instanceof Error ? err.message : String(err));
    }
  }

  /** Clone a scene's full doc (fetched fresh — the summary row has no `doc_json`)
   *  into a new scene titled "<title> (copy)", in the SAME section and workspace
   *  as the original (Canvas lists scenes across all workspaces, so this can
   *  differ from the currently-active one). */
  async function duplicate(e: MouseEvent | null, s: CanvasSceneSummary): Promise<void> {
    e?.stopPropagation();
    try {
      const row = await api.get<CanvasScene>(`/canvas/scenes/${s.id}`);
      let doc: unknown;
      try {
        doc = JSON.parse(row.doc_json);
      } catch {
        doc = undefined;
      }
      await api.post(`/workspaces/${row.workspace_id}/canvas/scenes`, {
        title: `${row.title} (copy)`,
        doc,
        section: row.section,
      });
      await canvas.loadScenes().catch(() => {});
      toasts.success('Scene duplicated', `${row.title} (copy)`);
    } catch (err) {
      toasts.error('Duplicate failed', err instanceof Error ? err.message : String(err));
    }
  }

  async function remove(e: MouseEvent | null, s: CanvasSceneSummary): Promise<void> {
    e?.stopPropagation();
    const ok = await confirmer.ask(`“${s.title}” and its drawing are deleted for good. This can’t be undone.`, {
      title: 'Delete scene',
      danger: true,
      confirmLabel: 'Delete scene',
    });
    if (!ok) return;
    try {
      await canvas.del(s.id);
      toasts.success('Scene deleted', s.title);
    } catch (err) {
      toasts.error('Delete failed', err instanceof Error ? err.message : String(err));
    }
  }
</script>

<div class="scene-list">
  <div class="search">
    <Icon name="search" size={13} />
    <input type="search" placeholder="Search scenes…" aria-label="Search scenes" bind:value={filter} spellcheck="false" />
    {#if filter}
      <button class="clear" onclick={() => (filter = '')} aria-label="Clear search" title="Clear search">
        <Icon name="x" size={12} />
      </button>
    {/if}
  </div>

  <div class="rows">
    {#if canvas.listError}
      <!-- Inline error (nothing loaded) or a stale bar over the last good list. -->
      <LoadState
        what="scenes"
        variant="compact"
        loading={canvas.listLoading}
        error={canvas.listError}
        empty={!canvas.scenes.length}
        onretry={() => void canvas.loadScenes().catch(() => {})}
      />
    {/if}
    {#if canvas.listError && !canvas.scenes.length}
      <!-- rendered above -->
    {:else if canvas.listLoading && !canvas.scenes.length}
      <LoadState what="scenes" variant="compact" loading empty />
    {:else if !rows.length}
      <div class="hint">
        {#if filter}
          No scenes match “{filter.trim()}”.
          <button class="btn small" onclick={() => (filter = '')}>Clear search</button>
        {:else}
          No scenes yet.
        {/if}
      </div>
    {:else}
      {#each groups as [section, items] (section)}
        {#if section !== ''}
          <button
            class="section-head"
            aria-expanded={!collapsed[section]}
            title={section}
            onclick={() => (collapsed = { ...collapsed, [section]: !collapsed[section] })}
          >
            <Icon name={collapsed[section] ? 'chevronRight' : 'chevronDown'} size={12} />
            <Icon name="folder" size={13} />
            <span class="section-label">{section.replace(/\//g, ' / ')}</span>
            <span class="section-count">{items.length}</span>
          </button>
        {/if}
        {#if section === '' || !collapsed[section]}
          {#each items as s (s.id)}
            <!-- The row is a real button (open) plus a sibling ⋯ button (the
                 row menu, also on right-click / ⇧F10) — no role=button div. -->
            <div
              class="row"
              class:active={canvas.currentId === s.id}
              class:nested={section !== ''}
              role="group"
              aria-label={s.title}
              oncontextmenu={(e) => rowMenu(e, s)}
            >
              <button
                class="row-open"
                aria-current={canvas.currentId === s.id ? 'true' : undefined}
                onclick={() => void canvas.open(s.id).catch(() => {})}
                ondblclick={(e) => rename(e, s)}
                onkeydown={(e) => {
                  if (e.key === 'F2') { e.preventDefault(); void rename(null, s); }
                  else if (e.key === 'F10' && e.shiftKey) rowMenu(e, s);
                }}
                title={s.title}
              >
                <span class="title">{s.title}</span>
                <span class="when">edited {rel(s.updated_at)}</span>
              </button>
              <button
                class="icon-btn row-more"
                class:shown={canvas.currentId === s.id}
                onclick={(e) => rowMenu(e, s)}
                aria-label="Actions for {s.title}"
                aria-haspopup="menu"
                title="Rename, move, duplicate or delete"
              >
                <Icon name="more" size={14} />
              </button>
            </div>
          {/each}
        {/if}
      {/each}
    {/if}
  </div>
</div>

<style>
  .scene-list {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .search {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 10px 10px 8px;
    padding: 4px 8px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text-dim);
  }
  /* The field itself is borderless; the box carries the focus ring. */
  .search:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent);
  }
  .search input {
    flex: 1 1 auto;
    border: none;
    outline: none;
    background: transparent;
    color: var(--text);
    font-size: var(--fs-m);
    min-width: 0;
  }
  .search input::-webkit-search-cancel-button {
    display: none;
  }
  .clear {
    border: none;
    background: none;
    color: var(--text-dim);
    cursor: pointer;
    padding: 0;
    display: inline-flex;
  }
  .rows {
    flex: 1 1 auto;
    overflow-y: auto;
    min-height: 0;
    padding: 0 6px 8px;
  }
  .hint {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 16px 12px;
    color: var(--text-dim);
    font-size: var(--fs-s);
    text-align: center;
  }
  .section-head {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 8px 4px;
    border: none;
    background: none;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: 0.02em;
    cursor: pointer;
    text-transform: uppercase;
  }
  .section-head:hover {
    color: var(--text);
  }
  .section-label {
    flex: 1 1 auto;
    min-width: 0;
    text-align: start;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .section-count {
    font-weight: 400;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 2px;
    padding-inline-end: 4px;
    border-radius: var(--radius-s);
  }
  .row.nested {
    margin-inline-start: 10px;
  }
  .row:hover {
    background: var(--hover);
  }
  .row.active {
    background: var(--accent-soft);
  }
  .row-open {
    flex: 1 1 auto;
    min-width: 0;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 2px;
    padding: 6px 8px;
    border: none;
    background: none;
    color: inherit;
    font: inherit;
    text-align: start;
    cursor: pointer;
    border-radius: var(--radius-s);
  }
  .title {
    max-width: 100%;
    font-size: var(--fs-m);
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .when {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .row-more {
    flex: none;
    width: 24px;
    height: 24px;
    opacity: 0;
  }
  /* Shown on hover, on keyboard focus, on the open scene, and wherever there
     is no hover (touch) — never a hover-only dead control. */
  .row:hover .row-more,
  .row-more:focus-visible,
  .row-more.shown {
    opacity: 1;
  }
  @media (hover: none) {
    .row-more {
      opacity: 1;
    }
  }
</style>
