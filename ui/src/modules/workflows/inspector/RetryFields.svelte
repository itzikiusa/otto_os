<script lang="ts">
  // A step's retry policy (node.retry, not params): extra attempts with
  // exponential backoff, or the default for its kind.
  import type { WorkflowNode } from '../../../lib/api/types';
  import { clearRetry, effectiveRetry, updateRetry } from '../retryPolicy';

  let { node, onedit }: { node: WorkflowNode; onedit: (mutate: (n: WorkflowNode) => void) => void } = $props();
  const r = $derived(effectiveRetry(node));
  const ids = $props.id();
</script>

<div class="rf">
  <span class="node-hint">Retries: {node.retry ? 'custom' : 'the default'} — {r.max_attempts} extra {r.max_attempts === 1 ? 'attempt' : 'attempts'}</span>
  {#if node.retry}<button type="button" class="btn ghost small" onclick={() => onedit((n) => clearRetry(n))}>Use the default</button>{/if}
  <label for="{ids}-max">Extra attempts (0–5)</label>
  <input id="{ids}-max" type="number" min="0" max="5" value={r.max_attempts}
    oninput={(e) => { const v = Number(e.currentTarget.value); onedit((n) => (n.retry = updateRetry(n, 'max_attempts', v))); }} />
  <label for="{ids}-bo">Wait before retrying (ms)</label>
  <input id="{ids}-bo" type="number" min="0" max="60000" step="100" value={r.backoff_ms}
    oninput={(e) => { const v = Number(e.currentTarget.value); onedit((n) => (n.retry = updateRetry(n, 'backoff_ms', v))); }} />
</div>

<style>
  .rf {
    display: contents;
  }
</style>
