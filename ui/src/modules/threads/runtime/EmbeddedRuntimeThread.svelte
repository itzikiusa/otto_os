<!--
  One structured-runtime thread hosted outside Threads (a personal agent's
  chat, through lib/runtimeThreadPort.ts): the Threads conversation view with
  the runtime rows loaded and kept live while it is mounted. The linked-thread
  chips switch this view between the lead and the agents it delegated to;
  while a linked one is shown, "Open in Threads" hands it to the Threads page.
-->
<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { router } from '../../../lib/router.svelte';
  import RuntimeThread from './RuntimeThread.svelte';
  import { runtimeThreads } from './runtimeThreads.svelte';

  let { sessionId, workspaceId, agentName }: { sessionId: string; workspaceId: string; agentName?: string } = $props();

  /** The thread shown: the host's, or a linked one picked from the chips. */
  let picked = $state<string | null>(null);
  const shown = $derived(picked && runtimeThreads.group(sessionId).some((t) => t.id === picked) ? picked : sessionId);
  const current = $derived(runtimeThreads.get(shown));

  onMount(() => runtimeThreads.mount());
  $effect(() => {
    const w = workspaceId;
    void sessionId;
    // A freshly created chat may not be in the loaded rows yet: reload when
    // the host's thread changes.
    if (w) untrack(() => void runtimeThreads.load(w).catch(() => {}));
  });
  $effect(() => {
    void sessionId;
    picked = null;
  });
</script>

<div class="rt-embed" data-testid="embedded-runtime-thread">
  <!-- The host's own header names the chat; this bar only appears while a
       linked (delegated) thread is shown in its place. -->
  {#if shown !== sessionId}
    <div class="rt-embed-bar">
      <span class="ellipsis" dir="auto" title={current?.title ?? ''}>{current?.title ?? 'Thread'}</span>
      <button type="button" class="btn small" onclick={() => (picked = null)}>Back to chat</button>
      <button type="button" class="btn small" title="Open this thread in Threads" onclick={() => router.go(`threads/${shown}`)}>
        <Icon name="external" size={12} /> Open in Threads
      </button>
    </div>
  {/if}
  <div class="rt-embed-body">
    {#key shown}
      <RuntimeThread sessionId={shown} focused agentChat={shown === sessionId ? agentName : undefined} onswitch={(id) => (picked = id === sessionId ? null : id)} />
    {/key}
  </div>
</div>

<style>
  .rt-embed {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-block-size: 0;
    block-size: 100%;
  }
  .rt-embed-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
    border-block-end: 1px solid var(--border);
    color: var(--text-dim);
    font-size: var(--fs-s);
    flex: none;
  }
  .rt-embed-bar > span {
    flex: 1;
    min-inline-size: 0;
  }
  .rt-embed-body {
    flex: 1;
    min-block-size: 0;
    display: flex;
    flex-direction: column;
  }
</style>
