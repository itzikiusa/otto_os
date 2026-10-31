///<reference types="svelte" />
;// Reader-mode page render. The fetched page's markdown is rendered through
// the SAME sanitizing renderer Vault's reading view uses (`renderNote` from
// `lib/render/markdown.ts`) — never `{@html}` on raw page content. No
// wikilink/attachment resolution applies here (this isn't a vault note), so
// `resolve`/`assetUrl` are no-ops.
//
// Mark mode: toggling "Mark element" arms a click-to-annotate overlay on the
// rendered `.page` tree. A click on a rendered element (while armed) builds a
// short, stable CSS selector scoped to `.page` via `buildSelector` (shared
// with the live-tab picker overlay — see `./selector.ts`; id/data-attr, else
// tag + nth-of-type per ancestor step — the DOM here is our own sanitized
// render, not the original page, so it's almost always the nth-of-type
// fallback), snapshots its outerHTML (excerpt) + textContent (text), and
// opens an inline note composer. Saving
// calls `browser.createAnnotation`, which the store also appends locally for
// instant feedback (see browser.svelte.ts). Existing marks for this URL are
// re-highlighted after every render by re-resolving each annotation's
// selector against the live DOM — a selector that no longer matches (page
// content changed) is silently skipped rather than erroring.

import { tick } from 'svelte';
import { toastError } from '../../lib/toastError';
import { renderNote } from '../../lib/render/markdown';
import { browser } from '../../lib/stores/browser.svelte';
import Icon from '../../lib/components/Icon.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { buildSelector } from './selector';
import { announce } from '../../lib/announce.svelte';
import type { BrowserPage } from '../../lib/api/types';

;type $$ComponentProps =  {
    page: BrowserPage | null;
    loading: boolean;
    error: string;
    onretry?: () => void;
    /** Empty state's next step: focus the address bar (⌘L). */
    onopenurl?: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  
  
  
  
  
  
  
  
  
  

  let {
    page,
    loading,
    error,
    onretry,
    onopenurl,
  }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();

  const html = $derived(
    page ? renderNote(page.markdown, { resolve: () => null, assetUrl: () => null }) : '',
  );
  // A long article (≥ 300 KB of markdown) skips layout/paint of its
  // off-screen blocks (`content-visibility: auto`, perf SB-16): the DOM stays
  // whole — find-in-page, marks and selectors are unaffected — but only what
  // is near the viewport is laid out.
  const LONG_ARTICLE = 300 * 1024;
  const longArticle = $derived((page?.markdown.length ?? 0) >= LONG_ARTICLE);

  let markMode = $state(false);
  let articleEl: HTMLElement | null = $state(null);
  let pending: { selector: string; excerpt: string; text: string } | null = $state(null);
  let noteText = $state('');
  let saving = $state(false);
  let markButton: HTMLButtonElement | undefined = $state();
  let pendingTarget: HTMLElement | null = null;

  function toggleMark(): void {
    markButton?.focus();
    markMode = !markMode;
    pending = null;
    // Said at the state edge: a status line inserted with its text is silent.
    if (markMode) announce(MARK_HINT);
  }
  const MARK_HINT = 'Choose a passage · ↑↓ to move · Enter to mark · Esc to stop';

  // A composer belongs to one fetched page, never the next URL or refresh.
  $effect(() => {
    void page;
    markMode = false;
    pending = null;
    pendingTarget = null;
    noteText = '';
  });

  // While marking, the rendered blocks form a roving keyboard selection.
  // Restore their original semantics as soon as marking ends.
  $effect(() => {
    const root = articleEl;
    if (!markMode || !root) return;
    void html;
    const nested = Array.from(root.querySelectorAll<HTMLElement>('a, button, input, select, textarea, [tabindex]'));
    const nestedTabs = nested.map(el => el.getAttribute('tabindex'));
    nested.forEach(el => { el.tabIndex = -1; });
    const blocks = Array.from(root.children).filter((el): el is HTMLElement => el instanceof HTMLElement);
    const previous = blocks.map((el) => [el.getAttribute('tabindex'), el.getAttribute('role')]);
    blocks.forEach((el, i) => { el.tabIndex = i === 0 ? 0 : -1; el.setAttribute('role', 'button'); });
    return () => {
      nested.forEach((el, i) => {
        const value = nestedTabs[i];
        if (value === null) el.removeAttribute('tabindex'); else el.setAttribute('tabindex', value);
      });
      blocks.forEach((el, i) => {
        for (const [j, attr] of ['tabindex', 'role'].entries()) {
          const value = previous[i][j];
          if (value === null) el.removeAttribute(attr); else el.setAttribute(attr, value);
        }
      });
    };
  });

  async function markElement(target: Element): Promise<void> {
    if (!articleEl) return;
    pendingTarget = target.closest<HTMLElement>('[role="button"]');
    const selector = buildSelector(target, articleEl);
    const excerpt = target.outerHTML.slice(0, 2000);
    const text = (target.textContent || '').trim().slice(0, 500);
    pending = { selector, excerpt, text };
    noteText = '';
    await tick();
    document.querySelector<HTMLTextAreaElement>('.mark-composer textarea')?.focus();
  }

  function onArticleClick(e: MouseEvent): void {
    if (!markMode || !articleEl) return;
    const target = e.target as Element | null;
    if (!target || target === articleEl) return;
    e.preventDefault();
    void markElement(target);
  }

  function onArticleKey(e: KeyboardEvent): void {
    if (!markMode || !articleEl) return;
    const blocks = Array.from(articleEl.children) as HTMLElement[];
    const i = blocks.findIndex(el => el === e.target || el.contains(e.target as Node));
    if (i < 0) return;
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      void markElement(blocks[i]);
      return;
    }
    const next = e.key === 'ArrowDown' ? (i + 1) % blocks.length
      : e.key === 'ArrowUp' ? (i + blocks.length - 1) % blocks.length
      : e.key === 'Home' ? 0 : e.key === 'End' ? blocks.length - 1 : -1;
    if (next < 0) return;
    e.preventDefault();
    blocks.forEach((el, j) => { el.tabIndex = j === next ? 0 : -1; });
    blocks[next].focus();
  }

  function cancelMark(): void {
    pending = null;
    noteText = '';
    pendingTarget?.focus();
  }

  async function saveMark(): Promise<void> {
    if (!pending || !page || saving) return;
    saving = true;
    const selection = pending;
    const sourcePage = page;
    try {
      await browser.createAnnotation({
        url: page.url,
        selector: pending.selector,
        excerpt: pending.excerpt,
        text: pending.text,
        comment: noteText.trim(),
      });
      if (page !== sourcePage || pending !== selection) return;
      pending = null;
      noteText = '';
      markMode = false;
      await tick();
      // Return focus to the trigger only if it was left on the (now removed)
      // composer: an attached agent's AskBar takes it on the new mark
      // (`browser.markTick`), and that nudge must win.
      const at = document.activeElement;
      if (!at || at === document.body) markButton?.focus();
    } catch (e) {
      toastError('Couldn’t save mark', e);
    } finally {
      saving = false;
    }
  }

  // Re-highlight existing marks for this page whenever the rendered HTML or
  // the annotation list changes.
  $effect(() => {
    void html;
    const anns = browser.annotations;
    const root = articleEl;
    if (!root) return;
    root.querySelectorAll('[data-mark-id]').forEach((el) => {
      el.removeAttribute('data-mark-id');
      el.classList.remove('marked');
    });
    for (const a of anns) {
      if (page && a.url !== page.url) continue;
      let el: Element | null = null;
      try {
        el = root.querySelector(a.selector);
      } catch {
        el = null;
      }
      if (el) {
        el.setAttribute('data-mark-id', a.id);
        el.classList.add('marked');
      }
    }
  });
;
async () => {

 { svelteHTML.createElement("div", { "class":`reader`,});
  if(loading || error){
    
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {        "what":`this page`,"variant":`page`,loading,error,"empty":true,onretry,}});}
  } else if (!page){
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {               "variant":`page`,"icon":`compass`,"title":`Open a page`,"body":`Type a URL in the address bar to read it here as clean text. Mark passages to hand them to an agent.`,"actionLabel":onopenurl ? 'Enter a URL' : undefined,"actionIcon":`search`,"onaction":onopenurl,}});}
  }else{
    if(page.degraded){
       { svelteHTML.createElement("div", { "class":`degraded`,});
              page.engine;     
       }
    }

     { svelteHTML.createElement("div", { "class":`toolbar`,});
      if(markMode){ { svelteHTML.createElement("span", { "class":`mark-hint`,});MARK_HINT; }}
       { const $$_button2 = svelteHTML.createElement("button", {         "class":`btn small`,"onclick":toggleMark,"title":markMode ? 'Stop marking' : 'Mark a passage to hand to an agent',});markMode;markButton = $$_button2;
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`target`,"size":12,}});}
        markMode ? 'Stop marking' : 'Mark passage';
       }
     }

    
    
    
     { const $$_article1 = svelteHTML.createElement("article", {          "class":`page md-body`,"onclick":onArticleClick,"onkeydown":onArticleKey,});markMode;longArticle;articleEl = $$_article1;
       { svelteHTML.createElement("h2", { "class":`page-title`,});page.title || page.url; }
      
       html;
     }

    if(pending){
       { svelteHTML.createElement("div", { "class":`mark-composer`,});
         { svelteHTML.createElement("p", { "class":`composer-excerpt`,});pending.text.slice(0, 140); }
         { svelteHTML.createElement("textarea", {              "dir":`auto`,"bind:value":noteText,"placeholder":`Check this price against last week’s`,"aria-label":`Note for this mark`,"rows":2,"spellcheck":`false`,"onkeydown":(e) => {
            if (e.key === 'Escape') { e.preventDefault(); cancelMark(); }
            else if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) { e.preventDefault(); void saveMark(); }
          },});/*Ωignore_startΩ*/() => noteText = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
         { svelteHTML.createElement("div", { "class":`composer-actions`,});
           { svelteHTML.createElement("button", {   "class":`btn`,"onclick":cancelMark,});  }
           { svelteHTML.createElement("button", {       "class":`btn primary`,"disabled":saving,"onclick":saveMark,"title":`Save mark (⌘↩)`,});
            saving ? 'Saving…' : 'Save mark';
           }
         }
       }
    }
  }
 }

 { svelteHTML.createElement("svelte:window", {  "onkeydown":(e) => { if (e.key === 'Escape' && markMode && !pending) { markMode = false; markButton?.focus(); } },});}


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ReaderView__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ReaderView__SvelteComponent_ = ReturnType<typeof ReaderView__SvelteComponent_>;
/*Ωignore_endΩ*/export default ReaderView__SvelteComponent_;