<script lang="ts" module>
  // Sanitized markdown for prose + tool text. Goes through the vault renderer
  // (marked + allowlist sanitizer) — NOT lib/md.ts, which is unsanitized —
  // with the chat's own extensions (chatMarkdown.ts): file references that
  // open the side panel, PR / issue chips, IDE-style code blocks (line
  // numbers, whole-block highlighting, Wrap · Open · Copy, fold).
  import { ensureHljs } from '../../../lib/hl';
  import { createMdCache } from './mdCache';
  import { renderChatMarkdown } from './chatMarkdown';

  // Shared across every mounted block: a re-derived block with an unchanged
  // string is a Map lookup, not marked + hljs + DOMParser (mdCache.ts).
  // Code-block chrome is part of the cached string (codeBlocks.ts).
  const cache = createMdCache(renderChatMarkdown, {
    // Room for the whole mounted window (≤300 turns, a few prose blocks each).
    maxEntries: 1000,
    maxChars: 4_000_000,
    maxEntryChars: 512 * 1024,
  });
  // Perf-spec probe (e2e/desktop-conversation-perf.spec.ts): opt-in via a
  // `window.__ottoMdProbe` object installed before load; nothing otherwise.
  if (typeof window !== 'undefined') {
    const probe = (window as unknown as { __ottoMdProbe?: { cache?: unknown } }).__ottoMdProbe;
    if (probe) probe.cache = cache;
  }
  // hljs loads lazily; until it lands, fenced code renders escaped. Such
  // output is not cached, and the block re-renders once when hljs arrives
  // (this used to ride on the next live delta re-rendering everything).
  let hlLoaded = false;
  const FENCE = /```|~~~/;
  let diagramSeq = 0;
</script>

<script lang="ts">
  import { getContext } from 'svelte';
  import { ctxMenu } from '../../../lib/contextmenu.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { openExternal } from '../../../lib/external';
  import { runCodeAction, type CodeActionHost } from './codeBlocks';
  import { splitLocation } from './chatMarkdown';
  import { CONV_CTX, type ConvContext } from './context';
  import { ui } from '../../../lib/stores/ui.svelte';

  interface Props {
    md: string;
    /** Compact variant for tool-result text / notes. */
    small?: boolean;
  }
  let { md, small = false }: Props = $props();
  const ctx = getContext<ConvContext | undefined>(CONV_CTX);
  /** Hosts without the conversation view's link handling still open links. */
  const openUrl = (url: string, inApp = false): void => {
    if (ctx?.openUrl) ctx.openUrl(url, inApp);
    else void openExternal(url);
  };

  let hlReady = $state(hlLoaded);
  if (!hlLoaded) {
    void ensureHljs().then(() => {
      hlLoaded = true;
      hlReady = true;
    });
  }
  const html = $derived.by(() => {
    const fenced = FENCE.test(md);
    // Read hlReady only when it matters, so fence-free blocks never re-render.
    return cache.get(md, !fenced || hlReady);
  });

  // Mermaid / D2 fences render as diagrams (lazy libs, like the vault's reading
  // view); a bad diagram keeps its source with the parse error. Only blocks
  // whose html has one pay for the DOM query.
  let rootEl = $state<HTMLDivElement | null>(null);
  $effect(() => {
    const h = html;
    const host = rootEl;
    if (!host || !h.includes('diagram-block')) return;
    for (const el of Array.from(host.querySelectorAll<HTMLElement>('div.diagram-block:not([data-rendered])'))) {
      el.setAttribute('data-rendered', '1');
      const kind = el.dataset.diagram;
      const src = el.querySelector('pre.diagram-src')?.textContent ?? '';
      const id = `chat-diag-${++diagramSeq}`;
      const dark = ui.resolvedScheme === 'dark';
      void (kind === 'd2'
        ? import('../../canvas/d2').then((m) => m.renderD2(id, src, { dark, isStale: () => !el.isConnected }))
        : import('../../canvas/mermaid').then((m) => m.renderMermaid(id, src, { dark, isStale: () => !el.isConnected }))
      ).then(({ svg, error, stale }: { svg?: string; error?: string; stale?: boolean }) => {
        if (!el.isConnected || stale) return;
        if (svg) {
          el.innerHTML = svg;
          el.classList.add('diagram-ok');
        } else {
          const err = document.createElement('div');
          err.className = 'diagram-error';
          err.textContent = `Diagram error: ${error ?? 'unknown'}`;
          el.prepend(err);
        }
      });
    }
  });

  const host: CodeActionHost = {
    openCode: ({ text, lang, file }) =>
      ctx?.openPreview?.({ kind: 'code', title: file ?? (lang ? `${lang} snippet` : 'Snippet'), text, lang, file }),
    openFile: (path) => ctx?.openPreview?.({ kind: 'file', path }),
  };

  function copy(text: string, what: string): void {
    void navigator.clipboard?.writeText(text).then(
      () => toasts.info('Copied', what),
      () => toasts.error('Copy failed', what),
    );
  }

  /** A file reference, an issue chip or an external link — what it opens. */
  function activate(a: HTMLAnchorElement, alt: boolean): boolean {
    if (a.classList.contains('file-ref')) {
      const path = a.dataset.path ?? '';
      const loc = a.dataset.anchor ? splitLocation(`x:${a.dataset.anchor}`) : null;
      if (path) ctx?.openPreview?.({ kind: 'file', path, line: loc?.line ?? null });
      return true;
    }
    if (a.classList.contains('ref-chip') && a.dataset.raw) {
      const n = Number(a.dataset.raw);
      const url = ctx?.issueUrl?.(n) ?? null;
      if (url) openUrl(url, alt);
      else toasts.info(`#${n}`, 'No GitHub remote is known for this session’s repository');
      return true;
    }
    const href = a.getAttribute('href') ?? '';
    if (/^https?:/i.test(href)) {
      openUrl(href, alt);
      return true;
    }
    if (href.startsWith('#')) {
      // An in-note heading link: scroll this block, never the app's hash router.
      const id = decodeURIComponent(href.slice(1));
      (a.closest('.md')?.querySelector(`[id="${CSS.escape(id)}"]`) as HTMLElement | null)?.scrollIntoView({ block: 'start' });
      return true;
    }
    return false;
  }

  function onClick(e: MouseEvent): void {
    if (runCodeAction(e.target, host)) return;
    const a = e.target instanceof Element ? e.target.closest('a') : null;
    if (a && activate(a, e.altKey)) e.preventDefault();
  }
  function onKey(e: KeyboardEvent): void {
    if (e.key !== 'Enter' && e.key !== ' ') return;
    // Anchors WITH an href already turn ⏎ into a click.
    const a = e.target instanceof HTMLAnchorElement && !e.target.hasAttribute('href') ? e.target : null;
    if (a && activate(a, e.altKey)) e.preventDefault();
  }

  function onContext(e: MouseEvent): void {
    const a = e.target instanceof Element ? e.target.closest('a') : null;
    if (!a) return;
    if (a.classList.contains('file-ref')) {
      const path = a.dataset.path ?? '';
      e.preventDefault();
      ctxMenu.show(e, [
        { label: 'Preview', icon: 'eye', action: () => activate(a, false) },
        { label: 'Copy path', icon: 'copy', action: () => copy(path, path) },
      ]);
      return;
    }
    const href = a.getAttribute('href') ?? (a.dataset.raw ? ctx?.issueUrl?.(Number(a.dataset.raw)) : null) ?? '';
    if (!/^https?:/i.test(href)) return;
    e.preventDefault();
    ctxMenu.show(e, [
      { label: 'Open in browser', icon: 'external', action: () => openUrl(href, false) },
      { label: 'Open in Otto’s browser', icon: 'globe', hint: '⌥-click', action: () => openUrl(href, true) },
      { separator: true },
      { label: 'Copy link', icon: 'link', action: () => copy(href, href) },
    ]);
  }
</script>

<!-- Code-block buttons are real <button>s inside the html; one delegated
     handler drives them (keyboard activation bubbles as a click too). File
     references / issue chips are `a[tabindex][role=link]` (⏎ opens them). -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="md md-body" class:small dir="auto" bind:this={rootEl} onclick={onClick} onkeydown={onKey} oncontextmenu={onContext}>{@html html}</div>

<style>
  .md {
    font-size: var(--fs-m);
    line-height: 1.6;
    color: var(--text);
    overflow-wrap: anywhere;
    min-width: 0;
  }
  .md.small {
    font-size: var(--fs-s);
  }
  /* Prose keeps a readable measure; code, tables and diagrams use the width. */
  .md :global(:is(p, ul, ol, blockquote, h1, h2, h3, h4, h5, h6)) {
    max-width: var(--prose-measure, none);
  }
  .md :global(p) {
    margin: 0 0 0.65em;
  }
  .md :global(p:last-child) {
    margin-bottom: 0;
  }
  .md :global(strong) {
    font-weight: 650;
  }
  .md :global(pre) {
    background: var(--code-bg, var(--surface-2));
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    padding: 8px 10px;
    overflow-x: auto;
    font-family: var(--font-mono);
    font-size: var(--fs-s);
    line-height: 1.55;
    direction: ltr;
    text-align: start;
    margin: 0 0 0.6em;
  }
  /* ── Code blocks (codeBlocks.ts): an editor-like frame — a header strip
     (language · file · line count · Wrap · Open · Copy), a line-number gutter,
     a fold for long blocks. */
  .md :global(.code-block) {
    position: relative;
    margin: 0.5em 0 0.85em;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--code-bg, var(--surface-2));
    overflow: hidden;
    direction: ltr;
  }
  .md :global(.code-block pre) {
    margin: 0;
    border: 0;
    border-radius: 0;
    background: none;
    padding: 8px 0 10px;
  }
  .md :global(.code-block code) {
    display: block;
    min-width: max-content;
  }
  .md :global(.code-block.wrap code) {
    min-width: 0;
  }
  /* One block span per line: numbers from a counter (never copied or
     selected), wrapped lines hang under their own text. */
  .md :global(.code-block .cl) {
    display: block;
    min-height: 1.55em;
    padding-inline: 12px 14px;
    white-space: pre;
  }
  .md :global(.code-block.numbered code) {
    counter-reset: cl;
  }
  .md :global(.code-block.numbered .cl) {
    counter-increment: cl;
    padding-inline-start: calc(var(--gut, 3ch) + 22px);
    text-indent: calc(-1 * (var(--gut, 3ch) + 12px));
  }
  .md :global(.code-block.numbered .cl::before) {
    content: counter(cl);
    display: inline-block;
    width: var(--gut, 3ch);
    margin-inline-end: 12px;
    text-align: end;
    text-indent: 0;
    color: var(--text-dim);
    opacity: 0.65;
    user-select: none;
    -webkit-user-select: none;
  }
  .md :global(.code-block .cl:hover) {
    background: var(--hover);
  }
  .md :global(.code-block.wrap .cl) {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .md :global(.code-head) {
    display: flex;
    align-items: center;
    gap: 2px;
    min-height: 28px;
    padding-inline: 12px 4px;
    border-bottom: 1px solid var(--border);
    background: color-mix(in srgb, var(--text) 3%, transparent);
    font-size: var(--fs-xs);
    color: var(--text-dim);
    min-width: 0;
  }
  .md :global(.code-lang) {
    flex-shrink: 0;
    font-weight: 600;
    color: var(--text);
    letter-spacing: 0.01em;
  }
  .md :global(.code-file) {
    min-width: 0;
    margin-inline-start: 8px;
    padding: 1px 6px;
    border: 0;
    border-radius: var(--radius-s);
    background: none;
    color: var(--accent-text);
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    cursor: pointer;
  }
  .md :global(.code-file:hover) {
    background: var(--hover);
    text-decoration: underline;
  }
  .md :global(.code-sp) {
    flex: 1;
  }
  .md :global(.code-n) {
    flex-shrink: 0;
    margin-inline-end: 6px;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .md :global(.code-btn),
  .md :global(.code-more) {
    background: none;
    border: 0;
    border-radius: var(--radius-s);
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-xs);
    padding: 2px 7px;
    cursor: pointer;
    flex-shrink: 0;
  }
  .md :global(.code-btn:hover),
  .md :global(.code-more:hover) {
    color: var(--text);
    background: var(--hover);
  }
  .md :global(.code-btn[aria-pressed='true']) {
    color: var(--accent-text);
  }
  .md :global(.code-btn:focus-visible),
  .md :global(.code-more:focus-visible),
  .md :global(.code-file:focus-visible) {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  /* ~18 lines + padding; the rest behind "Show all N lines". */
  .md :global(.code-block.capped:not(.expanded) pre) {
    max-height: calc(18 * 1.55em + 18px);
    overflow-y: hidden;
    -webkit-mask-image: linear-gradient(to bottom, rgba(0, 0, 0, 1) 78%, rgba(0, 0, 0, 0));
    mask-image: linear-gradient(to bottom, rgba(0, 0, 0, 1) 78%, rgba(0, 0, 0, 0));
  }
  .md :global(.code-more) {
    display: block;
    width: 100%;
    border-top: 1px solid var(--border);
    border-radius: 0;
    padding: 5px 10px;
    text-align: center;
  }
  /* A narrow pane drops the line count and the file name first. */
  @container (max-width: 420px) {
    .md :global(.code-n),
    .md :global(.code-file) {
      display: none;
    }
  }
  /* File references: accent, mono, open the side panel. */
  .md :global(a.file-ref) {
    color: var(--accent-text);
    cursor: pointer;
    text-decoration: none;
    font-family: var(--font-mono);
    font-size: var(--fs-s);
    border-bottom: 1px dotted color-mix(in srgb, var(--accent) 55%, transparent);
    direction: ltr;
    unicode-bidi: isolate;
  }
  .md :global(a.file-ref.code) {
    border-bottom: 0;
    font-size: inherit;
  }
  .md :global(a.file-ref.code code) {
    color: var(--accent-text);
    background: var(--accent-soft);
    border-color: color-mix(in srgb, var(--accent) 22%, transparent);
  }
  .md :global(a.file-ref:hover),
  .md :global(a.file-ref:hover code) {
    text-decoration: underline;
  }
  /* PR / issue references as chips. */
  .md :global(a.ref-chip) {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 0 7px;
    border-radius: 99px;
    border: 1px solid color-mix(in srgb, var(--accent) 30%, var(--border));
    background: var(--accent-soft);
    color: var(--accent-text);
    font-size: var(--fs-s);
    font-weight: 600;
    line-height: 1.45;
    text-decoration: none;
    cursor: pointer;
    vertical-align: baseline;
  }
  .md :global(a.ref-chip.pr::before) {
    content: '';
    width: 7px;
    height: 7px;
    border-radius: 50%;
    border: 1.5px solid currentColor;
  }
  .md :global(a.ref-chip:hover) {
    background: color-mix(in srgb, var(--accent) 22%, transparent);
  }
  .md :global(a:focus-visible) {
    outline: 2px solid var(--accent-text);
    outline-offset: 1px;
    border-radius: var(--radius-s);
  }
  .md :global(.tag) {
    color: var(--accent-text);
  }
  .md :global(blockquote.callout) {
    padding: 6px 12px;
    border-radius: var(--radius-s);
    background: var(--info-soft);
    color: var(--text);
  }
  .md :global(.callout-title) {
    font-weight: 600;
    text-transform: capitalize;
  }
  /* `> [!note]` + a line break: the break after the marker is not content. */
  .md :global(.callout-title + p > br:first-child) {
    display: none;
  }
  .md :global(table) {
    border-collapse: separate;
    border-spacing: 0;
    display: block;
    max-width: 100%;
    width: max-content;
    overflow-x: auto;
    font-size: var(--fs-s);
    margin: 0.5em 0 0.8em;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
  }
  .md :global(th),
  .md :global(td) {
    border-bottom: 1px solid var(--border);
    border-inline-end: 1px solid var(--border);
    padding: 5px 10px;
    text-align: start;
    vertical-align: top;
  }
  .md :global(tr > :last-child) {
    border-inline-end: 0;
  }
  .md :global(tbody tr:last-child > *) {
    border-bottom: 0;
  }
  .md :global(tbody tr:nth-child(even)) {
    background: color-mix(in srgb, var(--text) 3%, transparent);
  }
  .md :global(th) {
    background: var(--surface-2);
    font-weight: 600;
  }
  .md :global(a[href^='http']:not(.ref-chip)) {
    text-decoration-color: color-mix(in srgb, var(--accent) 45%, transparent);
    text-underline-offset: 2px;
  }
  .md :global(a[href^='http']:not(.ref-chip)::after) {
    content: '↗';
    font-size: var(--fs-xs);
    margin-inline-start: 1px;
    opacity: 0.7;
  }
  .md :global(img) {
    max-width: 100%;
    border-radius: var(--radius-s);
  }
  .md :global(input[type='checkbox']) {
    margin-inline-end: 6px;
    vertical-align: middle;
  }
  .md :global(.diagram-block) {
    margin: 0.5em 0 0.85em;
    padding: 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--code-bg, var(--surface-2));
    overflow-x: auto;
  }
  .md :global(.diagram-block.diagram-ok) {
    display: flex;
    justify-content: center;
  }
  .md :global(.diagram-block svg) {
    max-width: 100%;
    height: auto;
  }
  .md :global(.diagram-block pre) {
    margin: 0;
    border: 0;
    padding: 0;
    background: none;
    white-space: pre;
  }
  .md :global(.diagram-error) {
    color: var(--danger);
    font-size: var(--fs-s);
    margin-bottom: 8px;
  }
</style>
