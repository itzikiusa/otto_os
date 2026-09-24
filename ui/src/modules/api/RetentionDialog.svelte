<script lang="ts">
  // History retention (workspace setting, opt-in): keep the newest N requests
  // and/or drop requests older than D days. 0 = no limit. Replaces the two
  // chained text prompts.
  import Modal from '../../lib/components/Modal.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';

  interface Props { onclose: () => void }
  let { onclose }: Props = $props();

  // svelte-ignore state_referenced_locally
  let rows = $state(String(ws.apiHistoryRetention.rows));
  // svelte-ignore state_referenced_locally
  let days = $state(String(ws.apiHistoryRetention.days));
  let busy = $state(false);

  const parse = (v: string): number | null => {
    const n = Number(v.trim());
    return v.trim() !== '' && Number.isInteger(n) && n >= 0 ? n : null;
  };
  const r = $derived(parse(rows));
  const d = $derived(parse(days));

  async function save(): Promise<void> {
    if (r === null || d === null) return;
    busy = true;
    try {
      await ws.setApiHistoryRetention(r, d);
      toasts.success('History retention saved', 'Applied after the next request.');
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
  {#snippet footer()}
    <button class="btn" onclick={onclose}>Cancel</button>
    <button class="btn primary" onclick={save} disabled={busy || r === null || d === null}>Save</button>
  {/snippet}
</Modal>

<style>
  .lead {
    margin: 0 0 12px;
    font-size: var(--fs-s);
    line-height: 1.5;
    color: var(--text-dim);
  }
  .bad {
    color: var(--danger);
  }
</style>
