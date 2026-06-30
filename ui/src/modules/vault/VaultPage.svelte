<script lang="ts">
  // Vault v2 — the "Repo Brain". A tabbed page over a workspace's knowledge and
  // code intelligence:
  //   Knowledge — the Obsidian-like memory browser (search/index/reader/backlinks)
  //   Graph     — the unified knowledge + code force graph (headline view)
  //   Repos     — indexed code repositories + an "index a repo" form
  //   Symbols   — a code symbol browser
  //   Backends  — remote backends (Qdrant / SurrealDB / Ollama)
  //   Brain     — assemble focused context for a task
  // The contract lives in docs/contracts; types in ui/src/lib/api/types.ts.
  import { vault } from './vault.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import KnowledgeView from './KnowledgeView.svelte';
  import CodeGraphView from './CodeGraphView.svelte';
  import ReposView from './ReposView.svelte';
  import SymbolsView from './SymbolsView.svelte';
  import BackendsView from './BackendsView.svelte';
  import BrainView from './BrainView.svelte';
  import type { VaultTab } from './vault.svelte';

  // Load this workspace's memories + the active embedder when available.
  $effect(() => {
    if (ws.currentId) void vault.load();
  });
  $effect(() => {
    void vault.loadEmbedder();
  });

  const TABS: Array<{ id: VaultTab; label: string; icon: string }> = [
    { id: 'knowledge', label: 'Knowledge', icon: 'note' },
    { id: 'graph', label: 'Graph', icon: 'branch' },
    { id: 'repos', label: 'Repos', icon: 'box' },
    { id: 'symbols', label: 'Symbols', icon: 'command' },
    { id: 'backends', label: 'Backends', icon: 'plug' },
    { id: 'brain', label: 'Brain', icon: 'radar' },
  ];
</script>

<div class="vault-page">
  <div class="vp-tabs" role="tablist" aria-label="Vault sections" data-testid="vault-tabs">
    {#each TABS as t (t.id)}
      <button
        class="vp-tab"
        class:active={vault.tab === t.id}
        role="tab"
        aria-selected={vault.tab === t.id}
        data-testid={`vault-tab-${t.id}`}
        onclick={() => (vault.tab = t.id)}
      >
        <Icon name={t.icon} size={13} />
        {t.label}
      </button>
    {/each}
  </div>

  <div class="vp-body" role="tabpanel" aria-label={`Vault ${vault.tab}`}>
    {#if vault.tab === 'knowledge'}
      <KnowledgeView />
    {:else if vault.tab === 'graph'}
      <CodeGraphView />
    {:else if vault.tab === 'repos'}
      <ReposView />
    {:else if vault.tab === 'symbols'}
      <SymbolsView />
    {:else if vault.tab === 'backends'}
      <BackendsView />
    {:else if vault.tab === 'brain'}
      <BrainView />
    {/if}
  </div>
</div>

<style>
  .vault-page {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    overflow: hidden;
  }
  .vp-tabs {
    display: flex;
    gap: 4px;
    padding: 8px 12px 0;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
    overflow-x: auto;
  }
  .vp-tab {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    border: 1px solid transparent;
    border-bottom: none;
    background: transparent;
    color: var(--text-dim);
    font-size: 12.5px;
    font-weight: 600;
    padding: 7px 13px;
    border-radius: 8px 8px 0 0;
    cursor: pointer;
    white-space: nowrap;
  }
  .vp-tab:hover {
    color: var(--text);
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
  }
  .vp-tab.active {
    color: #0b0b0b;
    background: #7ee787;
    border-color: #7ee787;
  }
  .vp-body {
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }

  @media (max-width: 640px) {
    .vp-tabs { padding: 6px 8px 0; }
    .vp-tab { padding: 9px 12px; min-height: 38px; }
  }
</style>
