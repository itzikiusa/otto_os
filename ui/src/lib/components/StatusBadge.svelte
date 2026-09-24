<script lang="ts">
  // The shared status badge: a tone dot + a sentence-case word (colour never
  // carries meaning alone). Feed it a `StatusInfo` from lib/status.ts —
  // `runStatus(raw)` for runs/steps/jobs, `sessionState(...)` for sessions —
  // or a bare `tone` + `label`. `variant="pill"` (default) is the soft-tinted
  // pill; `variant="text"` is the same dot + word without the tint, for dense
  // list rows. Tones use the text-safe semantic tokens, so the label passes AA.
  import type { StatusInfo, Tone } from '../status';

  interface Props {
    status?: StatusInfo;
    tone?: Tone;
    label?: string;
    variant?: 'pill' | 'text';
    /** Hide the dot (e.g. the row already draws one). */
    dot?: boolean;
    title?: string;
    testid?: string;
  }
  let { status, tone, label, variant = 'pill', dot = true, title, testid }: Props = $props();

  const t = $derived<Tone>(tone ?? status?.tone ?? 'neutral');
  const text = $derived(label ?? status?.label ?? '');
  const live = $derived(status?.live === true);
</script>

<span
  class="sbadge tone-{t} {variant}"
  data-status={status?.key}
  data-testid={testid}
  title={title ?? status?.hint}
>
  {#if dot}<span class="sbadge-dot" class:live aria-hidden="true"></span>{/if}{text}
</span>

<style>
  .sbadge {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    flex-shrink: 0;
    font-size: var(--fs-xs);
    font-weight: 500;
    line-height: 1.4;
    white-space: nowrap;
    color: var(--text-dim);
  }
  .sbadge.pill {
    height: 18px;
    padding: 0 7px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: var(--surface-2);
  }
  .sbadge-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: currentColor;
    flex: none;
  }
  .sbadge-dot.live {
    animation: sbadge-pulse 1.6s ease-in-out infinite;
  }
  @media (prefers-reduced-motion: reduce) {
    .sbadge-dot.live {
      animation: none;
    }
  }
  @keyframes sbadge-pulse {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.4;
    }
  }
  .tone-info {
    color: var(--info);
  }
  .tone-success {
    color: var(--success);
  }
  .tone-warning {
    color: var(--warning);
  }
  .tone-danger {
    color: var(--danger);
  }
  .pill.tone-info {
    background: var(--info-soft);
    border-color: color-mix(in srgb, var(--info) 30%, transparent);
  }
  .pill.tone-success {
    background: var(--success-soft);
    border-color: color-mix(in srgb, var(--success) 30%, transparent);
  }
  .pill.tone-warning {
    background: var(--warning-soft);
    border-color: color-mix(in srgb, var(--warning) 30%, transparent);
  }
  .pill.tone-danger {
    background: var(--danger-soft);
    border-color: color-mix(in srgb, var(--danger) 30%, transparent);
  }
</style>
