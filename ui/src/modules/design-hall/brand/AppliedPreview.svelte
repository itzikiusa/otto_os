<script lang="ts">
  // "Applied to": three small renders of the kit — a landing hero (Site), a
  // social tile (Graphics) and a 3D card swatch (3D) — that re-tint as tokens
  // change. They are USER CONTENT previews (the kit's own colours and fonts),
  // driven entirely by `--pv-*` properties set from the document; nothing here
  // uses the app's accent. Decorative: the real artifacts are in "Used in".
  import type { BrandDoc } from '../../../lib/api/types';
  import StudioBadge from '../StudioBadge.svelte';
  import { brandPalette, fontTokens, onColor, pxTokens, typeStyles } from './tokens';

  interface Props {
    doc: BrandDoc;
  }
  let { doc }: Props = $props();

  const pal = $derived(brandPalette(doc));
  const fonts = $derived(fontTokens(doc));
  const display = $derived(fonts.find((f) => f.name === 'display')?.stack ?? fonts[0]?.stack ?? 'system-ui, sans-serif');
  const body = $derived(fonts.find((f) => f.name === 'body')?.stack ?? display);
  const displayWeight = $derived(String(typeStyles(doc).find((t) => t.name === 'display')?.weight ?? 800));
  const radius = $derived.by(() => {
    const r = pxTokens(doc, 'radius');
    const pick = r.find((x) => ['card', 'md', 'lg'].includes(x.name)) ?? r.find((x) => x.px < 100) ?? r[0];
    return `${Math.min(pick?.px ?? 12, 28)}px`;
  });
  const onPrimary = $derived(onColor(pal.primary, pal.ink));
  const eyebrow = $derived((doc.name || 'Brand').replace(/\s*brand kit\s*$/i, '').trim() || 'Brand');
  const headline = $derived(firstLine(doc.voice?.do) ?? 'Every purchase moves you up.');
  const cta = $derived(shortCta(doc.voice?.do) ?? 'Join free');

  function firstLine(list: string[] | undefined): string | null {
    const s = list?.find((x) => x.trim().length > 12);
    return s ? s.trim().replace(/[.!]+$/, '') + '.' : null;
  }
  function shortCta(list: string[] | undefined): string | null {
    return list?.find((x) => x.trim().length > 0 && x.trim().length <= 14)?.trim() ?? null;
  }
</script>

<div
  class="applied"
  data-testid="brand-applied"
  style:--pv-primary={pal.primary}
  style:--pv-accent={pal.accent}
  style:--pv-ink={pal.ink}
  style:--pv-surface={pal.surface}
  style:--pv-surface-alt={pal.surfaceAlt}
  style:--pv-on-primary={onPrimary}
  style:--pv-font-display={display}
  style:--pv-font-body={body}
  style:--pv-display-weight={displayWeight}
  style:--pv-radius={radius}
>
  <div class="head">
    <span class="section-title">Applied to</span>
    <span class="dim">live preview</span>
  </div>

  <figure class="ap">
    <div class="hero" aria-hidden="true">
      <div class="copy">
        <span class="eb">New · {eyebrow}</span>
        <b>{headline}</b>
        <span class="cta">{cta}</span>
      </div>
      <div class="blob"></div>
      <div class="card3"></div>
    </div>
    <figcaption><StudioBadge studio="site" /> Landing hero</figcaption>
  </figure>

  <div class="row2">
    <figure class="ap">
      <div class="social" aria-hidden="true">
        <span class="eb">{eyebrow}</span>
        <b>2× points this weekend</b>
        <div class="blob"></div>
        <div class="card3"></div>
      </div>
      <figcaption><StudioBadge studio="graphics" /> Social tile</figcaption>
    </figure>
    <figure class="ap">
      <div class="three" aria-hidden="true">
        <div class="card-wrap">
          <div class="card">
            <span class="name">{eyebrow}</span>
            <span class="chip3"></span>
          </div>
        </div>
      </div>
      <figcaption><StudioBadge studio="3d" /> 3D card swatch</figcaption>
    </figure>
  </div>
</div>

<style>
  .applied {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 14px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
  }
  .head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 8px;
  }
  .dim {
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  figure {
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }
  figcaption {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
    color: var(--text);
  }
  .row2 {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
  }

  /* ── Landing hero (user content: the kit's own colours) ── */
  .hero {
    position: relative;
    overflow: hidden;
    aspect-ratio: 16 / 9;
    border-radius: var(--radius-m);
    background: var(--pv-surface-alt);
    border: 1px solid var(--border);
    transition: background 160ms ease-out;
  }
  .hero .copy {
    position: absolute;
    inset-inline-start: 14px;
    inset-block-start: 50%;
    transform: translateY(-50%);
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-width: 52%;
    z-index: 1;
  }
  .eb {
    font: 700 10px/1.2 var(--pv-font-body);
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--pv-primary);
  }
  .hero b {
    font-family: var(--pv-font-display);
    font-weight: var(--pv-display-weight);
    font-size: 17px;
    line-height: 1.08;
    letter-spacing: -0.02em;
    color: var(--pv-ink);
  }
  .cta {
    align-self: flex-start;
    font: 600 10px/1 var(--pv-font-body);
    padding: 6px 9px;
    border-radius: calc(var(--pv-radius) / 2);
    background: var(--pv-primary);
    color: var(--pv-on-primary);
  }
  .hero .blob {
    position: absolute;
    inset-inline-end: 8%;
    inset-block-start: 18%;
    width: 38%;
    aspect-ratio: 1;
    border-radius: 50%;
    background: var(--pv-accent);
  }
  .hero .card3 {
    position: absolute;
    inset-inline-end: 6%;
    inset-block-start: 26%;
    width: 42%;
    aspect-ratio: 1.6;
    border-radius: var(--pv-radius);
    background: linear-gradient(135deg, var(--pv-primary), color-mix(in srgb, var(--pv-primary) 62%, var(--pv-ink)));
    transform: rotate(-9deg);
    box-shadow: 0 10px 24px rgba(0, 0, 0, 0.22);
  }

  /* ── Social tile ── */
  .social {
    position: relative;
    overflow: hidden;
    aspect-ratio: 1;
    border-radius: var(--radius-m);
    background: var(--pv-primary);
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .social .eb {
    color: var(--pv-on-primary);
    opacity: 0.85;
    position: relative;
    z-index: 1;
  }
  .social b {
    font-family: var(--pv-font-display);
    font-weight: var(--pv-display-weight);
    font-size: 14px;
    line-height: 1.1;
    color: var(--pv-on-primary);
    max-width: 80%;
    position: relative;
    z-index: 1;
  }
  .social .blob {
    position: absolute;
    inset-inline-end: -12%;
    inset-block-start: -10%;
    width: 42%;
    aspect-ratio: 1;
    border-radius: 50%;
    background: var(--pv-accent);
  }
  .social .card3 {
    position: absolute;
    inset-inline-end: -8%;
    inset-block-end: 12%;
    width: 60%;
    aspect-ratio: 1.6;
    border-radius: var(--pv-radius);
    background: color-mix(in srgb, var(--pv-primary) 70%, var(--pv-ink));
    transform: rotate(-10deg);
    box-shadow: 0 8px 18px rgba(0, 0, 0, 0.25);
  }

  /* ── 3D card swatch ── */
  .three {
    aspect-ratio: 1;
    border-radius: var(--radius-m);
    background: radial-gradient(circle at 50% 40%, var(--pv-surface), var(--pv-surface-alt));
    border: 1px solid var(--border);
    display: grid;
    place-items: center;
    perspective: 420px;
  }
  .card-wrap {
    width: 76%;
    transform: rotateX(14deg) rotateY(-18deg);
    transform-style: preserve-3d;
  }
  .card {
    aspect-ratio: 1.6;
    border-radius: calc(var(--pv-radius) * 0.7);
    background: linear-gradient(135deg, color-mix(in srgb, var(--pv-primary) 78%, var(--pv-surface)), var(--pv-primary) 55%, color-mix(in srgb, var(--pv-primary) 60%, var(--pv-ink)));
    box-shadow: 0 14px 22px rgba(0, 0, 0, 0.25);
    padding: 8px;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
  }
  .card .name {
    font: 700 10px/1 var(--pv-font-display);
    color: var(--pv-on-primary);
  }
  .chip3 {
    width: 18%;
    aspect-ratio: 1.3;
    border-radius: calc(var(--pv-radius) / 4);
    background: var(--pv-accent);
  }
  @media (prefers-reduced-motion: reduce) {
    .hero {
      transition: none;
    }
  }
</style>
