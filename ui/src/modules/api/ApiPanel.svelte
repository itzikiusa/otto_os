<script lang="ts">
  // Compact API client for the right-side panel. Reuses RequestBuilder +
  // ResponseViewer; a slim collection/history dropdown replaces the big tree.
  import { untrack } from 'svelte';
  import { apiClient } from '../../lib/stores/apiClient.svelte';
  import RequestBuilder from './RequestBuilder.svelte';
  import ResponseViewer from './ResponseViewer.svelte';
  import EnvSelector from './EnvSelector.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';

  // Load on first mount / workspace change — keyed on the workspace only
  // (loadAll's synchronous prologue reads the tabs, which must not re-trigger).
  $effect(() => {
    if (ws.currentId) untrack(() => void apiClient.loadAll());
  });

  // A flat picker: pick a saved request or a history entry to load into the builder.
  function onPick(e: Event): void {
    const v = (e.currentTarget as HTMLSelectElement).value;
    if (!v) return;
    if (v.startsWith('r:')) {
      const r = apiClient.requests.find((x) => x.id === v.slice(2));
      if (r) apiClient.loadRequestIntoDraft(r);
    } else if (v.startsWith('h:')) {
      const h = apiClient.history.find((x) => x.id === v.slice(2));
      if (h) void apiClient.selectHistory(h.id);
    } else if (v === 'new') {
      apiClient.newDraft();
    }
    (e.currentTarget as HTMLSelectElement).value = '';
  }
</script>

<div class="panel">
  {#if apiClient.historyLoadingId}<span class="note" role="status">Loading history request…</span>{/if}
  {#if apiClient.loadError}
    <div class="note err" role="alert">
      Couldn’t load saved requests. <button class="btn small" onclick={() => void apiClient.loadAll()}>Retry</button>
    </div>
  {/if}
  <div class="picker-row">
    <select class="input picker" onchange={onPick} aria-label="Load request">
      <option value="">Load…</option>
      <option value="new">New request</option>
      {#if apiClient.requests.length > 0}
        <optgroup label="Saved">
          {#each apiClient.requests as r (r.id)}
            <option value="r:{r.id}">{r.method} · {r.name}</option>
          {/each}
        </optgroup>
      {/if}
      {#if apiClient.history.length > 0}
        <optgroup label="Recent">
          {#each apiClient.history.slice(0, 15) as h (h.id)}
            <option value="h:{h.id}">{h.method} · {h.url}</option>
          {/each}
        </optgroup>
      {/if}
    </select>
  </div>

  <div class="builder-wrap">
    <RequestBuilder compact />
  </div>

  <details class="env-fold">
    <summary>Environment{#if apiClient.activeEnv} · <span class="env-on">{apiClient.activeEnv.name}</span>{/if}</summary>
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
    cursor: pointer;
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
