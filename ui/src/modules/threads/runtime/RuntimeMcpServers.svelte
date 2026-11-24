<!--
  Thread ⋯ ▸ "MCP servers": every server this thread's agent has — Otto's
  own tools, the workspace's servers (handed to the CLI or reached through
  Otto), the CLI's own config — each with a status word and, when it is not
  connected, why (mcpServers.ts). One column, readable on a phone.
-->
<script lang="ts">
  import { untrack } from 'svelte';
  import { api } from '../../../lib/api/client';
  import Modal from '../../../lib/components/Modal.svelte';
  import { harnessLabel, sessionProviderSlug } from '../../../lib/providers';
  import { auth } from '../../../lib/stores/auth.svelte';
  import { router } from '../../../lib/router.svelte';
  import type { McpServerDetail } from '../../../lib/api/types';
  import { mcpServerRows, type GovernedServer } from './mcpServers';
  import { mcpServersSheet } from './mcpServersSheet.svelte';
  import { runtimeThreads } from './runtimeThreads.svelte';
  import { runtimeViews } from './runtimeView.svelte';

  const id = $derived(mcpServersSheet.id);
  const session = $derived(id ? runtimeThreads.get(id) : undefined);
  const agent = $derived(session ? harnessLabel(sessionProviderSlug(session)).replace(/ Code$/, '') : 'The agent');
  let governed = $state<GovernedServer[]>([]);

  $effect(() => {
    const open = id;
    if (!open) return;
    const ws = untrack(() => session?.workspace_id);
    governed = [];
    if (ws && auth.can('mcp', 'view')) void loadGoverned(open, ws);
    return untrack(() => runtimeViews.open(open));
  });

  const servers = $derived(id ? (runtimeViews.projections[id]?.mcpServers ?? []) : []);
  const rows = $derived(mcpServerRows(servers, governed, agent));
  const neverStarted = $derived(servers.length === 0);

  /** The workspace's governed servers and how many of their tools are on. */
  async function loadGoverned(open: string, ws: string): Promise<void> {
    const q = encodeURIComponent(ws);
    const [servers, tools] = await Promise.all([
      api.get<McpServerDetail[]>(`/workspaces/${q}/mcp/servers`).catch(() => [] as McpServerDetail[]),
      api.get<{ tools: { server_name: string }[] }>(`/mcp/gateway/tools?workspace_id=${q}`).catch(() => ({ tools: [] })),
    ]);
    if (mcpServersSheet.id !== open) return;
    governed = servers
      .filter((s) => s.managed && s.enabled)
      .map((s) => ({ name: s.name, health_status: s.health_status, health_error: s.health_error, tools: tools.tools.filter((t) => t.server_name === s.name).length }));
  }

  function manage(): void {
    mcpServersSheet.close();
    router.go('mcp');
  }
</script>

{#if id}
  <Modal title="MCP servers" width={520} onclose={() => mcpServersSheet.close()}>
    <div class="ms" data-testid="runtime-mcp-servers">
      {#if neverStarted}
        <p class="ms-note">{agent} hasn’t started in this thread yet. Its servers show here once it does.</p>
      {:else}
        <p class="ms-note">What {agent} has in this thread, as of its last start.</p>
      {/if}
      <ul class="ms-list">
        {#each rows as r (r.name + r.from)}
          <li class="ms-row" data-testid="runtime-mcp-server" data-name={r.name}>
            <div class="ms-line">
              <span class="ms-name ellipsis" dir="auto">{r.name}</span>
              <span class="ms-status ms-{r.tone}">{r.status}</span>
            </div>
            <div class="ms-from">{r.from}</div>
            {#if r.reason}<p class="ms-reason" dir="auto">{r.reason}</p>{/if}
          </li>
        {/each}
      </ul>
    </div>
    {#snippet footer()}
      {#if auth.can('mcp', 'view')}<button type="button" class="btn" onclick={manage}>Manage servers</button>{/if}
      <button type="button" class="btn primary" onclick={() => mcpServersSheet.close()}>Done</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .ms {
    display: flex;
    flex-direction: column;
    gap: var(--sp-4);
    min-inline-size: 0;
  }
  .ms-note {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .ms-list {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .ms-row {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding-block: var(--sp-4);
    border-block-end: 1px solid var(--border);
    min-inline-size: 0;
  }
  .ms-row:last-child {
    border-block-end: 0;
  }
  .ms-line {
    display: flex;
    align-items: baseline;
    gap: var(--sp-4);
    min-inline-size: 0;
  }
  .ms-name {
    flex: 1;
    min-inline-size: 0;
    font-size: var(--fs-m);
    font-weight: 500;
  }
  .ms-status {
    flex: none;
    font-size: var(--fs-s);
  }
  .ms-success {
    color: var(--success);
  }
  .ms-warning {
    color: var(--warning);
  }
  .ms-danger {
    color: var(--danger);
  }
  .ms-dim,
  .ms-from {
    color: var(--text-dim);
  }
  .ms-from {
    font-size: var(--fs-s);
  }
  .ms-reason {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--fs-s);
    overflow-wrap: anywhere;
  }
</style>
