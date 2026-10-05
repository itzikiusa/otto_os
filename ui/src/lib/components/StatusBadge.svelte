<script lang="ts">
  // The shared status badge: a tone dot + a sentence-case word (colour never
  // carries meaning alone). Feed it a `StatusInfo` from lib/status.ts —
  // `runStatus(raw)` for runs/steps/jobs, `sessionState(...)` for sessions —
  // or a bare `tone` + `label`. `variant="pill"` (default) is the soft-tinted
  // pill; `variant="text"` is the same dot + word without the tint, for dense
  // list rows. Tones use the text-safe semantic tokens, so the label passes AA.
  // The pill IS the shared Badge (one pill system); only the text variant is
  // drawn here.
  import { badgeTone, type StatusInfo, type Tone } from '../status';
  import Badge from './Badge.svelte';

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

{#if variant === 'pill'}
  <Badge tone={badgeTone(t)} label={text} {dot} live={dot && live} status={status?.key} {testid} title={title ?? status?.hint} />
{:else}
  <span
    class="sbadge tone-{t}"
    data-status={status?.key}
    data-testid={testid}
    title={title ?? status?.hint}
  >
    {#if dot}<span class="sbadge-dot" class:live aria-hidden="true"></span>{/if}{text}
  </span>
{/if}

<style>
  .sbadge {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
    font-size: var(--fs-xs);
    font-weight: 500;
    line-height: 1.4;
    white-space: nowrap;
    color: var(--text-dim);
  }
  .sbadge-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: currentColor;
    flex: none;
  }
  .sbadge-dot.live {
    animation: otto-pulse 1.6s ease-in-out infinite;
  }
  @media (prefers-reduced-motion: reduce) {
    .sbadge-dot.live {
      animation: none;
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
</style>
