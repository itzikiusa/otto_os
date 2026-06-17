<script lang="ts">
  // Full API-client module page: a left sidebar (Collections / History / Env
  // tabs) and a center column with the RequestBuilder over the ResponseViewer.
  import Icon from '../../lib/components/Icon.svelte';
  import RequestBuilder from './RequestBuilder.svelte';
  import ResponseViewer from './ResponseViewer.svelte';
  import CollectionsTree from './CollectionsTree.svelte';
  import HistoryList from './HistoryList.svelte';
  import EnvSelector from './EnvSelector.svelte';
  import AutomationsView from './AutomationsView.svelte';
  import { apiClient } from '../../lib/stores/apiClient.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';

  type SideTab = 'collections' | 'automations' | 'history' | 'env';
  let sideTab: SideTab = $state('collections');

  // Load everything when the workspace changes (collections/requests/envs/history
  // plus automations, which the runner chains together).
  $effect(() => {
    if (ws.currentId) {
      void apiClient.loadAll();
      void apiClient.loadAutomations();
    }
  });

  const sideTabs: { id: SideTab; icon: string; label: string }[] = [
    { id: 'collections', icon: 'folder', label: 'Collections' },
    { id: 'automations', icon: 'zap', label: 'Automations' },
    { id: 'history', icon: 'clock', label: 'History' },
    { id: 'env', icon: 'globe', label: 'Env' },
  ];
</script>

<div class="api-page">
  <aside class="api-side">
    <div class="side-tabs" role="tablist">
      {#each sideTabs as t (t.id)}
        <button
          class="side-tab"
          class:active={sideTab === t.id}
          role="tab"
          aria-selected={sideTab === t.id}
          onclick={() => (sideTab = t.id)}
        >
          <Icon name={t.icon} size={13} />
          {t.label}
          {#if t.id === 'env' && apiClient.activeEnv}<span class="side-dot"></span>{/if}
        </button>
      {/each}
    </div>
    <div class="side-body">
      {#if sideTab === 'collections'}
        <CollectionsTree />
      {:else if sideTab === 'automations'}
        <AutomationsView />
      {:else if sideTab === 'history'}
        <HistoryList />
      {:else}
        <EnvSelector />
      {/if}
    </div>
  </aside>

  <div class="api-main">
    <div class="req-tabs" role="tablist">
      {#each apiClient.tabs as t, i (i)}
        <div class="req-tab" class:active={apiClient.activeTab === i} role="tab" tabindex="0" aria-selected={apiClient.activeTab === i} onclick={() => apiClient.switchTab(i)} onkeydown={(e) => { if (e.key === 'Enter') apiClient.switchTab(i); }}>
          <span class="req-tab-method {(t.method || 'GET').toLowerCase()}">{t.method}</span>
          <span class="req-tab-label">{apiClient.tabLabel(t)}</span>
          <button class="req-tab-close" title="Close tab" aria-label="Close tab" onclick={(e) => { e.stopPropagation(); apiClient.closeTab(i); }}>×</button>
        </div>
      {/each}
      <button class="req-tab-new" title="New request (⌘T)" aria-label="New request tab" onclick={() => apiClient.newDraft()}>+</button>
    </div>
    <div class="builder-pane">
      <RequestBuilder />
    </div>
    <div class="resp-pane">
      <ResponseViewer />
    </div>
  </div>
</div>

<style>
  .api-page {
    height: 100%;
    display: flex;
    min-height: 0;
  }
  .api-side {
    width: 280px;
    flex-shrink: 0;
    border-right: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .side-tabs {
    display: flex;
    flex-wrap: wrap;
    gap: 2px;
    padding: 10px 8px 8px;
    border-bottom: 1px solid var(--border);
    overflow: hidden;
  }
  .side-tab {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 26px;
    padding: 0 7px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: 11.5px;
    font-weight: 500;
    cursor: pointer;
    white-space: nowrap;
  }
  .side-tab:hover {
    background: var(--surface-2);
  }
  .side-tab.active {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--accent);
  }
  .side-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--status-working);
  }
  .side-body {
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
    padding: 10px;
    min-height: 0;
    min-width: 0;
  }
  .api-main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .req-tabs {
    display: flex;
    align-items: stretch;
    gap: 2px;
    padding: 6px 8px 0;
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
    flex-shrink: 0;
  }
  .req-tab {
    display: flex;
    align-items: center;
    gap: 6px;
    max-width: 180px;
    padding: 6px 8px 6px 10px;
    border: 1px solid transparent;
    border-bottom: none;
    border-radius: var(--radius-s) var(--radius-s) 0 0;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    font-size: 12px;
    white-space: nowrap;
  }
  .req-tab:hover { background: var(--surface-2); }
  .req-tab.active {
    background: var(--surface-2);
    border-color: var(--border);
    color: var(--text);
  }
  .req-tab-method {
    font-family: var(--font-mono, monospace);
    font-weight: 700;
    font-size: 10px;
    color: var(--accent);
  }
  .req-tab-method.post { color: var(--status-working); }
  .req-tab-method.delete { color: var(--status-exited); }
  .req-tab-method.put, .req-tab-method.patch { color: #d2691e; }
  .req-tab-label {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .req-tab-close {
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    font-size: 15px;
    line-height: 1;
    padding: 0 2px;
    border-radius: 4px;
  }
  .req-tab-close:hover { background: var(--border); color: var(--text); }
  .req-tab-new {
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    font-size: 18px;
    padding: 0 10px;
    border-radius: var(--radius-s);
  }
  .req-tab-new:hover { background: var(--surface-2); color: var(--accent); }
  .builder-pane {
    padding: 14px 16px;
    border-bottom: 1px solid var(--border);
    overflow-y: auto;
    flex: 0 0 auto;
    max-height: 60%;
  }
  .resp-pane {
    flex: 1;
    min-height: 0;
    padding: 10px 16px 16px;
    display: flex;
    flex-direction: column;
  }
</style>
