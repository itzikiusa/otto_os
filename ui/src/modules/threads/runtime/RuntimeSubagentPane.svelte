<!--
  A subagent opened as a tab in the right panel while its parent stays put
  (T3 Code's SideThreadPane, MIT, Copyright (c) 2026 T3 Tools Inc.): the
  "Subagent of …" line, its whole thread (steerable — it has a composer),
  and "Open in main view". Where a click opens a subagent is the per-device
  pref Settings ▸ Threads writes (threadPrefs.subagentIn). Having it here
  counts as looking at it (thread_seen_at, as the sidebar reads it).
-->
<script lang="ts">
  import { threads } from '../threadsStore.svelte';
  import { threadMeta } from '../threadMeta.svelte';
  import RuntimeSubagentOf from './RuntimeSubagentOf.svelte';
  import RuntimeThread from './RuntimeThread.svelte';

  let { id }: { id: string } = $props();
  // In front of the person: seen now, and again when a turn ends in view.
  $effect(() => {
    void threadMeta.endedAt(id);
    threadMeta.markSeen(id);
  });
</script>

<div class="sp" data-testid="runtime-subagent-pane" data-thread={id}>
  <RuntimeSubagentOf threadId={id} />
  <div class="sp-thread">
    {#key id}<RuntimeThread sessionId={id} focused={false} compact />{/key}
  </div>
  <div class="sp-foot">
    <button type="button" class="btn small ghost" data-testid="runtime-subagent-main" onclick={() => threads.selectThread(id)}>Open in main view</button>
  </div>
</div>

<style>
  .sp {
    display: flex;
    flex-direction: column;
    block-size: 100%;
    min-block-size: 0;
  }
  .sp-thread {
    flex: 1;
    min-block-size: 0;
    display: flex;
    flex-direction: column;
  }
  .sp-foot {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
    padding: 4px 12px;
    border-block-start: 1px solid var(--border);
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  @media (pointer: coarse) {
    .sp-foot {
      min-block-size: 44px;
    }
  }
</style>
