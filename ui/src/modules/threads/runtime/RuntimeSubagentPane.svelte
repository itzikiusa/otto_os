<!--
  A subagent opened as a tab in the right panel while its parent stays put
  (T3 Code's SideThreadPane, MIT, Copyright (c) 2026 T3 Tools Inc.): the
  "Subagent of …" line, its whole thread (steerable — it has a composer),
  "Open in main view", and the one per-device setting that flips where a
  click opens subagents.
-->
<script lang="ts">
  import { threads } from '../threadsStore.svelte';
  import { threadsChrome } from '../threadsChrome.svelte';
  import RuntimeSubagentOf from './RuntimeSubagentOf.svelte';
  import RuntimeThread from './RuntimeThread.svelte';

  let { id }: { id: string } = $props();
</script>

<div class="sp" data-testid="runtime-subagent-pane" data-thread={id}>
  <RuntimeSubagentOf threadId={id} />
  <div class="sp-thread">
    {#key id}<RuntimeThread sessionId={id} focused={false} compact />{/key}
  </div>
  <div class="sp-foot">
    <label class="sp-pref">
      <input
        type="checkbox"
        checked={threadsChrome.subagentsIn === 'main'}
        data-testid="runtime-subagents-in-main"
        onchange={(e) => threadsChrome.setSubagentsIn(e.currentTarget.checked ? 'main' : 'panel')}
      />
      Open subagents in the main view
    </label>
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
    gap: 8px;
    padding: 4px 12px;
    border-block-start: 1px solid var(--border);
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .sp-pref {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    flex: 1;
    min-inline-size: 0;
  }
  @media (pointer: coarse) {
    .sp-foot {
      min-block-size: 44px;
    }
  }
</style>
