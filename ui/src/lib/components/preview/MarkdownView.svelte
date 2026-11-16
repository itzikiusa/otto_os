<script lang="ts">
  // A Markdown file rendered with the vault reading-view renderer
  // (lib/render/markdown.ts: marked GFM — tables, task lists — the app's fence
  // highlighting, heading ids, the allowlist sanitizer), so a README reads like
  // a vault note. GitHub semantics on top: single newlines don't break lines,
  // front matter is hidden, ```mermaid fences render as diagrams, and links and
  // images resolve against the file's own folder:
  //   • a relative link to another file → `onOpenLink(absPath, anchor)`;
  //   • `#anchor` → scrolls this preview; http(s)/mailto → the system browser;
  //   • a relative image → `mediaUrl(absPath)` (the host's authenticated file
  //     route); blob: URLs it returns are revoked on re-render and unmount.
  import { onDestroy, onMount, untrack } from 'svelte';
  import { ensureHljs } from '../../hl';
  import { openExternal } from '../../external';
  import { hydrateDiagramBlocks } from '../../render/diagramBlocks';
  import { renderNote, slugifyHeading, stripFrontmatter } from '../../render/markdown';
  import { ui } from '../../stores/ui.svelte';
  import { resolvePreviewHref } from '../filePreviewKind';

  interface Props {
    path: string;
    content: string;
    root?: string | null;
    mediaUrl?: ((absPath: string) => string | Promise<string>) | null;
    onOpenLink?: ((absPath: string, anchor: string | null) => void) | null;
    /** Split view: keep this heading (0-based, -1 = top) at the top of the pane. */
    headingOrdinal?: number | null;
  }
  let { path, content, root = null, mediaUrl = null, onOpenLink = null, headingOrdinal = null }: Props = $props();

  let host: HTMLDivElement | undefined = $state();
  let hlReady = $state(false);
  onMount(() => void ensureHljs().then(() => (hlReady = true)));

  const html = $derived.by(() => {
    void hlReady; // re-render once the highlighter landed
    return renderNote(stripFrontmatter(content), {
      resolve: (raw) => resolvePreviewHref(path, raw, root)?.path ?? null,
      assetUrl: () => null,
      lazyAssets: true,
      use: [{ breaks: false }],
    });
  });

  let blobUrls: string[] = [];
  function revokeAll(): void {
    for (const u of blobUrls) URL.revokeObjectURL(u);
    blobUrls = [];
  }
  onDestroy(revokeAll);

  $effect(() => {
    void html;
    const el = host;
    if (!el) return;
    const dark = ui.resolvedScheme === 'dark';
    const load = mediaUrl;
    untrack(() => {
      revokeAll();
      for (const img of Array.from(el.querySelectorAll<HTMLImageElement>('img[data-asset]'))) {
        const ref = img.getAttribute('data-asset') ?? '';
        if (/^data:image\//i.test(ref)) {
          img.src = ref;
          continue;
        }
        if (!ref.startsWith('/') || !load) {
          img.alt = `${img.alt || 'image'} (not available here)`;
          continue;
        }
        void Promise.resolve()
          .then(() => load(ref))
          .then(
            (url) => {
              const owned = url.startsWith('blob:');
              if (!img.isConnected) return owned ? URL.revokeObjectURL(url) : undefined;
              if (owned) blobUrls.push(url);
              img.src = url;
            },
            () => (img.alt = `${img.alt || 'image'} (couldn’t load)`),
          );
      }
      hydrateDiagramBlocks(el, { dark, idPrefix: 'fp-md-diag' });
    });
  });

  /** The scroll container this preview sits in (FilePreview's body). */
  function scroller(): HTMLElement | null {
    return host?.closest<HTMLElement>('[data-preview-scroll]') ?? null;
  }
  function scrollToEl(target: Element | null): void {
    const sc = scroller();
    if (!sc) return;
    if (!target) {
      sc.scrollTop = 0;
      return;
    }
    sc.scrollTop += target.getBoundingClientRect().top - sc.getBoundingClientRect().top - 8;
  }
  function scrollToAnchor(anchor: string): void {
    const id = `h-${slugifyHeading(anchor)}`;
    const t = host?.querySelector(`[id="${CSS.escape(id)}"]`) ?? host?.querySelector(`[id="${CSS.escape(anchor)}"]`) ?? null;
    if (t) scrollToEl(t);
  }

  $effect(() => {
    const n = headingOrdinal;
    void html;
    if (n === null || !host) return;
    const el = host;
    // After the DOM for this html is in place.
    requestAnimationFrame(() => scrollToEl(n < 0 ? null : (el.querySelectorAll('h1,h2,h3,h4,h5,h6')[n] ?? null)));
  });

  $effect(() => {
    const el = host;
    if (!el) return;
    const onClick = (e: MouseEvent): void => {
      const a = (e.target as Element | null)?.closest?.('a');
      if (!a || !el.contains(a)) return;
      e.preventDefault();
      const dst = a.getAttribute('data-path');
      const anchor = a.getAttribute('data-anchor');
      if (dst) {
        if (dst === path && anchor) scrollToAnchor(anchor);
        else onOpenLink?.(dst, anchor);
        return;
      }
      if (a.hasAttribute('data-unresolved')) return;
      const href = a.getAttribute('href') ?? '';
      if (href.startsWith('#')) {
        let anchorText = href.slice(1);
        try {
          anchorText = decodeURIComponent(anchorText);
        } catch {
          /* a stray % — use it as written */
        }
        if (anchorText) scrollToAnchor(anchorText);
        return;
      }
      if (/^(https?:|mailto:)/i.test(href)) void openExternal(href);
    };
    el.addEventListener('click', onClick);
    return () => el.removeEventListener('click', onClick);
  });
</script>

<!-- html is sanitised by renderNote (allowlist sanitizer). -->
<div class="fp-md" dir="auto" bind:this={host} data-testid="file-preview-markdown">{@html html}</div>

<style>
  .fp-md {
    padding: 16px 20px;
    font-size: var(--fs-m);
    line-height: 1.6;
    color: var(--text);
    max-inline-size: 900px;
    overflow-wrap: anywhere;
  }
  .fp-md :global(h1) { font-size: var(--fs-2xl); margin: 0.6em 0 0.4em; padding-block-end: 0.2em; border-block-end: 1px solid var(--border); }
  .fp-md :global(h2) { font-size: var(--fs-xl); margin: 0.9em 0 0.4em; padding-block-end: 0.2em; border-block-end: 1px solid var(--border); }
  .fp-md :global(h3) { font-size: var(--fs-l); margin: 0.8em 0 0.3em; }
  .fp-md :global(h4), .fp-md :global(h5), .fp-md :global(h6) { font-size: var(--fs-m); margin: 0.8em 0 0.3em; }
  .fp-md :global(p) { margin: 0.5em 0; }
  .fp-md :global(a) { color: var(--accent-text); }
  .fp-md :global(a.unresolved) { color: var(--text-dim); }
  .fp-md :global(code) {
    font-family: var(--font-mono);
    font-size: var(--fs-s);
    background: var(--surface-2);
    padding: 2px 4px;
    border-radius: var(--radius-s);
  }
  .fp-md :global(pre) {
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    padding: 10px 12px;
    overflow-x: auto;
  }
  .fp-md :global(pre code) { background: none; padding: 0; }
  .fp-md :global(blockquote) {
    margin: 0.6em 0;
    padding-inline-start: 12px;
    border-inline-start: 3px solid var(--border-strong);
    color: var(--text-dim);
  }
  .fp-md :global(table) { border-collapse: collapse; display: block; overflow-x: auto; max-inline-size: 100%; }
  .fp-md :global(th), .fp-md :global(td) {
    border: 1px solid var(--border);
    padding: 4px 8px;
    text-align: start;
  }
  .fp-md :global(th) { background: var(--surface-2); }
  .fp-md :global(ul:has(> li > input[type='checkbox'])) { padding-inline-start: 4px; }
  .fp-md :global(li:has(> input[type='checkbox'])) { list-style: none; }
  .fp-md :global(input[type='checkbox']) { margin-inline-end: 6px; }
  .fp-md :global(img) { max-inline-size: 100%; height: auto; }
  .fp-md :global(hr) { border: 0; border-block-start: 1px solid var(--border); margin: 1em 0; }
  .fp-md :global(div.diagram-block) {
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    padding: 14px;
    margin: 10px 0;
    background: var(--surface-2);
    overflow-x: auto;
  }
  .fp-md :global(div.diagram-block.diagram-ok) { display: flex; justify-content: center; }
  .fp-md :global(div.diagram-block svg) { max-width: 100%; height: auto; }
  .fp-md :global(div.diagram-error) { color: var(--danger); font-size: var(--fs-s); margin-block-end: 8px; }
  .fp-md :global(pre.diagram-src) { margin: 0; font-size: var(--fs-s); }
</style>
