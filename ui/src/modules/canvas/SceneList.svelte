<script lang="ts">
  import { rowMenu as rowMenuKeys } from '../../lib/rowMenu';
  import { toastError } from '../../lib/toastError';
  // The Canvas list pane (a ListPane, design.md §3): "Scenes N · ＋ · ⋯" with
  // a filter row; scenes grouped into folding SECTIONS (a folder path like
  // "Platform/Staging"), rows on the shared row look (F2 / double-click
  // rename, ⋯ / right-click / long-press: rename, move, duplicate, delete).
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import ListPane from '../../lib/components/side/ListPane.svelte';
  import SideSection from '../../lib/components/side/SideSection.svelte';
  import { canvas } from '../../lib/stores/canvas.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { api } from '../../lib/api/client';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import type { CanvasSceneSummary, CanvasScene } from '../../lib/canvas/types';

  interface Props {
    /** The pane's one primary: "New scene" (the format menu, anchored to it). */
    onnew: (anchor: Element | null) => void;
    /** A row was chosen (a phone pushes the board). */
    onpick: () => void;
  }
  let { onnew, onpick }: Props = $props();


  let filter = $state('');
  let filterEl = $state<HTMLInputElement | null>(null);
  let collapsed = $state<Record<string, boolean>>({});
  function pick(s: CanvasSceneSummary): void {
    onpick();
    // The open scene stays as it is (a phone's Back keeps it): no reload.
    if (canvas.currentId !== s.id) void canvas.open(s.id).catch(() => {});
  }
  function clearFilter(): void {
    filter = '';
    filterEl?.focus();
  }


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
      toastError('Couldn’t rename', err);
    }
  }

  async function move(e: MouseEvent | null, s: CanvasSceneSummary): Promise<void> {
    e?.stopPropagation();
    const sec = await confirmer.promptText('Sections group scenes in the list. Use / to nest them.', {
      title: 'Move to section',
      initial: s.section ?? '',
      placeholder: 'e.g. Platform/Staging — empty = no section',
      confirmLabel: 'Move',
      allowEmpty: true,
    });
    if (sec === null) return;
    try {
      await canvas.updateMeta(s.id, { section: sec.trim() || null });
    } catch (err) {
      toastError('Couldn’t move', err);
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
      toastError('Couldn’t duplicate', err);
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
      toastError('Couldn’t delete', err);
    }
  }
</script>

<ListPane
  count={canvas.scenes.length}
  primary={{ icon: 'plus', label: 'New scene', onclick: () => onnew(document.querySelector('[data-testid="canvas-new-scene"]')), testid: 'canvas-new-scene' }}
  menu={() => [{ label: 'Refresh', icon: 'refresh', action: () => void canvas.loadScenes().catch(() => {}) }]}
  loading={canvas.listLoading && !canvas.scenes.length}
  testid="canvas-scenes"
>
  {#snippet searchRow()}
    <div class="cv-search">
      <Icon name="search" size={14} />
      <input dir="auto" bind:this={filterEl} type="search" placeholder="Filter scenes…" aria-label="Filter scenes" bind:value={filter} spellcheck="false" />
      {#if filter}
        <button type="button" class="icon-btn" onclick={clearFilter} aria-label="Clear the filter" title="Clear the filter">
          <Icon name="x" size={12} />
        </button>
      {/if}
    </div>
  {/snippet}

  <div class="scene-list">
    {#if canvas.listError}
      <!-- A refresh failed: a stale bar over the last good list (nothing
           loaded at all → the page owns the error, the pane is not shown). -->
      <LoadState what="scenes" variant="compact" loading={canvas.listLoading} error={canvas.listError} onretry={() => void canvas.loadScenes().catch(() => {})} />
    {/if}
    {#if !rows.length}
      <div class="hint">
        No scenes match “{filter.trim()}”.
        <button type="button" class="btn small" onclick={clearFilter}>Clear search</button>
      </div>
    {:else}
      {#each groups as [section, items] (section)}
        {#if section === ''}
          {#each items as s (s.id)}{@render row(s)}{/each}
        {:else}
          <SideSection
            title={section.replace(/\//g, ' / ')}
            count={items.length}
            bind:open={() => !collapsed[section], (v) => (collapsed = { ...collapsed, [section]: !v })}
          >
            {#each items as s (s.id)}{@render row(s)}{/each}
          </SideSection>
        {/if}
      {/each}
    {/if}
  </div>
</ListPane>

{#snippet row(s: CanvasSceneSummary)}
  <!-- The shared row look (app.css .side-row), drawn here for F2 / double-click rename. -->
  <div class="side-row two-line" class:selected={canvas.currentId === s.id} data-testid="canvas-scene-row">
    <button
      type="button"
      class="side-row-main"
      data-side-row
      aria-current={canvas.currentId === s.id ? 'true' : undefined}
      title={s.title}
      onclick={() => pick(s)}
      ondblclick={(e) => rename(e, s)}
      onkeydown={(e) => {
        if (e.key === 'F2') { e.preventDefault(); void rename(null, s); }
      }}
      oncontextmenu={(e) => rowMenu(e, s)}
      use:rowMenuKeys
    >
      <span class="side-row-text">
        <span class="side-row-title" dir="auto">{s.title}</span>
        <span class="side-row-meta">edited {rel(s.updated_at)}</span>
      </span>
    </button>
    <button
      type="button"
      class="icon-btn side-row-more"
      tabindex="-1"
      onclick={(e) => rowMenu(e, s)}
      aria-label="Actions for {s.title}"
      aria-haspopup="menu"
      title="Rename, move, duplicate or delete"
    >
      <Icon name="more" size={14} />
    </button>
  </div>
{/snippet}

<style>
  /* The pane's one search row (the Threads look): one clear control. */
  .cv-search {
    display: flex;
    align-items: center;
    gap: 6px;
    block-size: 28px;
    padding-inline: 8px 2px;
    border: 1px solid var(--control-border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text-dim);
  }
  .cv-search:focus-within {
    color: var(--text);
    border-color: var(--accent-text);
  }
  .cv-search input {
    flex: 1;
    min-width: 0;
    block-size: 100%;
    border: 0;
    padding: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
    outline: none;
  }
  .cv-search input::-webkit-search-cancel-button {
    display: none;
  }
  @media (pointer: coarse) {
    .cv-search {
      block-size: 36px;
    }
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
</style>
