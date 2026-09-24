<script lang="ts">
  // Environment badge — the one treatment for a connection / cluster / account
  // environment everywhere (DB Explorer, Brokers, Kubernetes, AWS): prod in the
  // danger tone, staging in warning, dev neutral (lib/status.ts `envTone`).
  // `readOnly` on a non-prod target shows "RO" instead (prod stays "prod" —
  // prod is the stronger guard). Text is ≥ --fs-xs, weight 600.
  import { envTone } from '../status';

  interface Props {
    env: string | null | undefined;
    readOnly?: boolean;
    /** Override the label (e.g. "…" while opening). */
    label?: string;
  }
  let { env, readOnly = false, label }: Props = $props();

  const info = $derived(envTone(env));
  const ro = $derived(readOnly && info.key !== 'prod');
  const text = $derived(label ?? (ro ? 'RO' : info.label));
  const tip = $derived(ro ? `Read-only${info.hint ? ` · ${info.hint}` : ''}` : (info.hint ?? info.label));
</script>

<span
  class="env-badge tone-{ro ? 'neutral' : info.tone}"
  data-env={info.key}
  title={tip}
>{text}</span>

<style>
  .env-badge {
    display: inline-flex;
    align-items: center;
    flex-shrink: 0;
    height: 16px;
    padding: 0 6px;
    border-radius: 999px;
    font-size: var(--fs-xs);
    font-weight: 600;
    line-height: 1;
    white-space: nowrap;
    color: var(--text-dim);
    background: var(--surface-2);
    border: 1px solid var(--border);
  }
  .tone-danger {
    color: var(--danger);
    background: var(--danger-soft);
    border-color: color-mix(in srgb, var(--danger) 35%, transparent);
  }
  .tone-warning {
    color: var(--warning);
    background: var(--warning-soft);
    border-color: color-mix(in srgb, var(--warning) 35%, transparent);
  }
</style>
