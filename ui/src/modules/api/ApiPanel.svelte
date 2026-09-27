<script lang="ts">
  // Compact API client for the right-side panel. Reuses RequestBuilder +
  // ResponseViewer; a slim collection/history dropdown replaces the big tree.
  import { untrack } from 'svelte';
  import { apiClient } from '../../lib/stores/apiClient.svelte';
  import RequestBuilder from './RequestBuilder.svelte';
  import ResponseViewer from './ResponseViewer.svelte';
  import EnvSelector from './EnvSelector.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';

  // Load on first mount / workspace change — keyed on the workspace only
  // (loadAll's synchronous prologue reads the tabs, which must not re-trigger).
  $effect(() => {
    if (ws.currentId) untrack(() => void apiClient.loadAll());
  });

  // A flat, filterable picker (one shared menu, built on open) — a native
  // <select> mounted one <option> per saved request for as long as the panel
  // was open. Pinned rows (New request + the 15 most recent history entries)
  // always show; saved requests are searched by method/name/url, 50 at a time.
  function openPicker(e: MouseEvent): void {
    const items: MenuItem[] = [
      { label: 'New request', icon: 'plus', pinned: true, action: () => apiClient.newDraft() },
      ...apiClient.history.slice(0, 15).map((h): MenuItem => ({
        label: `${h.method} · ${h.url}`,
        icon: 'clock',
        pinned: true,
        action: () => void apiClient.selectHistory(h.id),
      })),
    ];
    if (apiClient.requests.length > 0) {
      items.push({ separator: true, pinned: true });
      for (const r of apiClient.requests) {
        items.push({ label: `${r.method} · ${r.name} · ${r.url}`, action: () => apiClient.loadRequestIntoDraft(r) });
      }
    }
    ctxMenu.show(e, items, { filter: true, filterPlaceholder: 'Search saved requests', maxVisible: 50 });
  }
</script>

<div class="panel">
  {#if apiClient.historyLoadingId}<span class="note" role="status">Loading history request…</span>{/if}
  {#if apiClient.requestsLoadError}
    <div class="note err" role="alert">
      Couldn’t load saved requests. <button class="btn small" onclick={() => void apiClient.loadAll()}>Retry</button>
    </div>
  {/if}
  <div class="picker-row">
    <button type="button" class="input picker" aria-label="Load request" aria-haspopup="menu" title="Load a saved or recent request" onclick={openPicker}>
      <span class="picker-label">Load…</span>
      <Icon name="chevronDown" size={12} />
    </button>
  </div>

  <div class="builder-wrap">
    <RequestBuilder compact />
  </div>

  <details class="env-fold">
    <summary>Environment{#if apiClient.activeEnv}{' '}· <span class="env-on">{apiClient.activeEnv.name}</span>{/if}</summary>
    <EnvSelector compact />
  </details>

  <div class="resp-wrap">
    <ResponseViewer compact />
  </div>
</div>

<style>
  .panel {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    padding: 10px;
    gap: 10px;
  }
  .note {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .note.err {
    color: var(--danger);
  }
  .picker-row {
    flex-shrink: 0;
  }
  .picker {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 6px;
    cursor: pointer;
    text-align: start;
  }
  .picker-label {
    flex: 1;
    min-width: 0;
    color: var(--text-dim);
  }
  .picker:focus-visible {
    outline: none;
    border-color: var(--accent-text);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 22%, transparent);
  }
  .builder-wrap {
    flex-shrink: 0;
  }
  .env-fold {
    flex-shrink: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 6px 8px;
  }
  .env-fold > summary {
    cursor: pointer;
    font-size: var(--fs-s);
    color: var(--text-dim);
    user-select: none;
  }
  .env-on {
    color: var(--accent-text);
    font-weight: 600;
  }
  .resp-wrap {
    flex: 1;
    min-height: 120px;
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--border);
    padding-top: 8px;
  }
</style>
