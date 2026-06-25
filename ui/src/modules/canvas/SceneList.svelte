<script lang="ts">
  // Scene list — the workspace's canvas scenes grouped into collapsible SECTIONS
  // (a folder path like "Platform/Staging" → sections + sub-sections), with
  // search, New, click-to-open, inline RENAME, MOVE-to-section, and delete.
  import Icon from '../../lib/components/Icon.svelte';
  import { canvas } from '../../lib/stores/canvas.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import type { CanvasSceneSummary, CanvasFormat } from './types';

  interface Props {
    /** Create a brand-new scene of the chosen format (the page opens it). */
    oncreate: (format: CanvasFormat) => void;
  }
  let { oncreate }: Props = $props();

  let filter = $state('');
  let collapsed = $state<Record<string, boolean>>({});
  let newMenu = $state(false);

  function create(format: CanvasFormat): void {
    newMenu = false;
    oncreate(format);
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

  function ago(iso: string): string {
    const t = Date.parse(iso);
    if (!t) return '';
    const s = Math.max(0, Math.floor((Date.now() - t) / 1000));
    if (s < 60) return 'just now';
    const m = Math.floor(s / 60);
    if (m < 60) return `${m}m ago`;
    const h = Math.floor(m / 60);
    if (h < 24) return `${h}h ago`;
    return `${Math.floor(h / 24)}d ago`;
  }

  async function rename(e: MouseEvent, s: CanvasSceneSummary): Promise<void> {
    e.stopPropagation();
    const t = await confirmer.promptText('Rename canvas', {
      title: 'Rename',
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

  async function move(e: MouseEvent, s: CanvasSceneSummary): Promise<void> {
    e.stopPropagation();
    const sec = await confirmer.promptText('Move to section', {
      title: 'Section',
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

  async function remove(e: MouseEvent, s: CanvasSceneSummary): Promise<void> {
    e.stopPropagation();
    const ok = await confirmer.ask(`Delete scene "${s.title}"? This can't be undone.`, {
      danger: true,
      confirmLabel: 'Delete',
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
  <div class="head">
    <button class="btn primary new" onclick={() => (newMenu = !newMenu)}>
      <Icon name="plus" size={14} /> New scene
      <Icon name="chevronDown" size={12} />
    </button>
    {#if newMenu}
      <button class="new-backdrop" aria-label="Close menu" onclick={() => (newMenu = false)}></button>
      <div class="new-menu">
        <button onclick={() => create('excalidraw')}>
          <Icon name="shapes" size={15} />
          <span><strong>Excalidraw board</strong><small>Editable shapes — draw by hand</small></span>
        </button>
        <button onclick={() => create('mermaid')}>
          <Icon name="branch" size={15} />
          <span><strong>Mermaid diagram</strong><small>Auto-rendered, any type</small></span>
        </button>
      </div>
    {/if}
  </div>

  <div class="search">
    <Icon name="search" size={13} />
    <input placeholder="Search scenes…" bind:value={filter} spellcheck="false" />
    {#if filter}
      <button class="clear" onclick={() => (filter = '')} aria-label="Clear search">
        <Icon name="x" size={12} />
      </button>
    {/if}
  </div>

  <div class="rows">
    {#if canvas.listLoading && !canvas.scenes.length}
      <div class="hint">Loading…</div>
    {:else if canvas.listError}
      <div class="hint err">{canvas.listError}</div>
    {:else if !rows.length}
      <div class="hint">{filter ? 'No matches.' : 'No scenes yet.'}</div>
    {:else}
      {#each groups as [section, items] (section)}
        {#if section !== ''}
          <button
            class="section-head"
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
            <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
            <div
              class="row"
              class:active={canvas.currentId === s.id}
              class:nested={section !== ''}
              onclick={() => void canvas.open(s.id)}
              role="button"
              tabindex="0"
              ondblclick={(e) => rename(e, s)}
              onkeydown={(e) => {
                if (e.key === 'Enter') void canvas.open(s.id);
              }}
            >
              <div class="meta">
                <span class="title">{s.title}</span>
                <span class="when">{ago(s.updated_at)}</span>
              </div>
              <div class="actions">
                <button onclick={(e) => rename(e, s)} aria-label="Rename" title="Rename">
                  <Icon name="edit" size={13} />
                </button>
                <button onclick={(e) => move(e, s)} aria-label="Move to section" title="Move to section">
                  <Icon name="folder" size={13} />
                </button>
                <button class="del" onclick={(e) => remove(e, s)} aria-label="Delete" title="Delete">
                  <Icon name="trash" size={13} />
                </button>
              </div>
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
  .head {
    padding: 10px 10px 6px;
    position: relative;
  }
  .new {
    width: 100%;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
  }
  .new-backdrop {
    position: fixed;
    inset: 0;
    z-index: 19;
    border: none;
    background: transparent;
    cursor: default;
  }
  .new-menu {
    position: absolute;
    top: 44px;
    left: 10px;
    right: 10px;
    z-index: 20;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    box-shadow: var(--shadow, 0 8px 28px rgba(0, 0, 0, 0.25));
    overflow: hidden;
  }
  .new-menu button {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 9px 11px;
    border: none;
    background: none;
    color: var(--text);
    cursor: pointer;
    text-align: start;
  }
  .new-menu button:hover {
    background: var(--surface-2);
  }
  .new-menu span {
    display: flex;
    flex-direction: column;
    line-height: 1.3;
  }
  .new-menu small {
    color: var(--text-dim);
    font-size: 11px;
  }
  .search {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0 10px 8px;
    padding: 4px 8px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text-dim);
  }
  .search input {
    flex: 1 1 auto;
    border: none;
    outline: none;
    background: transparent;
    color: var(--text);
    font-size: 12.5px;
    min-width: 0;
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
    padding: 16px 12px;
    color: var(--text-dim);
    font-size: 12.5px;
    text-align: center;
  }
  .hint.err {
    color: var(--status-exited);
  }
  .section-head {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 7px 8px 3px;
    border: none;
    background: none;
    color: var(--text-dim);
    font-size: 11.5px;
    font-weight: 700;
    letter-spacing: 0.02em;
    cursor: pointer;
    text-transform: uppercase;
  }
  .section-head:hover {
    color: var(--text);
  }
  .section-label {
    flex: 1 1 auto;
    text-align: start;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .section-count {
    font-weight: 600;
    opacity: 0.7;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 8px;
    border-radius: var(--radius-s);
    cursor: pointer;
  }
  .row.nested {
    margin-inline-start: 10px;
  }
  .row:hover {
    background: var(--surface-2);
  }
  .row.active {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
  }
  .meta {
    flex: 1 1 auto;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .title {
    font-size: 13px;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .when {
    font-size: 11px;
    color: var(--text-dim);
  }
  .actions {
    flex: 0 0 auto;
    display: flex;
    gap: 1px;
    opacity: 0;
  }
  .row:hover .actions {
    opacity: 1;
  }
  .actions button {
    border: none;
    background: none;
    color: var(--text-dim);
    cursor: pointer;
    padding: 2px;
    display: inline-flex;
    border-radius: var(--radius-s);
  }
  .actions button:hover {
    color: var(--text);
    background: var(--surface);
  }
  .actions .del:hover {
    color: var(--status-exited);
  }
</style>
