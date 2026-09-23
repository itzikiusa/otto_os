<script lang="ts">
  // The first thing a new (empty) site shows: six starter templates, each a
  // LIVE miniature rendered with the site's brand theme, plus "Start blank".
  // Picking one only edits the working copy — nothing is saved until Save.
  import Icon from '../../../lib/components/Icon.svelte';
  import { renderPageBody } from './engine/render';
  import { SITE_TEMPLATES } from './engine/templates';
  import { themeStyle, type Theme } from './engine/theme';

  interface Props {
    theme: Theme;
    title: string;
    readonly: boolean;
    onpick: (templateId: string | null) => void;
  }
  let { theme, title, readonly, onpick }: Props = $props();

  const style = $derived(themeStyle(theme));
  const previews = $derived(
    SITE_TEMPLATES.map((t) => {
      const d = t.build('');
      const page = { ...d.pages[0], sections: d.pages[0].sections.slice(0, 4) };
      return { t, html: renderPageBody(page, { theme, editable: false }) };
    }),
  );
</script>

<div class="picker" data-testid="site-templates">
  <header>
    <span class="badge"><Icon name="layout" size={16} /></span>
    <div>
      <h2>Start your site</h2>
      <p>Pick a starting point — every template uses your brand tokens, real copy and motion that respects reduced-motion. You can swap any section later.</p>
    </div>
  </header>
  <div class="grid">
    {#each previews as p (p.t.id)}
      <button class="tpl" disabled={readonly} onclick={() => onpick(p.t.id)} data-testid="site-template" data-template={p.t.id}>
        <span class="shot" aria-hidden="true"><span class="mini os-site os-still" {style}>{@html p.html}</span></span>
        <span class="meta"><strong>{p.t.name}</strong><span>{p.t.description}</span></span>
      </button>
    {/each}
    <button class="tpl blank" disabled={readonly} onclick={() => onpick(null)} data-testid="site-template-blank">
      <span class="shot empty" aria-hidden="true"><Icon name="plus" size={16} /></span>
      <span class="meta"><strong>Start blank</strong><span>One empty page — add blocks from the library</span></span>
    </button>
  </div>
</div>

<style>
  .picker {
    height: 100%;
    overflow-y: auto;
    padding: 28px 32px 40px;
    background: var(--bg);
  }
  header {
    display: flex;
    gap: 14px;
    align-items: flex-start;
    max-width: 760px;
    margin-block-end: 22px;
  }
  .badge {
    display: grid;
    place-items: center;
    flex: none;
    width: 36px;
    height: 36px;
    border-radius: var(--radius-m);
    background: var(--studio-site);
    color: var(--studio-glyph);
  }
  h2 {
    margin: 0 0 4px;
    font-size: var(--fs-xl);
    font-weight: 600;
  }
  header p {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--fs-m);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
    gap: 16px;
  }
  .tpl {
    display: flex;
    flex-direction: column;
    padding: 0;
    overflow: hidden;
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    background: var(--surface);
    color: var(--text);
    text-align: start;
    cursor: pointer;
    transition: border-color 130ms ease-out, box-shadow 130ms ease-out, transform 130ms ease-out;
  }
  .tpl:hover:not(:disabled) {
    border-color: var(--accent);
    box-shadow: var(--shadow);
    transform: translateY(-2px);
  }
  .tpl:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .tpl:disabled {
    cursor: default;
    opacity: 0.6;
  }
  .shot {
    position: relative;
    display: block;
    height: 170px;
    overflow: hidden;
    background: var(--surface-2);
    border-block-end: 1px solid var(--border);
  }
  .mini {
    position: absolute;
    inset-block-start: 0;
    inset-inline-start: 0;
    width: 1280px;
    transform: scale(0.215);
    transform-origin: 0 0;
    pointer-events: none;
  }
  :global([dir='rtl']) .mini {
    inset-inline-start: auto;
    inset-inline-end: 0;
    transform-origin: 100% 0;
  }
  .shot.empty {
    display: grid;
    place-items: center;
    color: var(--text-dim);
    background: repeating-linear-gradient(45deg, var(--surface) 0 10px, var(--surface-2) 10px 20px);
  }
  .meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 10px 12px 12px;
    font-size: var(--fs-s);
  }
  .meta strong {
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .meta span {
    color: var(--text-dim);
  }
  @media (prefers-reduced-motion: reduce) {
    .tpl {
      transition: none;
    }
    .tpl:hover:not(:disabled) {
      transform: none;
    }
  }
</style>
