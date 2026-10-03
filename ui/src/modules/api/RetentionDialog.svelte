<script lang="ts">
  // History retention (workspace setting, opt-in): keep the newest N requests
  // and/or drop requests older than D days, and the newest N runs per
  // automation. 0 = no limit. Saving applies the limits right away.
  import Modal from '../../lib/components/Modal.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { apiClient } from '../../lib/stores/apiClient.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { formatBytes } from './storageGauge';

  interface Props { onclose: () => void }
  let { onclose }: Props = $props();

  // svelte-ignore state_referenced_locally
  let rows = $state(String(ws.apiHistoryRetention.rows));
  // svelte-ignore state_referenced_locally
  let days = $state(String(ws.apiHistoryRetention.days));
  // svelte-ignore state_referenced_locally
  let runs = $state(String(ws.apiRunsKeep));
  let busy = $state(false);

  const parse = (v: string): number | null => {
    const n = Number(v.trim());
    return v.trim() !== '' && Number.isInteger(n) && n >= 0 ? n : null;
  };
  const r = $derived(parse(rows));
  const d = $derived(parse(days));
  const k = $derived(parse(runs));
  $effect(() => { void apiClient.loadStorage(); });
  const gauge = $derived(apiClient.storage);

  async function save(): Promise<void> {
    if (r === null || d === null || k === null) return;
    busy = true;
    try {
      await ws.setApiHistoryRetention(r, d);
      if (k !== ws.apiRunsKeep) await ws.setApiRunsKeep(k);
      const applied = r || d || k ? await apiClient.applyRetention() : true;
      if (applied) toasts.success('Retention saved', r || d || k ? 'Applied now, and after every request or run.' : 'Everything is kept.');
      onclose();
    } catch (e) {
      toasts.error('Couldn’t save history retention', e instanceof Error ? e.message : String(e));
    } finally {
      busy = false;
    }
  }
</script>

<Modal title="History retention" width={420} {onclose}>
  <p class="lead">Every request you send is kept in this workspace’s history, including response bodies. Limit how much is kept; older requests are deleted for good.</p>
  {#if gauge}
    <p class="gauge" role="status">
      Now: {gauge.history_rows.toLocaleString()} requests ({formatBytes(gauge.history_bytes)}) · {gauge.run_rows.toLocaleString()} automation runs ({formatBytes(gauge.run_bytes)})
    </p>
  {/if}
  <div class="field">
    <label for="ret-rows">Keep the newest</label>
    <input id="ret-rows" class="input" inputmode="numeric" bind:value={rows} />
    <span class="hint" class:bad={r === null}>{r === null ? 'Enter a whole number (0 = no limit).' : r === 0 ? 'No limit on the number of requests.' : `Keep ${r} requests.`}</span>
  </div>
  <div class="field">
    <label for="ret-days">Delete requests older than (days)</label>
    <input id="ret-days" class="input" inputmode="numeric" bind:value={days} />
    <span class="hint" class:bad={d === null}>{d === null ? 'Enter a whole number (0 = never).' : d === 0 ? 'Never delete by age.' : `Delete after ${d} days.`}</span>
  </div>
  <div class="field">
    <label for="ret-runs">Automation runs to keep (per automation)</label>
    <input id="ret-runs" class="input" inputmode="numeric" bind:value={runs} />
    <span class="hint" class:bad={k === null}>{k === null ? 'Enter a whole number (0 = no limit).' : k === 0 ? 'Keep every run.' : `Keep the newest ${k} finished runs of each automation.`}</span>
  </div>
  {#snippet footer()}
    <button class="btn" onclick={onclose}>Cancel</button>
    <button class="btn primary" onclick={save} disabled={busy || r === null || d === null || k === null}>Save</button>
  {/snippet}
</Modal>

<style>
  .lead {
    margin: 0 0 12px;
    font-size: var(--fs-s);
    line-height: 1.5;
    color: var(--text-dim);
  }
  .gauge {
    margin: 0 0 12px;
    font-size: var(--fs-s);
    color: var(--text);
  }
  .bad {
    color: var(--danger);
  }
</style>
