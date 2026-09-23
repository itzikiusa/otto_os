<script lang="ts">
  // Full-size preview of the site without editor chrome: the working copy
  // rendered as the standalone document the export ships (or, for a local
  // publish, the HTML the daemon served). It runs in a frame with
  // `sandbox="allow-same-origin"` — never scripts — so the page can't run code,
  // while the studio can follow the site's own page links (tiers.html → Tiers).
  import Modal from '../../../lib/components/Modal.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { documentCss, pageFileName, renderDocument, themeCss, type EmbedInfo } from './engine/render';
  import type { Theme } from './engine/theme';
  import type { SiteDoc } from './engine/types';
  import SITE_CSS from './engine/site.css?raw';
  import { DEVICE_WIDTH, type SiteDevice } from './SiteCanvas.svelte';

  interface Props {
    doc: SiteDoc;
    theme: Theme;
    pageId: string;
    embed: (uri: string) => EmbedInfo | null;
    asset: (uri: string) => string | null;
    /** Server-rendered HTML per page id (a local publish) — shown instead of the working copy. */
    served?: Record<string, string> | null;
    servedUrl?: string | null;
    onclose: () => void;
  }
  let { doc, theme, pageId: initialPage, embed, asset, served = null, servedUrl = null, onclose }: Props = $props();

  let pageId = $state('');
  $effect.pre(() => {
    if (!pageId) pageId = initialPage || doc.pages[0]?.id || '';
  });
  let device = $state<SiteDevice | 'fill'>('fill');
  const page = $derived(doc.pages.find((p) => p.id === pageId) ?? doc.pages[0]);
  const css = $derived(themeCss(theme) + documentCss(theme) + SITE_CSS);
  const srcdoc = $derived.by(() => {
    if (!page) return '';
    if (served) return served[page.id] ?? '';
    return renderDocument(
      doc,
      page,
      {
        theme,
        embed,
        asset,
        homeHref: doc.pages[0] ? pageFileName(doc, doc.pages[0]) : '#',
        pageHref: (id) => {
          const p = doc.pages.find((x) => x.id === id);
          return p ? pageFileName(doc, p) : '#';
        },
      },
      css,
    );
  });

  let frame = $state<HTMLIFrameElement | null>(null);
  let winH = $state(800);
  $effect(() => {
    const onResize = () => (winH = window.innerHeight);
    onResize();
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  });

  /** Follow the site's own page links; anything else stays put (no navigation inside the preview). */
  function wire(): void {
    const d = frame?.contentDocument;
    if (!d) return;
    d.addEventListener('click', (e) => {
      const a = (e.target as Element | null)?.closest?.('a[href]') as HTMLAnchorElement | null;
      if (!a) return;
      const href = a.getAttribute('href') ?? '';
      if (href.startsWith('#')) return;
      e.preventDefault();
      const file = href.split(/[?#]/)[0];
      const target = doc.pages.find((p) => pageFileName(doc, p) === file || (served && p.slug && href.includes(`/preview/${p.slug}`)));
      if (target) pageId = target.id;
    });
  }
  const width = $derived(device === 'fill' ? '100%' : `${DEVICE_WIDTH[device]}px`);
</script>

<Modal title="Preview" width={1480} {onclose}>
  <div class="bar">
    {#if doc.pages.length > 1}
      <select class="input" bind:value={pageId} aria-label="Page">
        {#each doc.pages as p (p.id)}<option value={p.id}>{p.title || 'Untitled page'}</option>{/each}
      </select>
    {/if}
    <div class="segmented" role="group" aria-label="Width">
      <button class:active={device === 'fill'} aria-pressed={device === 'fill'} onclick={() => (device = 'fill')}>Fill</button>
      <button class:active={device === 'desktop'} aria-pressed={device === 'desktop'} onclick={() => (device = 'desktop')}>Desktop</button>
      <button class:active={device === 'tablet'} aria-pressed={device === 'tablet'} onclick={() => (device = 'tablet')}>Tablet</button>
      <button class:active={device === 'mobile'} aria-pressed={device === 'mobile'} onclick={() => (device = 'mobile')}>Mobile</button>
    </div>
    <span class="grow"></span>
    {#if servedUrl}
      <span class="served" title={servedUrl}><Icon name="shield" size={12} /> Served by your Otto daemon · only reachable from this Mac</span>
    {:else}
      <span class="served"><Icon name="info" size={12} /> Working copy, including unsaved edits · Esc to close</span>
    {/if}
  </div>
  <div class="well" style:height={`${Math.max(320, winH - 220)}px`}>
    <iframe
      bind:this={frame}
      title={`Preview of ${page?.title ?? 'the site'}`}
      sandbox="allow-same-origin"
      {srcdoc}
      style:width
      onload={wire}
      data-testid="site-preview-frame"
    ></iframe>
  </div>
</Modal>

<style>
  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    margin-block-end: 10px;
  }
  .grow {
    flex: 1;
  }
  .served {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .well {
    display: flex;
    justify-content: center;
    overflow: auto;
    border-radius: var(--radius-m);
    background: var(--term-bg);
    border: 1px solid var(--border);
  }
  iframe {
    max-width: 100%;
    height: 100%;
    border: 0;
    background: var(--surface);
    transition: width 200ms ease-out;
  }
  @media (prefers-reduced-motion: reduce) {
    iframe {
      transition: none;
    }
  }
</style>
