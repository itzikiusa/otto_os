///<reference types="svelte" />
;
  import { scrollBehavior } from '../../lib/motion';
  import { authedBlobUrl as fetchBlobUrl } from '../../lib/api/client';
  import { assetPath as vaultAssetPath } from '../../lib/api/vault';

  /** Rendered reading-view HTML per (vault, path, hash, resolved links). */
  const renderCache = new Map<string, string>();
  const RENDER_CACHE_MAX = 8;

  /** Attachment blob URLs, shared across note switches (LRU; the evicted URL
   *  is revoked — an <img> that already decoded it keeps showing). */
  const assetUrls = new Map<string, Promise<string>>();
  const ASSET_CACHE_MAX = 200;

  function assetBlobUrl(wsId: string, vaultId: number, path: string): Promise<string> {
    const key = `${wsId}:${vaultId}:${path}`;
    const hit = assetUrls.get(key);
    if (hit) {
      assetUrls.delete(key);
      assetUrls.set(key, hit);
      return hit;
    }
    const p = fetchBlobUrl(vaultAssetPath(wsId, vaultId, path));
    p.catch(() => assetUrls.delete(key)); // a failed load retries next render
    assetUrls.set(key, p);
    while (assetUrls.size > ASSET_CACHE_MAX) {
      const [k, old] = assetUrls.entries().next().value!;
      assetUrls.delete(k);
      void old.then((u) => URL.revokeObjectURL(u), () => {});
    }
    return p;
  }
;;// The note pane: breadcrumb + edit⇄read toggle, CodeMirror markdown editor
// (autosave + wikilink completion) or the sanitized reading view (wikilink
// nav, note embeds hydrated one level deep, image attachments, tag chips).


import { untrack } from 'svelte';
import { announceShown } from '../../lib/announce.svelte';
import type { Completion, CompletionContext, CompletionResult } from '@codemirror/autocomplete';
import CodeEditor from '../../lib/components/CodeEditor.svelte';
import Icon from '../../lib/components/Icon.svelte';
import { vaultNote } from '../../lib/api/vault';
import { ui } from '../../lib/stores/ui.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { renderMermaid } from '../../lib/render/mermaid';
import { renderD2 } from '../canvas/d2';
import { renderNote, resolverFrom, slugifyHeading, stripFrontmatter } from '../../lib/render/markdown';
import { plainPreview, rendersOffThread } from './noteRenderPlan';
import { renderNoteOffThread } from './noteRenderAsync';
import RefineDrawer from './RefineDrawer.svelte';
import StructuredNote from './StructuredNote.svelte';
import LinkPreview from './LinkPreview.svelte';
import { linkPreview } from './previewStore.svelte';
import { structuredModel } from './structuredNote';
import { vault, vaultConflictKind } from './vault.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // -- "Refine with Otto" drawer — open state lives here keyed BY PATH (outside
  // the note data), so reloading the note after the agent edits it does not
  // close the drawer or drop its terminal.
  let refineOpen = $state<Record<string, boolean>>({});
  const refineShown = $derived(!!(vault.notePath && refineOpen[vault.notePath]));

  function toggleRefine(): void {
    const p = vault.notePath;
    if (!p) return;
    refineOpen = { ...refineOpen, [p]: !refineOpen[p] };
  }

  // "Review + fix" (tree context menu) queues a pending refine — force the
  // drawer open for that note; the drawer itself consumes it (the user presses Send).
  $effect(() => {
    const pending = vault.pendingRefine;
    if (pending && vault.notePath === pending.path && !refineOpen[pending.path]) {
      refineOpen = { ...refineOpen, [pending.path]: true };
    }
  });

  // -- attachments: authed blob URLs, patched into the live <img> -------------
  // The renderer emits `<img data-asset="path">` with no src (lazyAssets); an
  // effect fills `src` in place once the blob lands. Images used to be a
  // dependency of the whole render: N images = N+1 full re-renders, embed
  // re-fetches and diagram re-renders (and blob: srcs never passed the
  // sanitizer anyway).
  const noAssetUrl = (): null => null;

  function hydrateImages(root: ParentNode): void {
    const v = vault.current;
    if (!v) return;
    const wsId = vault.wsId, vaultId = v.id;
    for (const el of Array.from(root.querySelectorAll<HTMLImageElement>('img[data-asset]:not([src])'))) {
      const path = el.getAttribute('data-asset')!;
      void assetBlobUrl(wsId, vaultId, path).then(
        (u) => { el.src = u; },
        () => el.classList.add('asset-error'),
      );
    }
  }

  // -- reading view ------------------------------------------------------------
  /** Parse + highlight + sanitize once per note content: toggling Edit⇄Read or
   *  an equal poll refresh reuses the HTML instead of re-rendering 300 KB.
   *  Notes over 64 KB (F10) parse in a worker: the pane shows the escaped
   *  plain body at once, and `offThreadDone` re-runs this when the html lands
   *  in the cache. */
  let offThreadDone = $state(0);
  /** renderKey of a large note whose worker render timed out (Retry shown). */
  let renderTimedOut = $state<string | null>(null);
  let renderRetry = $state(0);
  function retryRender(): void {
    renderTimedOut = null;
    renderRetry++;
  }
  const offThreadInFlight = new Set<string>();
  const renderKey = $derived.by(() => {
    const n = vault.note;
    if (!n || vault.editing) return '';
    return `${vault.current?.id}:${n.meta.path}:${n.meta.hash}:${n.outgoing.map((o) => o.dst_path ?? '').join('|')}`;
  });
  function cacheHtml(key: string, html: string): void {
    renderCache.set(key, html);
    while (renderCache.size > RENDER_CACHE_MAX) renderCache.delete(renderCache.keys().next().value!);
  }
  const rendered = $derived.by(() => {
    void offThreadDone;
    const n = vault.note;
    const key = renderKey;
    if (!n || !key) return '';
    const hit = renderCache.get(key);
    if (hit !== undefined) {
      renderCache.delete(key);
      renderCache.set(key, hit); // LRU touch
      return hit;
    }
    if (rendersOffThread(n.raw)) return plainPreview(stripFrontmatter(n.raw));
    const html = renderNote(stripFrontmatter(n.raw), {
      resolve: resolverFrom(n.outgoing),
      assetUrl: noAssetUrl,
      lazyAssets: true,
    });
    cacheHtml(key, html);
    return html;
  });

  // Large note, not cached: render it in the worker; a superseded result
  // (the note changed or was left meanwhile) is still cached, never shown.
  $effect(() => {
    const key = renderKey;
    const n = vault.note;
    void renderRetry;
    if (!key || !n || !rendersOffThread(n.raw) || renderCache.has(key) || offThreadInFlight.has(key)) return;
    if (untrack(() => renderTimedOut) === key) return;
    const raw = n.raw, outgoing = n.outgoing;
    offThreadInFlight.add(key);
    untrack(() => {
      void renderNoteOffThread(raw, outgoing).then(({ html, timedOut }) => {
        offThreadInFlight.delete(key);
        if (timedOut) {
          // The note hung the worker (S18-305): stay on the plain preview,
          // never cache it, and offer Retry instead of re-parsing on the
          // main thread (which would freeze the whole UI the same way).
          if (renderKey === key) renderTimedOut = key;
          return;
        }
        cacheHtml(key, html);
        if (renderKey === key) offThreadDone++;
      });
    });
  });

  $effect(() => {
    void rendered;
    const host = readEl;
    if (host) untrack(() => hydrateImages(host));
  });

  let readEl = $state<HTMLElement | undefined>();

  function onReadClick(e: MouseEvent): void {
    const t = (e.target as HTMLElement).closest('a.internal-link, a.embed-title, span.tag, div.note-embed');
    if (!t) return;
    if (t.classList.contains('tag')) {
      const tag = t.getAttribute('data-tag');
      if (tag) vault.searchTag(tag);
      return;
    }
    const path = t.getAttribute('data-path') ?? t.getAttribute('data-embed-path');
    if (path) {
      e.preventDefault();
      if (/\.md$/i.test(path)) {
        const anchor = t.getAttribute('data-anchor');
        void vault.open(path).then(() => {
          if (anchor && !anchor.startsWith('^')) scrollToHeading(anchor);
        });
      }
      return;
    }
    // Unresolved → offer to create the note.
    const raw = t.getAttribute('data-raw');
    if (raw && t.getAttribute('data-unresolved')) {
      e.preventDefault();
      const p = raw.endsWith('.md') ? raw : `${raw}.md`;
      void confirmer.ask(`Create “${p}”?`, { title: 'Create note', confirmLabel: 'Create', danger: false }).then((ok) => {
        if (ok) void vault.createNote(p, `# ${raw}\n\n`);
      });
    }
  }

  function onReadKeydown(e: KeyboardEvent): void {
    if (e.key !== 'Enter' && e.key !== ' ') return;
    const t = (e.target as HTMLElement).closest('span.tag, a.internal-link');
    if (!t) return;
    if (t.classList.contains('tag') || e.key === ' ') {
      e.preventDefault();
      (t as HTMLElement).click();
    }
  }

  // Page previews on inline wikilinks (delegated; same popover as the
  // structured panel’s chips). Leaving the link, scrolling or switching
  // notes closes it.
  function onReadOver(e: Event): void {
    const a = (e.target as HTMLElement).closest?.('a.internal-link[data-path]');
    if (a) linkPreview.show(a, a.getAttribute('data-path'));
  }
  function onReadOut(e: Event): void {
    const a = (e.target as HTMLElement).closest?.('a.internal-link[data-path]');
    if (a && !a.contains((e as MouseEvent).relatedTarget as Node | null)) linkPreview.hide();
  }
  $effect(() => {
    void vault.notePath;
    void vault.editing;
    linkPreview.hide();
  });

  function scrollToHeading(anchor: string): void {
    requestAnimationFrame(() => {
      readEl?.querySelector(`#h-${CSS.escape(slugifyHeading(anchor))}`)?.scrollIntoView({
        behavior: scrollBehavior(),
        block: 'start',
      });
    });
  }

  // Hydrate note embeds (depth 1, no recursion — embedded bodies render plain).
  // At most EMBED_WORKERS reads in flight: the daemon admits only a handful of
  // concurrent note preparations and answers the rest "busy" (409), which
  // used to mark every embed past the 4th as broken. A busy answer is
  // retried with a short backoff instead of failing the embed.
  const EMBED_WORKERS = 2;

  async function readEmbed(wsId: string, vaultId: number, path: string) {
    for (let attempt = 0; ; attempt++) {
      try {
        return await vaultNote(wsId, vaultId, path);
      } catch (e) {
        if (attempt >= 4 || vaultConflictKind(e) !== 'busy') throw e;
        await new Promise((r) => setTimeout(r, 250 * 2 ** attempt));
      }
    }
  }

  $effect(() => {
    void rendered;
    const host = readEl;
    if (!host || !vault.current) return;
    const wsId = vault.wsId, vaultId = vault.current.id;
    const seen = new Set<string>([vault.notePath ?? '']);
    const queue: [Element, string][] = [];
    for (const el of Array.from(host.querySelectorAll('div.note-embed[data-embed-path]'))) {
      const p = el.getAttribute('data-embed-path')!;
      if (el.getAttribute('data-hydrated') || seen.has(p)) continue;
      el.setAttribute('data-hydrated', '1');
      queue.push([el, p]);
    }
    const worker = async (): Promise<void> => {
      for (let next = queue.shift(); next; next = queue.shift()) {
        const [el, p] = next;
        try {
          const n = await readEmbed(wsId, vaultId, p);
          const html = renderNote(stripFrontmatter(n.raw), {
            // Embedded content resolves its own links but never re-embeds.
            resolve: resolverFrom(n.outgoing),
            assetUrl: noAssetUrl,
            lazyAssets: true,
          });
          const body = document.createElement('div');
          body.className = 'embed-body md-body';
          body.innerHTML = html;
          // Strip nested embeds inside the embed (depth guard).
          body.querySelectorAll('div.note-embed').forEach((x) => x.removeAttribute('data-embed-path'));
          el.appendChild(body);
          hydrateImages(body);
        } catch {
          el.classList.add('embed-error');
        }
      }
    };
    untrack(() => {
      for (let i = 0; i < Math.min(EMBED_WORKERS, queue.length); i++) void worker();
    });
  });

  // -- diagram blocks: render mermaid / D2 fences to inline SVG -----------------
  // lib/render/markdown emits <div class="diagram-block" data-diagram=…><pre>src</pre></div>;
  // swap each for the rendered SVG (lazy-loaded libs). On error keep the source
  // visible with the parse message — never a blank hole in the note.
  let diagramSeq = 0;
  $effect(() => {
    void rendered;
    const host = readEl;
    if (!host) return;
    for (const el of Array.from(host.querySelectorAll('div.diagram-block:not([data-rendered])'))) {
      el.setAttribute('data-rendered', '1');
      const kind = el.getAttribute('data-diagram');
      const src = el.querySelector('pre.diagram-src')?.textContent ?? '';
      const id = `vault-diag-${++diagramSeq}`;
      untrack(() => {
        const render =
          kind === 'd2'
            ? renderD2(id, src, { dark: ui.resolvedScheme === 'dark', isStale: () => !el.isConnected })
            : renderMermaid(id, src, { dark: ui.resolvedScheme === 'dark', isStale: () => !el.isConnected });
        void render.then(({ svg, error }) => {
          if (!el.isConnected) return;
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
      });
    }
  });

  // -- editor: [[wikilink]] + #tag completion -----------------------------------
  async function vaultCompletions(cx: CompletionContext): Promise<CompletionResult | null> {
    const wiki = cx.matchBefore(/\[\[([^\]\n]*)$/);
    if (wiki) {
      const q = wiki.text.slice(2);
      const hits = await vault.switcherQuery(q);
      const options: Completion[] = hits.slice(0, 30).map((h) => {
        const target = h.path.replace(/\.md$/i, '');
        const insert = h.alias ? `${target}|${h.alias}]]` : `${target}]]`;
        return {
          label: h.alias ?? h.title,
          detail: h.path,
          apply: insert,
          type: 'text',
        };
      });
      return { from: wiki.from + 2, options, filter: false };
    }
    const tag = cx.matchBefore(/(?:^|\s)#([\p{L}\p{N}_\-/]*)$/u);
    if (tag && cx.explicit !== false) {
      const start = tag.text.indexOf('#');
      const q = tag.text.slice(start + 1).toLowerCase();
      const options: Completion[] = vault.tags
        .filter((t) => t.tag.toLowerCase().startsWith(q))
        .slice(0, 20)
        .map((t) => ({ label: `#${t.tag}`, apply: t.tag, detail: `${t.count}`, type: 'keyword' }));
      if (!options.length) return null;
      return { from: tag.from + start + 1, options, filter: false };
    }
    return null;
  }

  function onKeydown(e: KeyboardEvent): void {
    if ((e.metaKey || e.ctrlKey) && e.key === 's') {
      e.preventDefault();
      void vault.saveNow();
    }
    if ((e.metaKey || e.ctrlKey) && e.key === 'e') {
      e.preventDefault();
      vault.setView(!vault.editing);
    }
  }

  const crumb = $derived((vault.notePath ?? '').split('/'));

  // -- structured view for typed OKF notes (plain notes: model is null) ---------
  // The toggle is a per-device preference (all typed notes at once).
  const STRUCTURED_KEY = 'otto.vault.structuredView';
  let structuredOn = $state((() => { try { return localStorage.getItem(STRUCTURED_KEY) !== 'off'; } catch { return true; } })());
  const structured = $derived(vault.note && !vault.editing ? structuredModel(vault.note) : null);
  function toggleStructured(): void {
    structuredOn = !structuredOn;
    try { localStorage.setItem(STRUCTURED_KEY, structuredOn ? 'on' : 'off'); } catch { /* private window */ }
  }
;
async () => {



 { svelteHTML.createElement("svelte:window", {  "onkeydown":onKeydown,});}

if(vault.note){
   { svelteHTML.createElement("div", { "class":`note-view`,});
     { svelteHTML.createElement("header", {});
       { svelteHTML.createElement("nav", {     "class":`crumbs`,"aria-label":`Note path`,"title":vault.notePath ?? '',});
            for(let part of __sveltets_2_ensureArray(crumb)){let i = 1;i;
          if(i < crumb.length - 1){
             { svelteHTML.createElement("span", { "class":`c dim`,});part; } { svelteHTML.createElement("span", { "class":`sep`,});  }
          }else{
             { svelteHTML.createElement("span", { "class":`c`,});part.replace(/\.md$/i, ''); }
          }
        }
       }
       { svelteHTML.createElement("div", { "class":`actions`,});
        
        if(vault.saving){
           {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('span'),('Saving…')));{ svelteHTML.createElement("span", __sveltets_2_union($$action_0), {   "class":`save-state`,});  }}
        } else if (vault.dirty){
           { svelteHTML.createElement("span", {   "class":`save-state`,"title":`Autosaves in a moment`,});  }
        }
        if(structured){
           { svelteHTML.createElement("button", {            "class":`mode-btn`,"title":structuredOn ? 'Hide structured panels' : 'Show structured panels',"aria-label":`Structured view`,"aria-pressed":structuredOn,"onclick":toggleStructured,});structuredOn;
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`layout`,"size":14,}});}
           }
        }
         { svelteHTML.createElement("button", {       "class":`mode-btn`,"title":`Note edit history`,"aria-label":`Note edit history`,"onclick":() => void vault.openHistory(vault.notePath ?? ''),}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`clock`,"size":14,}});} }
         { svelteHTML.createElement("button", {            "class":`mode-btn`,"title":`Refine with Otto`,"aria-label":`Refine with Otto`,"aria-pressed":refineShown,"onclick":toggleRefine,});refineShown;
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`sparkle`,"size":14,}});}
         }
         { svelteHTML.createElement("button", {       "class":`mode-btn labelled`,"title":vault.editing ? 'Reading view (⌘E)' : 'Edit (⌘E)',"onclick":() => vault.setView(!vault.editing),});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":vault.editing ? 'eye' : 'edit',"size":13,}});}
          vault.editing ? 'Read' : 'Edit';
         }
       }
     }

    if(vault.conflict){
       { svelteHTML.createElement("div", {   "class":`conflict`,"role":`alert`,});
                       
         { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => void vault.conflictOverwrite(),"title":`Save your version; the disk version stays in History`,});   }
         { svelteHTML.createElement("button", {   "class":`btn small danger`,"onclick":() => void vault.conflictReload(),});   }
       }
    }

    if(vault.editing){
       { svelteHTML.createElement("div", { "class":`editor-wrap`,});
         { const $$_rotidEedoC2C = __sveltets_2_ensureComponent(CodeEditor); new $$_rotidEedoC2C({ target: __sveltets_2_any(), props: {                "path":vault.notePath ?? 'note.md',"content":vault.draft,"root":"","language":`markdown`,"readOnly":false,"minimal":true,"wrap":true,"completionSource":vaultCompletions,"onchange":(c: string) => vault.onDraftChange(c),}});}
       }
    }else{
      
      
       { const $$_div1 = svelteHTML.createElement("div", {                  "class":`read md-body`,"onclick":onReadClick,"onkeydown":onReadKeydown,"onmouseover":onReadOver,"onmouseout":onReadOut,"onfocusin":onReadOver,"onfocusout":() => linkPreview.hide(),"onscroll":() => linkPreview.hide(),});readEl = $$_div1;
        if(structured && structuredOn && vault.note){
           { const $$_etoNderutcurtS2C = __sveltets_2_ensureComponent(StructuredNote); new $$_etoNderutcurtS2C({ target: __sveltets_2_any(), props: {    "model":structured,"note":vault.note,}});}
        }
        if(renderTimedOut !== null && renderTimedOut === renderKey){
           {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),('Rendering this note timed out; showing plain text.')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {   "class":`render-timeout`,});
             { svelteHTML.createElement("span", {});         }
             { svelteHTML.createElement("button", {     "type":`button`,"class":`btn sm`,"onclick":retryRender,});  }
           }}
        }
         rendered;
       }
    }

     { const $$_weiverPkniL1C = __sveltets_2_ensureComponent(LinkPreview); new $$_weiverPkniL1C({ target: __sveltets_2_any(), props: {}});}

    if(refineShown && vault.notePath){
      
      `${vault.wsId}:${vault.current?.id}:${vault.notePath}`; {
         { const $$_rewarDenifeR1C = __sveltets_2_ensureComponent(RefineDrawer); new $$_rewarDenifeR1C({ target: __sveltets_2_any(), props: {  "path":vault.notePath,}});}
      }
    }
   }
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const NoteView__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type NoteView__SvelteComponent_ = ReturnType<typeof NoteView__SvelteComponent_>;
/*Ωignore_endΩ*/export default NoteView__SvelteComponent_;