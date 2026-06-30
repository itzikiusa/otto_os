<script lang="ts">
  // Backends tab — cards for the three remote backends. Each card edits/saves its
  // config, tests reachability, and offers a confirmed install flow.
  import { onMount } from 'svelte';
  import { vault } from './vault.svelte';
  import VaultBackendCard from './VaultBackendCard.svelte';

  const KINDS = ['qdrant', 'surreal', 'ollama'] as const;

  onMount(() => {
    void vault.loadBackends();
  });

  function backendFor(kind: string) {
    return vault.backends.find((b) => b.kind === kind) ?? null;
  }
</script>

<div class="backends">
  <header class="be-head">
    <h2>Remote backends</h2>
    <p class="hint">
      Plug the Vault into remote services for vector search (Qdrant), the graph
      (SurrealDB), and embeddings (Ollama). Secrets are stored in the macOS
      Keychain — never in the database.
    </p>
  </header>

  <div class="be-grid">
    {#each KINDS as kind (kind)}
      <VaultBackendCard {kind} backend={backendFor(kind)} />
    {/each}
  </div>
</div>

<style>
  .backends {
    height: 100%;
    overflow-y: auto;
    padding: 18px 22px 28px;
  }
  .be-head { margin-bottom: 16px; max-width: 760px; }
  h2 { font-size: 16px; margin: 0 0 4px; }
  .hint { font-size: 12.5px; color: var(--text-dim); margin: 0; line-height: 1.5; }
  .be-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
    gap: 14px;
  }
</style>
