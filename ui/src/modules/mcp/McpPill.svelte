<script lang="ts">
  // One color-coded pill for every MCP status vocabulary (health, risk_label,
  // injection_risk, governed decision, approval status). Centralizing the tone
  // mapping here keeps every tab's badges consistent — green=safe, amber=caution,
  // red=danger, grey=neutral, blue=informational. Renders through the shared
  // Badge; this file only owns the MCP vocabulary → tone mapping.
  import Badge from '../../lib/components/Badge.svelte';
  import { sentenceCase } from '../../lib/labels';

  interface Props {
    kind: 'health' | 'risk' | 'injection' | 'decision' | 'status' | 'direction';
    value: string | null | undefined;
    /** Dense table cells: the outline badge, so several side by side stay quiet. */
    small?: boolean;
  }
  let { kind, value, small = false }: Props = $props();

  // Sentence case ("Pending approval"), like every other status badge.
  const label = $derived(sentenceCase(value ?? 'unknown'));

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
          : v === 'auto_approved'
            ? 'warn' // ran without a person — visible, never silent
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

<Badge {tone} label={label} variant={small ? 'outline' : 'soft'} />
