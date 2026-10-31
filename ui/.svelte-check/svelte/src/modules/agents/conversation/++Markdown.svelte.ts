///<reference types="svelte" />
;
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
;;

import { getContext } from 'svelte';
import { ctxMenu } from '../../../lib/contextmenu.svelte';
import { toasts } from '../../../lib/toast.svelte';
import { copyText } from '../../../lib/clipboard';
import { openExternal } from '../../../lib/external';
import { runCodeAction, type CodeActionHost } from './codeBlocks';
import { splitLocation } from './chatMarkdown';
import { CONV_CTX, type ConvContext } from './context';
import { ui } from '../../../lib/stores/ui.svelte';
;

  interface Props {
    md: string;
    /** Compact variant for tool-result text / notes. */
    small?: boolean;
  };function $$render() {

  
  
  
  
  
  
  
  
  

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
        : import('../../../lib/render/mermaid').then((m) => m.renderMermaid(id, src, { dark, isStale: () => !el.isConnected }))
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

  /** Copy a path / link from a menu row. Through lib/clipboard so an origin
   *  without `navigator.clipboard` (LAN http, the self-signed listener) still
   *  copies, or at least says so — never a silent no-op. */
  async function copy(text: string, what: string): Promise<void> {
    if (await copyText(text)) toasts.info('Copied', what);
    else toasts.error('Couldn’t copy', `The clipboard isn’t available here. Select the text and copy it manually: ${what}`);
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
;
async () => {





 { const $$_div0 = svelteHTML.createElement("div", {          "class":`md md-body`,"dir":`auto`,"onclick":onClick,"onkeydown":onKey,"oncontextmenu":onContext,});small;rootEl = $$_div0; html; }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const Markdown__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type Markdown__SvelteComponent_ = ReturnType<typeof Markdown__SvelteComponent_>;
/*Ωignore_endΩ*/export default Markdown__SvelteComponent_;