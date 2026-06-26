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

  const label = $derived((value ?? 'unknown').replace(/_/g, ' '));

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
    font-size: 10px;
    line-height: 1.5;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    padding: 1px 6px;
    border-radius: 4px;
    white-space: nowrap;
    background: color-mix(in srgb, var(--text-dim) 16%, transparent);
    color: var(--text-dim);
  }
  .pill.small {
    font-size: 9px;
    padding: 0 5px;
  }
  .pill.ok {
    background: color-mix(in srgb, var(--status-working, #28c840) 18%, transparent);
    color: var(--status-working, #28c840);
  }
  .pill.warn {
    background: color-mix(in srgb, #e0a000 22%, transparent);
    color: #e0a000;
  }
  .pill.bad {
    background: color-mix(in srgb, var(--status-exited, #ff5f57) 22%, transparent);
    color: var(--status-exited, #ff5f57);
  }
  .pill.info {
    background: color-mix(in srgb, #3b82f6 20%, transparent);
    color: #3b82f6;
  }
</style>
