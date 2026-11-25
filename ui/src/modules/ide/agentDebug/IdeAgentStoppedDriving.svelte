<script lang="ts">
  // "‹Claude› stopped driving — this debug session is yours": the agent that
  // started this debug session stopped (Stop on the driving bar, UI control
  // turned off, its session ended) while the debuggee still runs. One quiet
  // line: the session is the person's now — keep using it, or Stop it.
  import { uiControl, agentName } from '../../../lib/stores/uiControl.svelte';
  import { announceShown } from '../../../lib/announce.svelte';
  import { ide } from '../ideStore.svelte';

  const a = $derived(uiControl.agentDebug);
  const show = $derived(!!a && a.released && ide.activeDebug?.id === a.debugId);
  let stopping = $state(false);

  async function stop(): Promise<void> {
    if (stopping) return;
    stopping = true;
    try {
      await ide.stopDebug();
      uiControl.forgetDebug();
    } finally {
      stopping = false;
    }
  }
</script>

{#if show && a}
  <div class="dbg-agent-left" data-testid="ide-debug-agent-stopped" use:announceShown={{ text: `${agentName(a.agent)} stopped driving — this debug session is yours` }}>
    <span class="dbg-agent-left-text" dir="auto">{agentName(a.agent)} stopped driving — this debug session is yours</span>
    <button type="button" class="btn small" onclick={stop} disabled={stopping} data-testid="ide-debug-agent-stopped-stop">Stop</button>
    <button type="button" class="btn small ghost" onclick={() => uiControl.forgetDebug()} data-testid="ide-debug-agent-stopped-dismiss">Dismiss</button>
  </div>
{/if}

<style>
  .dbg-agent-left {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 8px;
    padding-block: 6px;
    padding-inline: 12px 8px;
    border-radius: var(--radius-m);
    background: var(--info-soft);
    color: var(--text);
    font-size: var(--fs-s);
  }
  .dbg-agent-left-text {
    flex: 1 1 auto;
    min-width: 0;
    overflow-wrap: anywhere;
  }
</style>
