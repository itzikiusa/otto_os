<script lang="ts">
  // HTTP status as a soft-tinted pill: "200 OK", tone by class, with the class
  // in words in the tooltip (colour is never the only signal).
  import { statusTone, statusWord } from '../../lib/api/apiVars';

  interface Props {
    status: number | null | undefined;
    text?: string | null;
    small?: boolean;
  }
  let { status, text = null, small = false }: Props = $props();
</script>

<span class="schip {statusTone(status)}" class:small title={statusWord(status)}>
  {status ?? 'No response'}{#if text}&nbsp;{text}{/if}
</span>

<style>
  .schip {
    display: inline-flex;
    align-items: center;
    height: 20px;
    padding: 0 8px;
    border-radius: 999px;
    font-size: var(--fs-xs);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    flex-shrink: 0;
    color: var(--text-dim);
    background: var(--surface-2);
    border: 1px solid var(--border);
  }
  .small {
    height: 18px;
    padding: 0 6px;
  }
  .ok { color: var(--success); background: var(--success-soft); border-color: color-mix(in srgb, var(--success) 30%, transparent); }
  .redirect { color: var(--info); background: var(--info-soft); border-color: color-mix(in srgb, var(--info) 30%, transparent); }
  .client { color: var(--warning); background: var(--warning-soft); border-color: color-mix(in srgb, var(--warning) 30%, transparent); }
  .server { color: var(--danger); background: var(--danger-soft); border-color: color-mix(in srgb, var(--danger) 30%, transparent); }
</style>
