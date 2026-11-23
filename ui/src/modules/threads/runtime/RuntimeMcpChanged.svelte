<script lang="ts">
  // ONE quiet line when the workspace's MCP servers changed since this
  // thread's agent started: "MCP servers changed · Restart agent to apply"
  // (close + reopen: the agent resumes the same conversation with the new
  // set). Read on mount and whenever the agent reports its servers again —
  // never per stream event; the workspace list is shared for 15 s.
  import { announceShown } from '../../../lib/announce.svelte';
  import { api } from '../../../lib/api/client';
  import type { McpServerStatus } from '../../../lib/api/types';
  import { toastError } from '../../../lib/toastError';
  import { mcpChanged } from './mcpServers';
  import { nativeServerNames } from './nativeServers';
  import { runtimeThreads } from './runtimeThreads.svelte';

  interface Props {
    sessionId: string;
    workspaceId: string | undefined;
    servers: McpServerStatus[];
    /** No turn running: a restart would not cut one off. */
    idle: boolean;
  }
  let { sessionId, workspaceId, servers, idle }: Props = $props();
  let enabled = $state<string[] | null>(null);
  let busy = $state(false);

  $effect(() => {
    void servers;
    const ws = workspaceId;
    if (!ws) return;
    let live = true;
    void nativeServerNames(ws, (path) => api.get(path)).then((n) => { if (live) enabled = n; });
    return () => { live = false; };
  });

  const changed = $derived(enabled !== null && mcpChanged(servers, enabled));

  async function restart(): Promise<void> {
    busy = true;
    try {
      await runtimeThreads.close(sessionId);
      await runtimeThreads.reopen(sessionId);
    } catch (e) {
      toastError('Couldn’t restart the agent', e);
    } finally {
      busy = false;
    }
  }
</script>

{#if changed}
  <div class="mc" data-testid="runtime-mcp-changed" use:announceShown={'MCP servers changed'}>
    <span>MCP servers changed</span>
    {#if idle}
      <button type="button" class="btn small ghost" disabled={busy} onclick={restart} title="Restart the agent with the new servers; the conversation continues">Restart agent to apply</button>
    {:else}
      <span class="mc-dim">· applies when the agent restarts</span>
    {/if}
  </div>
{/if}

<style>
  .mc {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    padding-inline: var(--sp-4);
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .mc-dim {
    color: var(--text-dim);
  }
  @media (pointer: coarse) {
    .mc :global(.btn) {
      min-block-size: var(--hit-min);
    }
  }
</style>
