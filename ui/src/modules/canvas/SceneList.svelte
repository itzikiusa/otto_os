<script lang="ts">
  // Scene list — compact rows of the workspace's canvas scenes (most-recent
  // first), with a search filter, a "New scene" button, click-to-open, and a
  // confirmed delete. Mirrors the connection-list / saved-query patterns in the
  // DB Explorer for visual consistency.
  import Icon from '../../lib/components/Icon.svelte';
  import { canvas } from '../../lib/stores/canvas.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import type { CanvasSceneSummary } from './types';

  interface Props {
    /** Create a brand-new scene (handled by the page so it can open it). */
    oncreate: () => void;
  }
  let { oncreate }: Props = $props();

  let filter = $state('');

  const rows = $derived.by((): CanvasSceneSummary[] => {
    const q = filter.trim().toLowerCase();
    const list = q
      ? canvas.scenes.filter((s) => s.title.toLowerCase().includes(q))
      : canvas.scenes;
    // Most-recently-updated first (RFC3339 sorts chronologically as strings).
    return [...list].sort((a, b) => b.updated_at.localeCompare(a.updated_at));
  });

  // "3 minutes ago"-style relative time, kept tiny (no deps).
  function ago(iso: string): string {
    const t = Date.parse(iso);
    if (!t) return '';
    const s = Math.max(0, Math.floor((Date.now() - t) / 1000));
    if (s < 60) return 'just now';
    const m = Math.floor(s / 60);
    if (m < 60) return `${m}m ago`;
    const h = Math.floor(m / 60);
    if (h < 24) return `${h}h ago`;
    const d = Math.floor(h / 24);
    return `${d}d ago`;
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
    <button class="btn primary new" onclick={oncreate}>
      <Icon name="plus" size={14} /> New scene
    </button>
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
      {#each rows as s (s.id)}
        <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
        <div
          class="row"
          class:active={canvas.currentId === s.id}
          onclick={() => void canvas.open(s.id)}
          role="button"
          tabindex="0"
          onkeydown={(e) => {
            if (e.key === 'Enter') void canvas.open(s.id);
          }}
        >
          <div class="meta">
            <span class="title">{s.title}</span>
            <span class="when">{ago(s.updated_at)}</span>
          </div>
          <button class="del" onclick={(e) => remove(e, s)} aria-label="Delete scene">
            <Icon name="trash" size={13} />
          </button>
        </div>
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
  }
  .new {
    width: 100%;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
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
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 8px;
    border-radius: var(--radius-s);
    cursor: pointer;
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
  .del {
    flex: 0 0 auto;
    border: none;
    background: none;
    color: var(--text-dim);
    cursor: pointer;
    opacity: 0;
    padding: 2px;
    display: inline-flex;
    border-radius: var(--radius-s);
  }
  .row:hover .del {
    opacity: 1;
  }
  .del:hover {
    color: var(--status-exited);
    background: var(--surface);
  }
</style>
