<script lang="ts">
  // The one tinted word-in-a-pill: a `.chip` (app.css) with a tone. Status,
  // kind and count pills render through it instead of hand-rolling a local
  // `.pill`. The tone comes from a domain mapper in lib/status.ts
  // (`badgeTone(runStatus(x).tone)`, `designStatus(x)`, …) or a module's model,
  // never picked ad hoc at the call site. Colour never carries meaning alone —
  // the label is always there; `dot` adds a leading dot and `live` pulses it
  // (live state only, stops under reduced motion).
  //
  //   <Badge tone="warn" label="In review" dot />
  //   <Badge tone="info" variant="outline">3 open</Badge>
  import type { Snippet } from 'svelte';
  import type { BadgeTone } from '../status';

  interface Props {
    tone?: BadgeTone;
    /** `soft` (default) = tinted fill; `outline` = border + coloured text only,
     *  for dense rows where several badges sit side by side. */
    variant?: 'soft' | 'outline';
    label?: string;
    /** Leading dot in the tone colour. */
    dot?: boolean;
    /** The dot pulses — only for state that is live right now. Implies `dot`. */
    live?: boolean;
    title?: string;
    testid?: string;
    /** The domain state key, exposed as `data-status` for tests and styling hooks. */
    status?: string;
    children?: Snippet;
  }
  let { tone = 'neutral', variant = 'soft', label, dot = false, live = false, title, testid, status, children }: Props = $props();
</script>

<span class="chip badge {tone}" class:outline={variant === 'outline'} {title} data-tone={tone} data-status={status} data-testid={testid}>
  {#if dot || live}<span class="badge-dot" class:live aria-hidden="true"></span>{/if}{#if children}{@render children()}{:else}<span class="badge-label">{label}</span>{/if}
</span>

<style>
  /* Never wider than its row: a long label (a user tag, a path) ends in an
     ellipsis instead of pushing the page sideways — set `title` for the full text. */
  .badge {
    flex-shrink: 0;
    max-width: 100%;
    min-width: 0;
    overflow: hidden;
  }
  .badge-label {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  /* `.chip` (app.css) supplies shape, type and the soft tone fills; outline
     keeps the tone's text + border and drops the fill. */
  .badge.outline {
    background: transparent;
  }
  .badge.outline.neutral {
    border-color: var(--border-strong);
  }
  .badge-dot {
    inline-size: 6px;
    block-size: 6px;
    border-radius: 50%;
    background: currentColor;
    flex: none;
  }
  .badge-dot.live {
    animation: otto-pulse 1.4s ease-in-out infinite;
  }
  @media (prefers-reduced-motion: reduce) {
    .badge-dot.live {
      animation: none;
    }
  }
</style>
