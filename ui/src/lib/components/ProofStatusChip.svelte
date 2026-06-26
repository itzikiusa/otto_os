<script lang="ts">
  // A colored pill for a proof pack's overall status (missing/partial/passed/
  // failed/waived), with an optional risk score. `compact` shrinks it for the
  // inline sidebar session-row use.
  interface Props {
    status: string;
    risk?: number;
    compact?: boolean;
  }
  let { status, risk, compact = false }: Props = $props();

  type Tone = 'ok' | 'bad' | 'warn' | 'dim' | 'neutral';
  const TONE: Record<string, Tone> = {
    passed: 'ok',
    failed: 'bad',
    partial: 'warn',
    missing: 'dim',
    waived: 'neutral',
  };
  const tone = $derived(TONE[status] ?? 'dim');
  const showRisk = $derived(typeof risk === 'number' && risk > 0);
</script>

<span
  class="proof-status {tone}"
  class:compact
  title={showRisk ? `proof: ${status} · risk ${risk}` : `proof: ${status}`}
>
  <span class="lbl">{status}</span>
  {#if showRisk}<span class="risk">{risk}</span>{/if}
</span>

<style>
  .proof-status {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 18px;
    padding: 0 7px;
    border-radius: 999px;
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.01em;
    white-space: nowrap;
    text-transform: capitalize;
    border: 1px solid transparent;
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .proof-status.compact {
    height: 14px;
    padding: 0 5px;
    font-size: 9px;
  }
  .proof-status .risk {
    font-variant-numeric: tabular-nums;
    opacity: 0.85;
  }
  .proof-status.ok {
    color: var(--status-working);
    background: color-mix(in srgb, var(--status-working) 14%, transparent);
    border-color: color-mix(in srgb, var(--status-working) 35%, transparent);
  }
  .proof-status.bad {
    color: var(--status-exited);
    background: color-mix(in srgb, var(--status-exited) 14%, transparent);
    border-color: color-mix(in srgb, var(--status-exited) 35%, transparent);
  }
  .proof-status.warn {
    color: var(--status-warn);
    background: color-mix(in srgb, var(--status-warn) 14%, transparent);
    border-color: color-mix(in srgb, var(--status-warn) 38%, transparent);
  }
  .proof-status.dim {
    color: var(--text-dim);
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
    border-color: color-mix(in srgb, var(--text-dim) 28%, transparent);
  }
  .proof-status.neutral {
    color: var(--text-dim);
    background: var(--surface-2);
    border-color: var(--border);
  }
</style>
