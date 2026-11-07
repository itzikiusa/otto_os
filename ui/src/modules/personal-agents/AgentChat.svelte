<!--
  A personal agent's chat tab: one conversation pinned to the agent's
  provider / model / persona folder. On a structured-runtime provider it is a
  Threads thread rendered with the Threads conversation view (through
  lib/runtimeThreadPort.ts) — for the main agent, the threads of the agents
  it delegates to show as linked-thread chips above it. Other providers keep
  a terminal session.
-->
<script lang="ts">
  import { untrack } from 'svelte';
  import { announce, unannounce } from '../../lib/announce.svelte';
  import { personalAgentsApi } from '../../lib/api/personalAgents';
  import LazyTerminal from '../../lib/components/LazyTerminal.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { loadRuntimeThreadView } from '../../lib/runtimeThreadPort';

  interface Props {
    agentId: string;
    agentName: string;
    workspaceId: string;
    /** Bumped by Reset: the old chat is gone, open a fresh one. */
    reloadKey?: number;
  }
  let { agentId, agentName, workspaceId, reloadKey = 0 }: Props = $props();

  const ThreadView = lazyComponent(loadRuntimeThreadView);
  let chat = $state<{ id: string; runtime: boolean } | null>(null);
  let error = $state('');
  // Bumped by Retry (clearing an already-empty chat is no change).
  let attempt = $state(0);

  $effect(() => {
    void attempt;
    void reloadKey;
    const id = agentId;
    chat = null;
    error = '';
    const opening = `Opening ${untrack(() => agentName)}’s chat…`;
    announce(opening);
    // Clear it once the chat is up (or this agent is left), so a stale
    // "Opening X’s chat…" never lingers in the live region.
    const done = (): void => unannounce(opening);
    // The server serialises the get-or-create per agent; a stale answer for
    // another agent is dropped.
    personalAgentsApi
      .chatSession(id)
      .then((r) => {
        if (id === agentId) chat = { id: r.session_id, runtime: !!r.runtime };
      })
      .catch((e) => {
        if (id === agentId) error = loadErrorText(e);
      })
      .finally(done);
    return done;
  });
</script>

{#if error}
  <LoadState what="the chat" {error} empty onretry={() => (attempt += 1)} />
{:else if chat?.runtime}
  <div class="chatwrap">
    {#if ThreadView.error}
      <LoadState what="the chat" error={ThreadView.error} onretry={() => ThreadView.retry()} />
    {:else if ThreadView.component}
      <ThreadView.component sessionId={chat.id} {workspaceId} {agentName} />
    {:else}
      <LoadState what="the chat" loading />
    {/if}
  </div>
{:else if chat}
  <div class="chatwrap">
    <LazyTerminal sessionId={chat.id} autoFocus />
  </div>
{:else}
  <p class="muted">Opening {agentName}’s chat…</p>
{/if}

<style>
  .chatwrap {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
</style>
