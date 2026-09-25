<script lang="ts">
  // One color-coded pill for every MCP status vocabulary (health, risk_label,
  // injection_risk, governed decision, approval status). Centralizing the tone
  // mapping here keeps every tab's badges consistent — green=safe, amber=caution,
  // red=danger, grey=neutral, blue=informational.
  interface Props {
    kind: 'health' | 'risk' | 'injection' | 'decision' | 'status' | 'direction';
    value: string | null | undefined;
    /** Render small (table cells). */
    small?: boolean;
  }
  let { kind, value, small = false }: Props = $props();

  // Sentence case ("Pending approval"), like every other status badge.
  const label = $derived.by(() => {
    const t = (value ?? 'unknown').replace(/_/g, ' ');
    return t.charAt(0).toUpperCase() + t.slice(1);
  });

  type Tone = 'ok' | 'warn' | 'bad' | 'neutral' | 'info';
  const tone: Tone = $derived.by((): Tone => {
    const v = value ?? 'unknown';
    switch (kind) {
      case 'health':
        return v === 'healthy' ? 'ok' : v === 'unhealthy' ? 'bad' : 'neutral';
      case 'risk':
        return v === 'read' ? 'ok' : v === 'write' ? 'warn' : v === 'dangerous' ? 'bad' : 'neutral';
      case 'injection':
        return v === 'low' ? 'ok' : v === 'medium' ? 'warn' : v === 'high' ? 'bad' : 'neutral';
      case 'decision':
        return v === 'allowed'
          ? 'ok'
          : v === 'approved' || v === 'dry_run'
            ? 'info'
            : v === 'pending_approval'
              ? 'warn'
              : 'bad';
      case 'status':
        return v === 'approved'
          ? 'ok'
          : v === 'denied'
            ? 'bad'
            : v === 'pending'
              ? 'warn'
              : v === 'consumed'
                ? 'info'
                : 'neutral';
      case 'direction':
        return v === 'inbound' ? 'info' : 'neutral';
      default:
        return 'neutral';
    }
  });
</script>

<span class="pill {tone}" class:small>{label}</span>

<style>
  .pill {
    display: inline-block;
    font-size: var(--fs-xs);
    line-height: 1.5;
    padding: 1px 7px;
    border-radius: 999px;
    white-space: nowrap;
    background: color-mix(in srgb, var(--text-dim) 16%, transparent);
    color: var(--text-dim);
  }
  .pill.small {
    /* Table cells: tighter box, but never below the 11px readable floor. */
    padding: 0 6px;
  }
  .pill.ok {
    background: var(--success-soft);
    color: var(--success);
  }
  .pill.warn {
    background: var(--warning-soft);
    color: var(--warning);
  }
  .pill.bad {
    background: var(--danger-soft);
    color: var(--danger);
  }
  .pill.info {
    background: var(--info-soft);
    color: var(--info);
  }
</style>
