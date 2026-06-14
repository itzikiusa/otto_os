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
    gap: 2px;
    padding: 10px 10px 8px;
    border-bottom: 1px solid var(--border);
  }
  .side-tab {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 26px;
    padding: 0 9px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
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
