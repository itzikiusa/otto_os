///<reference types="svelte" />
;// Global context menu overlay — mount once in App.svelte.
//
// Positioning: a cursor menu (right-click) opens at the pointer; an anchored
// menu (ctxMenu.showAt / a button click) opens under its trigger, lined up
// with the trigger's start or end edge (mirrored in RTL), flips above only
// when it fits there, and is otherwise clamped into the window. The height
// cap is measured against window.innerHeight — never vh, which in the
// WKWebView resolves to the SCREEN height.
//
// Keyboard: focus moves to the first item on open (the search box in a
// filterable menu); ↑/↓ Home/End move, Enter/Space activate, a letter jumps
// to the next item starting with it, Esc/Tab close. Focus returns to the
// trigger on close (see ctxMenu.close()).

import { tick } from 'svelte';
import Icon, { asIcon } from './Icon.svelte';
import { ctxMenu, type MenuItem } from '../contextmenu.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  const PAD = 8; // viewport margin
  const GAP = 4; // trigger ↔ menu

  let menuEl: HTMLDivElement | null = $state(null);

  // Final position + height cap. `ready` keeps the menu invisible for the one
  // frame before it has been measured, so it never flashes at a wrong spot.
  let cx = $state(0);
  let cy = $state(0);
  let maxH = $state(0);
  let ready = $state(false);

  const isRtl = (): boolean => document.documentElement.dir === 'rtl';

  function place(): void {
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    maxH = Math.max(80, vh - PAD * 2);
    if (!menuEl) return;
    const w = menuEl.offsetWidth;
    const h = Math.min(menuEl.offsetHeight, maxH);
    const a = ctxMenu.anchor;
    let x: number;
    let y: number;
    if (a) {
      // Logical alignment: 'start' = the trigger's leading edge.
      const alignLeft = (ctxMenu.align === 'start') !== isRtl();
      x = alignLeft ? a.left : a.right - w;
      const below = a.bottom + GAP;
      const above = a.top - GAP - h;
      // Prefer below; flip above only when it fits there entirely.
      y = below + h > vh - PAD && above >= PAD ? above : below;
    } else {
      x = isRtl() ? ctxMenu.x - w : ctxMenu.x;
      y = ctxMenu.y;
    }
    // Clamp INSIDE the viewport rather than flipping without a floor: a menu
    // taller than the window would flip to a negative top and be unreachable.
    cx = Math.max(PAD, Math.min(x, vw - w - PAD));
    cy = Math.max(PAD, Math.min(y, vh - h - PAD));
  }

  $effect(() => {
    if (!ctxMenu.open) return;
    void ctxMenu.seq; // re-run for a menu re-opened in the same tick
    ready = false;
    maxH = Math.max(80, window.innerHeight - PAD * 2);
    // Defer one frame so the menu has rendered and can be measured.
    const raf = requestAnimationFrame(async () => {
      place();
      ready = true;
      await tick();
      if (ctxMenu.filter) searchEl?.focus();
      else focusAt(0);
    });
    return () => cancelAnimationFrame(raf);
  });

  // ── Roving focus ───────────────────────────────────────────────────────────
  function itemEls(): HTMLButtonElement[] {
    return menuEl ? Array.from(menuEl.querySelectorAll<HTMLButtonElement>('.ctx-item:not([aria-disabled="true"])')) : [];
  }
  function focusAt(i: number): void {
    const els = itemEls();
    if (!els.length) {
      menuEl?.focus();
      return;
    }
    els[(i + els.length) % els.length].focus();
  }
  function move(delta: number): void {
    const els = itemEls();
    const cur = els.indexOf(document.activeElement as HTMLButtonElement);
    if (cur === -1) focusAt(delta > 0 ? 0 : -1);
    else if (ctxMenu.filter && cur === 0 && delta < 0) searchEl?.focus();
    else focusAt(cur + delta);
  }
  function typeAhead(ch: string): void {
    const els = itemEls();
    if (!els.length) return;
    const start = els.indexOf(document.activeElement as HTMLButtonElement);
    for (let k = 1; k <= els.length; k++) {
      const el = els[(start + k) % els.length];
      if ((el.textContent ?? '').trim().toLowerCase().startsWith(ch)) {
        el.focus();
        return;
      }
    }
  }

  // Keys are handled on the window in the CAPTURE phase while the menu is
  // open: whatever holds focus (a terminal after a right-click, the menu, the
  // search box) the menu owns the navigation keys, and the underlying pane
  // does not also react to them.
  function onWindowKey(e: KeyboardEvent): void {
    if (!ctxMenu.open) return;
    const inSearch = !!searchEl && document.activeElement === searchEl;
    const swallow = (): void => {
      e.preventDefault();
      e.stopPropagation();
    };
    switch (e.key) {
      case 'Escape':
        swallow();
        ctxMenu.close();
        return;
      case 'Tab':
        swallow();
        ctxMenu.close();
        return;
      case 'ArrowDown':
        swallow();
        if (inSearch) focusAt(0);
        else move(1);
        return;
      case 'ArrowUp':
        swallow();
        if (!inSearch) move(-1);
        return;
      case 'Home':
      case 'End':
        if (inSearch) return; // caret movement in the field
        swallow();
        focusAt(e.key === 'Home' ? 0 : -1);
        return;
      case 'Enter':
      case ' ': {
        if (inSearch) return; // Enter → onSearchKey; Space types a space
        swallow();
        const el = document.activeElement;
        if (el instanceof HTMLButtonElement && el.closest('.ctx-menu')) el.click();
        return;
      }
    }
    if (inSearch || e.metaKey || e.ctrlKey || e.altKey || e.key.length !== 1) return;
    swallow();
    if (ctxMenu.filter && searchEl) {
      // Typing on a row of a filterable menu goes to its search box.
      searchEl.focus();
      ctxMenu.query += e.key;
    } else {
      typeAhead(e.key.toLowerCase());
    }
  }

  /** The menu caps at 260px: a long repo / branch / session name ellipsizes,
   *  so hovering a clipped label shows it whole (unclipped rows get none). An
   *  item's own `title` (e.g. why it is disabled) always wins. */
  function titleIfClipped(e: MouseEvent, item: MenuItem): void {
    const row = e.currentTarget as HTMLElement;
    if (item.title) {
      row.title = item.title;
      return;
    }
    const el = row.querySelector<HTMLElement>('.ctx-label');
    if (!el) return;
    row.title = el.scrollWidth > el.clientWidth + 1 ? (el.textContent ?? '') : '';
  }

  /** Conditional rows (`...(cond ? [...] : [])`) leave separators at the
   *  edges or doubled up; drop those so every menu reads clean. */
  function tidy(items: MenuItem[]): MenuItem[] {
    const out: MenuItem[] = [];
    for (const it of items) {
      const sep = it.separator || !it.label;
      if (sep && (out.length === 0 || out[out.length - 1].separator || !out[out.length - 1].label)) continue;
      out.push(it);
    }
    while (out.length && (out[out.length - 1].separator || !out[out.length - 1].label)) out.pop();
    return out;
  }

  function clickItem(item: MenuItem): void {
    if (item.disabled) return;
    // Close FIRST: an action may open another menu (a ⋯ row that clicks a
    // collapsed "New ▾" button) and must not be closed right after.
    ctxMenu.close();
    item.action?.();
  }

  // ── Filterable mode ────────────────────────────────────────────────────────
  // Order-preserving view: pinned items + separators always show; other items
  // must match the query, and at most `maxVisible` of them render at once (the
  // rest collapse into a "+N more" hint until the query narrows the list).
  const view = $derived.by(() => {
    const items = ctxMenu.items;
    if (!ctxMenu.filter) return { items: tidy(items), hidden: 0 };
    const q = ctxMenu.query.trim().toLowerCase();
    const cap = ctxMenu.maxVisible > 0 ? ctxMenu.maxVisible : Infinity;
    const out: typeof items = [];
    let shown = 0;
    let hidden = 0;
    for (const it of items) {
      if (it.pinned || it.separator || !it.label) {
        out.push(it);
        continue;
      }
      if (q !== '' && !it.label.toLowerCase().includes(q)) continue;
      if (shown < cap) {
        out.push(it);
        shown++;
      } else {
        hidden++;
      }
    }
    return { items: tidy(out), hidden };
  });

  let searchEl: HTMLInputElement | null = $state(null);

  /** Enter in the search box activates the first matched list item. */
  function onSearchKey(e: KeyboardEvent): void {
    if (e.key !== 'Enter') return;
    e.preventDefault();
    const first = view.items.find((it) => !it.pinned && !it.separator && it.label && !it.disabled && it.action);
    if (first) clickItem(first);
  }
;
async () => {

 { svelteHTML.createElement("svelte:window", {    "onkeydowncapture":onWindowKey,"onresize":() => ctxMenu.open && place(),});}

if(ctxMenu.open){
  
   { svelteHTML.createElement("div", {           "class":`ctx-backdrop`,"onclick":() => ctxMenu.close(),"oncontextmenu":(e) => { e.preventDefault(); ctxMenu.close(); },"onwheel":() => ctxMenu.close(),"role":`presentation`,}); }

   { const $$_div0 = svelteHTML.createElement("div", {             "class":`ctx-menu glass-raised`,"style":`left:${cx}px;top:${cy}px;max-height:${maxH}px`,"role":`menu`,"aria-label":`Context menu`,"tabindex":-1,});menuEl = $$_div0;ready;
    if(ctxMenu.filter){
       { svelteHTML.createElement("div", { "class":`ctx-search`,});
         { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":12,}});}
         { const $$_input2 = svelteHTML.createElement("input", {                 "dir":`ltr`,"class":`ctx-search-input`,"type":`text`,"placeholder":ctxMenu.filterPlaceholder,"aria-label":ctxMenu.filterPlaceholder,"bind:value":ctxMenu.query,"spellcheck":`false`,"onkeydown":onSearchKey,});searchEl = $$_input2;/*Ωignore_startΩ*/() => ctxMenu.query = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       }
    }
        for(let item of __sveltets_2_ensureArray(view.items)){let i = 1;i;
      if(item.separator || !item.label){
         { svelteHTML.createElement("div", {   "class":`ctx-sep`,"role":`separator`,}); }
      }else{
         { svelteHTML.createElement("button", {                   "class":`ctx-item`,"aria-disabled":item.disabled ? 'true' : undefined,"role":item.checked === undefined ? 'menuitem' : 'menuitemcheckbox',"aria-checked":item.checked === undefined ? undefined : item.checked,"tabindex":-1,"onclick":() => clickItem(item),"onmouseenter":(e) => titleIfClipped(e, item),"onmousemove":(e) => {
            // Pointer and keyboard share one highlight: hovering moves focus
            // (not onto a disabled row — it only shows its tooltip).
            const el = e.currentTarget;
            if (item.disabled) return;
            if (document.activeElement !== el && document.activeElement !== searchEl) el.focus({ preventScroll: true });
          },});item.danger;item.disabled;
          if(item.checked !== undefined){
            
            if(item.checked){
               { svelteHTML.createElement("span", { "class":`ctx-icon ctx-check`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":13,}});} }
            }else{
               { svelteHTML.createElement("span", { "class":`ctx-icon-gap`,}); }
            }
            if(item.icon){ { svelteHTML.createElement("span", { "class":`ctx-icon`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":asIcon(item.icon),"size":13,}});} }}
          } else if (item.icon){
             { svelteHTML.createElement("span", { "class":`ctx-icon`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":asIcon(item.icon),"size":13,}});} }
          }else{
             { svelteHTML.createElement("span", { "class":`ctx-icon-gap`,}); }
          }
           { svelteHTML.createElement("span", { "class":`ctx-label`,});item.label; }
          if(item.hint){ { svelteHTML.createElement("span", { "class":`ctx-hint`,});item.hint; }}
         }
      }
    }
    if(view.hidden > 0){
       { svelteHTML.createElement("div", { "class":`ctx-more`,}); view.hidden;      }
    }
   }
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ContextMenu__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ContextMenu__SvelteComponent_ = ReturnType<typeof ContextMenu__SvelteComponent_>;
/*Ωignore_endΩ*/export default ContextMenu__SvelteComponent_;