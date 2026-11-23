<script lang="ts">
  // Collapsible live device log tail drawer for Browser's embedded SimulatorView.
  import Icon from '../../../lib/components/Icon.svelte';
  import type { SimLogServerMsg } from '../../../lib/api/types';

  export type SimLogEntry = Extract<SimLogServerMsg, { type: 'log' }> & { id: number };

  interface Props {
    deviceName: string;
    logs: readonly SimLogEntry[];
    onclear: () => void;
  }
  let { deviceName, logs, onclear }: Props = $props();
</script>

<div class="simv-logs">
  <div class="simv-logs-head">
    <span>Device logs ({logs.length})</span>
    <button class="icon-btn simv-icon" onclick={onclear} aria-label="Clear device logs" title="Clear logs">
      <Icon name="trash" size={12} />
    </button>
  </div>
  <div class="simv-logs-body scroll-thin" dir="ltr">
    {#if logs.length === 0}
      <p class="simv-logs-empty">Streaming logs from {deviceName}…</p>
    {:else}
      {#each logs as line (line.id)}
        <div class="simv-logline" class:err={line.level === 'error' || line.level === 'fatal'} class:warn={line.level === 'warn'}>
          <span class="simv-ltag">{line.tag || line.level}</span>
          <span class="simv-lmsg">{line.message}</span>
        </div>
      {/each}
    {/if}
  </div>
</div>

<style>
  .simv-icon {
    width: 26px;
    height: 26px;
  }
  .simv-logs {
    display: flex;
    flex-direction: column;
    height: 170px;
    max-height: 40%;
    border-top: 1px solid var(--border);
    background: var(--surface);
    flex: none;
  }
  .simv-logs-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 4px 10px;
    border-bottom: 1px solid var(--border);
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
  }
  .simv-logs-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 4px 8px;
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
  }
  .simv-logs-empty {
    margin: 8px 2px;
    color: var(--text-dim);
  }
  .simv-logline {
    display: flex;
    gap: 8px;
    padding: 2px 4px;
    min-width: 0;
  }
  .simv-logline.err { color: var(--danger); }
  .simv-logline.warn { color: var(--warning); }
  .simv-ltag {
    flex-shrink: 0;
    color: var(--text-dim);
    max-width: 110px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .simv-lmsg {
    flex: 1;
    min-width: 0;
    word-break: break-word;
  }
</style>
