///<reference types="svelte" />
;
import { focusOnMount } from '../../lib/focusOnMount';
// Browser tab: a real inline browser with TABS. Each tab is its own native
// child webview (Tauri), so switching tabs is instant and preserves the page's
// scroll/form/login state. A `window.open()` / `target=_blank` inside a tab is
// intercepted natively (no OS popup) and surfaced as `otto://browser-new-tab`,
// which opens a real in-app tab here and focuses it.
//
// On the plain web build (no native webview) each tab falls back to a single
// <iframe>; sites that send X-Frame-Options refuse to frame — use "Open
// externally" for those.
//
// "Take over" mode reloads the active tab via the daemon's proxy endpoint so a
// picker script is injected; clicking elements captures a CSS-selector
// description, the user comments, and all comments are sent to the active agent.
import { tick } from 'svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { ui } from '../../lib/stores/ui.svelte';
import { onTabKey } from '../../lib/tabKeys';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import { openExternal as openExternalUrl } from '../../lib/external';
import { nativeBrowser, nativeBrowserAvailable } from '../../lib/nativeBrowser';
import { api, baseUrl } from '../../lib/api/client';
import { toasts } from '../../lib/toast.svelte';
import { toastError } from '../../lib/toastError';
import { plural } from '../../lib/plural';
import { pollWhileVisible } from '../../lib/poll';
import Icon from '../../lib/components/Icon.svelte';
import type { AttachedIssue, BrowserProxyTicket } from '../../lib/api/types';
;

  // ── Tabs ────────────────────────────────────────────────────────────────────
  type Tab = { id: string; url: string; title: string };;;
  type Annotation = { desc: string; comment: string; url: string };;;

  type Popover = { open: boolean; x: number; y: number; desc: string; url: string };;
;type $$ComponentProps =  { active?: boolean };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // `active` = this panel is the one on screen. The right panel keeps the
  // browser MOUNTED while another tab (or the collapsed strip) is showing, so
  // its tabs, pages and take-over annotations survive a tab switch; a native
  // webview always paints above the HTML, so a hidden panel must hide them.
  let { active = true }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();

  const session = $derived(ws.activeSession);
  /** The agent session "Send to agent" posts to — named on the button, and the
   *  button is disabled (with the reason) when there is none. */
  const sendTarget = $derived(session?.kind === 'agent' ? session : null);
  const attachedIssue = $derived(
    (session?.meta?.issue as AttachedIssue | undefined) ?? null,
  );

  let tabSeq = 0;
  let tabs = $state<Tab[]>([{ id: 't0', url: '', title: 'New tab' }]);
  let activeId = $state('t0');
  // Per-tab last-navigated URL. The native webview is told to navigate ONLY when
  // a tab's url actually changes; switching tabs just show/hides, so each tab's
  // page stays live (state preserved). NOT reactive — bookkeeping only.
  const openedUrl: Record<string, string> = {};

  const activeTab = $derived(tabs.find((t) => t.id === activeId) ?? null);

  // The tab list scrolls horizontally with its scrollbar hidden — keep the
  // active tab (e.g. one a page just opened) scrolled into view.
  let stripEl = $state<HTMLDivElement | null>(null);
  $effect(() => {
    const id = activeId;
    const el = stripEl?.querySelector<HTMLElement>(`[data-tab-id="${id}"]`);
    el?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  });
  const current = $derived(activeTab?.url ?? ''); // active tab's loaded URL

  let urlInput = $state('');
  let reloadTick = $state(0); // bump to force the iframe (web build / take-over) to reload

  // ── Take-over state ────────────────────────────────────────────────────────
  let takeover = $state(false);

  let annotations = $state<Annotation[]>([]);

  let popover = $state<Popover>({ open: false, x: 0, y: 0, desc: '', url: '' });
  let popoverComment = $state('');
  let browserEl = $state<HTMLDivElement | null>(null);
  let popEl = $state<HTMLDivElement | null>(null);
  let takeoverBtnEl = $state<HTMLButtonElement | null>(null);

  // The iframe DOM node — bound below with bind:this (web build / take-over only)
  let frame = $state<HTMLIFrameElement | null>(null);

  // ── Native browser (Tauri child webview, one per tab) ───────────────────────
  let hostEl = $state<HTMLDivElement | null>(null);
  let urlFocused = $state(false);
  const useNative = $derived(nativeBrowserAvailable && !takeover);

  function hostRect(): { x: number; y: number; width: number; height: number } | null {
    if (!hostEl) return null;
    const r = hostEl.getBoundingClientRect();
    if (r.width < 1 || r.height < 1) return null;
    // The native WKWebView page-zoom magnifies the whole SPA from the window's
    // top-left WITHOUT reflowing, so getBoundingClientRect() (CSS px) maps to
    // window-logical points by × the zoom factor. The child webview is positioned
    // in window-logical points, so scale the rect — otherwise it's mis-aligned
    // (too small → desktop shows through; too big → spills over) at zoom ≠ 1.
    const z = ui.zoom || 1;
    return { x: r.left * z, y: r.top * z, width: r.width * z, height: r.height * z };
  }

  // ── Tab helpers ─────────────────────────────────────────────────────────────
  function makeTitle(url: string): string {
    if (!url) return 'New tab';
    try {
      const u = new URL(url);
      const tail = u.pathname.replace(/\/+$/, '').split('/').filter(Boolean).pop();
      return tail || u.hostname;
    } catch {
      return url;
    }
  }

  function newTab(url = ''): string {
    const id = `t${++tabSeq}`;
    tabs = [...tabs, { id, url, title: makeTitle(url) }];
    activeId = id;
    urlInput = url;
    takeover = false;
    return id;
  }

  function setActiveTab(id: string): void {
    if (id === activeId) return;
    activeId = id;
    const t = tabs.find((x) => x.id === id);
    urlInput = t?.url ?? '';
    takeover = false;
    popover = { open: false, x: 0, y: 0, desc: '', url: '' };
  }

  function closeTab(id: string): void {
    const idx = tabs.findIndex((t) => t.id === id);
    if (idx < 0) return;
    if (nativeBrowserAvailable) void nativeBrowser.close(id);
    delete openedUrl[id];
    const remaining = tabs.filter((t) => t.id !== id);
    if (remaining.length === 0) {
      // Never leave zero tabs — replace with a fresh blank one.
      tabs = [];
      newTab('');
      return;
    }
    tabs = remaining;
    if (activeId === id) {
      const next = remaining[Math.min(idx, remaining.length - 1)];
      activeId = next.id;
      urlInput = next.url;
      takeover = false;
    }
  }

  // ── Native webview drivers ──────────────────────────────────────────────────
  // Show the active tab's webview over the host rect; hide every other tab.
  // A native webview always paints above the HTML, so it must also hide for SPA
  // overlays (palette / modals / context menus) and take-over / start pages.
  $effect(() => {
    if (!nativeBrowserAvailable) return;
    const list = tabs; // reactive dep
    const tab = activeTab; // reactive dep
    const overlay = ui.overlayOpen || ctxMenu.open;
    const showActive = active && useNative && !!tab && !!tab.url && !overlay;
    for (const t of list) {
      if (!showActive || !tab || t.id !== tab.id) void nativeBrowser.hide(t.id);
    }
    if (!showActive || !tab) return;
    const r = hostRect();
    if (!r) return;
    if (openedUrl[tab.id] !== tab.url) {
      openedUrl[tab.id] = tab.url;
      void nativeBrowser.open(tab.id, tab.url, r); // create-or-navigate + show
    } else {
      void nativeBrowser.bounds(tab.id, r);
      void nativeBrowser.show(tab.id);
    }
  });

  // Keep the active tab's webview aligned with the panel as it resizes / moves.
  $effect(() => {
    if (!active || !nativeBrowserAvailable || !useNative || !activeTab?.url || !hostEl) return;
    const id = activeId;
    const _z = ui.zoom; // re-align immediately when the page zoom changes
    // Event-driven (perf P14): the host's own size (panel drag, tab bar), the
    // app's size (anything reflowing the shell), window resize and the end of
    // a layout transition — each coalesced to one rect read per frame, and the
    // bounds IPC only sent when the rect actually moved. This replaced a 400 ms
    // forever-interval that re-read layout and sent an IPC every tick; a 2 s
    // visible-only check remains for pure position drift nothing observes.
    let last = '';
    let frame = 0;
    const apply = (): void => {
      frame = 0;
      const r = hostRect();
      if (!r) return;
      const key = `${r.x},${r.y},${r.width},${r.height}`;
      if (key === last) return;
      last = key;
      void nativeBrowser.bounds(id, r);
    };
    const sync = (): void => {
      if (!frame) frame = requestAnimationFrame(apply);
    };
    const ro = new ResizeObserver(sync);
    ro.observe(hostEl);
    ro.observe(document.body);
    window.addEventListener('resize', sync);
    window.addEventListener('transitionend', sync, true);
    const p = pollWhileVisible(() => {
      apply();
    }, { ms: 2000, immediate: true });
    return () => {
      ro.disconnect();
      window.removeEventListener('resize', sync);
      window.removeEventListener('transitionend', sync, true);
      p.stop();
      if (frame) cancelAnimationFrame(frame);
    };
  });

  // Reflect a tab's in-page navigations into its stored URL + the address bar.
  // Event-driven (wry on_navigation) — never polls url(), which panics on a
  // webview that hasn't committed a load yet (and poisons a shared lock).
  $effect(() => {
    if (!nativeBrowserAvailable) return;
    let unlisten = (): void => {};
    let disposed = false;
    void nativeBrowser
      .onUrlChange((id, url) => {
        // Keep openedUrl in sync so the driver effect doesn't re-navigate (loop).
        openedUrl[id] = url;
        const t = tabs.find((x) => x.id === id);
        if (t && t.url !== url) {
          tabs = tabs.map((x) => (x.id === id ? { ...x, url, title: makeTitle(url) } : x));
        }
        if (!urlFocused && id === activeId && url !== urlInput) urlInput = url;
      })
      .then((un) => (disposed ? un() : (unlisten = un)));
    return () => {
      disposed = true;
      unlisten();
    };
  });

  // A page asked to open a new tab (window.open / target=_blank) — open + focus it.
  $effect(() => {
    if (!nativeBrowserAvailable) return;
    let unlisten = (): void => {};
    let disposed = false;
    void nativeBrowser
      .onNewTab((url) => newTab(url))
      .then((un) => {
        if (disposed) un();
        else unlisten = un;
      });
    return () => {
      disposed = true;
      unlisten();
    };
  });

  // Tear every tab's webview down when the panel/tab unmounts.
  $effect(() => () => {
    if (nativeBrowserAvailable) void nativeBrowser.closeAll();
  });

  // ── Derived iframe src (web build / take-over) ──────────────────────────────
  // Take-over loads the page through the daemon proxy with a SINGLE-USE ticket
  // bound to the URL (minted per load) — never the bearer token, which would
  // otherwise sit in the iframe URL where the proxied page can read it. The
  // proxy serves the page sandboxed (opaque origin), and the iframe carries a
  // matching `sandbox` attribute.
  let proxySrc = $state('');
  let proxySeq = 0;
  $effect(() => {
    const url = current;
    const on = takeover;
    void reloadTick; // a reload needs a fresh ticket (the last one is spent)
    const seq = ++proxySeq;
    proxySrc = '';
    if (!on || !url) return;
    api
      .post<BrowserProxyTicket>('/browser/proxy-ticket', { url })
      .then((r) => {
        if (seq !== proxySeq) return;
        proxySrc = `${baseUrl()}/browser/proxy?url=${encodeURIComponent(url)}&ticket=${encodeURIComponent(r.ticket)}`;
      })
      .catch((e) => {
        if (seq !== proxySeq) return;
        toastError('Couldn’t take over this page', e);
        takeover = false;
      });
  });
  const frameSrc = $derived(current ? (takeover ? proxySrc : current) : '');

  // ── Helpers ────────────────────────────────────────────────────────────────
  function normalize(u: string): string {
    const t = u.trim();
    if (!t) return '';
    return /^[a-z]+:\/\//i.test(t) ? t : `https://${t}`;
  }

  // Load a URL into the ACTIVE tab.
  function load(u: string): void {
    const href = normalize(u);
    if (!href) return;
    if (!activeTab) {
      newTab(href);
      reloadTick++;
      return;
    }
    urlInput = href;
    tabs = tabs.map((t) =>
      t.id === activeId ? { ...t, url: href, title: makeTitle(href) } : t,
    );
    reloadTick++;
  }

  function reload(): void {
    if (!current) return;
    if (useNative) void nativeBrowser.reload(activeId);
    else reloadTick++;
  }

  // Reset the ACTIVE tab to its start page.
  function home(): void {
    takeover = false;
    annotations = [];
    popover = { open: false, x: 0, y: 0, desc: '', url: '' };
    if (!activeTab) return;
    delete openedUrl[activeId];
    tabs = tabs.map((t) => (t.id === activeId ? { ...t, url: '', title: 'New tab' } : t));
    urlInput = '';
    if (nativeBrowserAvailable) void nativeBrowser.hide(activeId);
  }

  function openExternal(u: string): void {
    // Route through the shell `open` command (Tauri); plain anchors with
    // target=_blank don't reach the system browser inside the webview.
    void openExternalUrl(normalize(u));
  }

  // Toggle the web inspector (console / network / elements) for the active tab.
  function toggleDevtools(): void {
    if (nativeBrowserAvailable && activeTab?.url) void nativeBrowser.devtools(activeId);
  }

  function onEnter(e: KeyboardEvent): void {
    if (e.key === 'Enter') load(urlInput);
  }

  // ── Take-over toggle ───────────────────────────────────────────────────────
  function toggleTakeover(): void {
    if (takeover) releaseTakeover();
    else takeover = true;
  }

  function releaseTakeover(): void {
    takeover = false;
    popover = { open: false, x: 0, y: 0, desc: '', url: '' };
  }

  // Tell the iframe's injected picker to enable/disable the crosshair.
  function syncTakeoverMessage(): void {
    frame?.contentWindow?.postMessage({ type: 'otto-takeover', enabled: takeover }, '*');
  }

  function onFrameLoad(): void {
    syncTakeoverMessage();
  }

  // ── postMessage listener for picker events from the proxy iframe ──────────
  $effect(() => {
    function onMessage(ev: MessageEvent): void {
      if (!ev.data || typeof ev.data !== 'object') return;

      if (ev.data.type === 'otto-element' && takeover) {
        // Only the take-over frame's own picker may open the comment popover.
        if (!frame || ev.source !== frame.contentWindow) return;
        // The framed page is untrusted and can post these at will: never let
        // one wipe a comment the user is writing, and never take its `url`
        // (it becomes the "while reviewing {url}" line sent to the agent) —
        // the tab's own loaded URL is the truth (S18-19).
        if (popover.open && popoverComment.trim()) return;
        const raw = ev.data as { desc?: unknown; x?: unknown; y?: unknown };
        const desc = String(raw.desc ?? '').slice(0, 2000);
        const x = Number(raw.x) || 0;
        const y = Number(raw.y) || 0;
        const url = current;

        // x/y are the click's clientX/Y inside the iframe; the popover is
        // positioned in .browser, where the frame starts below the tab strip
        // and toolbar — offset by the frame's position, then clamp to it.
        const left = frame.offsetLeft;
        const top = frame.offsetTop;
        const maxX = left + Math.max(0, frame.clientWidth - 320);
        const maxY = top + Math.max(0, frame.clientHeight - 200);

        popoverComment = '';
        popover = {
          open: true,
          x: Math.min(Math.max(left, left + x), maxX),
          y: Math.min(Math.max(top, top + y), maxY),
          desc,
          url,
        };
      }
    }

    window.addEventListener('message', onMessage);
    return () => window.removeEventListener('message', onMessage);
  });

  // Re-sync the take-over message whenever the mode changes.
  $effect(() => {
    const _t = takeover;
    syncTakeoverMessage();
  });

  // Clamp the popover into the panel using its REAL size (the estimate above
  // can't know how tall the description + textarea render) — a popover near the
  // bottom/inline-end edge would otherwise hang off the panel.
  $effect(() => {
    if (!popover.open || !popEl || !browserEl) return;
    const pad = 8;
    const maxX = Math.max(pad, browserEl.clientWidth - popEl.offsetWidth - pad);
    const maxY = Math.max(pad, browserEl.clientHeight - popEl.offsetHeight - pad);
    const x = Math.min(Math.max(pad, popover.x), maxX);
    const y = Math.min(Math.max(pad, popover.y), maxY);
    if (x !== popover.x || y !== popover.y) popover = { ...popover, x, y };
  });

  // Closing the popover (Esc, Cancel, or after Add) hands focus back to the take-over toggle (the
  // control that opened this mode) instead of dropping it on <body>.
  function closePopover(): void {
    popover = { ...popover, open: false };
    void tick().then(() => takeoverBtnEl?.focus());
  }

  // ── Global Esc handler ────────────────────────────────────────────────────
  $effect(() => {
    function onKeydown(e: KeyboardEvent): void {
      // Only an Esc meant for THIS panel: not when it's off screen, not one a
      // Modal/menu/terminal already handled, and not with focus elsewhere —
      // it used to close the comment box (losing the draft) from anywhere.
      if (!active || e.defaultPrevented || !browserEl?.contains(document.activeElement)) return;
      if (e.key === 'Escape') {
        if (popover.open) closePopover();
        else if (takeover) releaseTakeover();
      }
    }
    window.addEventListener('keydown', onKeydown);
    return () => window.removeEventListener('keydown', onKeydown);
  });

  // ── Popover: "Add" button ─────────────────────────────────────────────────
  function addAnnotation(): void {
    if (!popover.open) return;
    annotations = [
      ...annotations,
      { desc: popover.desc, comment: popoverComment, url: popover.url },
    ];
    popoverComment = '';
    // Same close as Esc/Cancel: the focused Add button or textarea unmounts,
    // so focus goes back to the take-over toggle, not <body>.
    closePopover();
  }

  function popoverKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) addAnnotation();
  }

  // ── Send to agent ─────────────────────────────────────────────────────────
  async function sendToAgent(): Promise<void> {
    if (!ws.activeSessionId || ws.activeSession?.kind !== 'agent') {
      toasts.error('Can’t send yet', 'Open an agent session to receive the feedback.');
      return;
    }

    const n = annotations.length;
    const urlRef = annotations[0]?.url ?? current;
    const lines = annotations
      .map((a, i) => `${i + 1}. ${a.desc} — ${a.comment}`)
      .join('\n');

    const text =
      `User left ${n} comment(s) while reviewing ${urlRef}:\n\n` +
      `${lines}\n\n` +
      `Please inspect each and propose fixes.`;

    try {
      await api.post(`/sessions/${ws.activeSessionId}/input`, { text, submit: true });
      annotations = [];
      toasts.success('Sent to agent', plural(n, 'comment'));
    } catch (e) {
      toastError('Couldn’t send the comments to the agent', e);
    }
  }
;
async () => {

 { const $$_div0 = svelteHTML.createElement("div", {  "class":`browser`,});browserEl = $$_div0;
  
  
   { svelteHTML.createElement("div", { "class":`tabstrip-row`,});
     { const $$_div2 = svelteHTML.createElement("div", {      "class":`tabstrip`,"role":`tablist`,"aria-label":`Browser tabs`,});stripEl = $$_div2;
         for(let t of __sveltets_2_ensureArray(tabs)){t.id;
        
         { svelteHTML.createElement("div", {          "class":`btab`,"role":`presentation`,...__sveltets_2_empty({"data-tab-id":t.id}),"onauxclick":(e) => {
            if (e.button === 1) {
              e.preventDefault();
              closeTab(t.id);
            }
          },});t.id === activeId;
           { svelteHTML.createElement("button", {               "class":`btab-main`,"role":`tab`,"aria-selected":t.id === activeId,"tabindex":t.id === activeId ? 0 : -1,"title":t.url || 'New tab',"onclick":() => setActiveTab(t.id),"onkeydown":onTabKey,});
             { svelteHTML.createElement("span", { "class":`btab-title`,});t.title; }
           }
           { svelteHTML.createElement("button", {           "class":`btab-close`,"tabindex":t.id === activeId ? 0 : -1,"title":`Close ${t.title || 'tab'}`,"aria-label":`Close ${t.title || 'tab'}`,"onclick":(e) => {
              e.stopPropagation();
              closeTab(t.id);
            },});
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
           }
         }
      }
     }
     { svelteHTML.createElement("button", {       "class":`btab-new`,"title":`New tab`,"aria-label":`New tab`,"onclick":() => newTab(''),});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}
     }
   }

  
   { svelteHTML.createElement("div", { "class":`toolbar`,});
     { svelteHTML.createElement("button", {         "class":`icon-btn`,"title":`Reload`,"aria-label":`Reload`,"disabled":!current,"onclick":reload,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":13,}});}
     }
     { svelteHTML.createElement("button", {         "class":`icon-btn`,"title":`Start page`,"aria-label":`Start page`,"disabled":!current,"onclick":home,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`home`,"size":13,}});}
     }
     { svelteHTML.createElement("input", {                    "dir":`ltr`,"aria-label":`Address`,"class":`input url-input`,"bind:value":urlInput,"placeholder":`Search or enter URL…`,"spellcheck":`false`,"autocomplete":`off`,"onfocus":() => (urlFocused = true),"onblur":() => (urlFocused = false),"onkeydown":onEnter,});/*Ωignore_startΩ*/() => urlInput = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     { svelteHTML.createElement("button", {         "class":`icon-btn`,"title":`Go`,"aria-label":`Go`,"disabled":!urlInput.trim(),"onclick":() => load(urlInput),});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"size":13,}});}
     }

    
     { const $$_button2 = svelteHTML.createElement("button", {               "class":`icon-btn takeover-btn`,"title":takeover
        ? 'Release (Esc)'
        : 'Take over — click elements to comment & send to the agent',"aria-label":`Take over`,"aria-pressed":takeover,"disabled":!current,"onclick":toggleTakeover,});takeoverBtnEl = $$_button2;takeover;
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`cursor`,"size":13,}});}
     }

    if(nativeBrowserAvailable){
       { svelteHTML.createElement("button", {           "class":`icon-btn`,"title":`DevTools — console, network, elements`,"aria-label":`DevTools`,"disabled":!current,"onclick":toggleDevtools,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`terminal`,"size":13,}});}
       }
    }

     { svelteHTML.createElement("button", {           "class":`icon-btn`,"title":`Open in system browser`,"aria-label":`Open in system browser`,"disabled":!(current || urlInput.trim()),"onclick":() => openExternal(current || urlInput),});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`external`,"size":12,}});}
     }
   }

  if(current){
    if(useNative){
      
       { const $$_div1 = svelteHTML.createElement("div", {  "class":`frame native-host`,});hostEl = $$_div1; }
    }else{
      
      
      frameSrc + '#' + (takeover ? '' : reloadTick); {
         { const $$_iframe1 = svelteHTML.createElement("iframe", {               "class":`frame`,"src":frameSrc,"title":`Browser`,"referrerpolicy":`no-referrer`,"sandbox":takeover ? 'allow-scripts' : undefined,"onload":onFrameLoad,});frame = $$_iframe1;takeover; }
      }
    }

    
    if(popover.open){
       { const $$_div1 = svelteHTML.createElement("div", {              "class":`popover`,"style":`left:${popover.x}px; top:${popover.y}px;`,"role":`dialog`,"aria-label":`Add comment`,"tabindex":-1,"onkeydown":popoverKeydown,});popEl = $$_div1;
         { svelteHTML.createElement("div", {   "class":`popover-desc`,"title":popover.desc,});popover.desc; }
         {const $$action_0 = __sveltets_2_ensureAction(focusOnMount(svelteHTML.mapElementTag('textarea')));{ svelteHTML.createElement("textarea", __sveltets_2_union($$action_0), {               "dir":`auto`,"aria-label":`Comment`,"class":`input popover-textarea`,"bind:value":popoverComment,"placeholder":`Your comment…`,"rows":3,"onkeydown":popoverKeydown,});/*Ωignore_startΩ*/() => popoverComment = __sveltets_2_any(null);/*Ωignore_endΩ*/ }}
         { svelteHTML.createElement("div", { "class":`popover-actions`,});
           { svelteHTML.createElement("button", {   "class":`btn`,"onclick":closePopover,});
            
           }
           { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":!popoverComment.trim(),"onclick":addAnnotation,});
             
           }
         }
       }
    }

    
    if(annotations.length > 0){
       { svelteHTML.createElement("div", { "class":`annot-badge`,});
         { svelteHTML.createElement("span", { "class":`annot-count`,});annotations.length;  }
         { svelteHTML.createElement("button", {         "class":`btn primary small send-target`,"onclick":sendToAgent,"disabled":!sendTarget,"title":sendTarget ? `Send the marks to ${sendTarget.title || 'the active agent'} and start its next turn` : 'Open an agent session to send marks',});
           annotations.length;  sendTarget?.title || 'agent';
         }
         { svelteHTML.createElement("button", {       "class":`btn small`,"title":`Clear all annotations`,"onclick":() => (annotations = []),});
          
         }
       }
    }

     { svelteHTML.createElement("div", { "class":`frame-foot`,});
       { svelteHTML.createElement("span", {   "class":`dim ellipsis`,"title":current,});current; }
       { svelteHTML.createElement("button", {   "class":`link`,"onclick":() => openExternal(current),});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`external`,"size":12,}});}  
       }
     }
  }else{
     { svelteHTML.createElement("div", { "class":`start`,});
       { svelteHTML.createElement("p", { "class":`hint`,});
        if(nativeBrowserAvailable){             
                    
                         
                    }else{            
                     
                }
       }

      if(attachedIssue){
         { svelteHTML.createElement("section", { "class":`section`,});
           { svelteHTML.createElement("div", { "class":`section-title`,});  }
           { svelteHTML.createElement("button", {   "class":`quick-link`,"onclick":() => openExternal(attachedIssue.url),});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`ticket`,"size":13,}});}
             { svelteHTML.createElement("div", { "class":`ql-text`,});
               { svelteHTML.createElement("span", { "class":`ql-key`,});attachedIssue.key; }
               { svelteHTML.createElement("span", { "class":`ql-label`,});attachedIssue.summary; }
             }
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`external`,"size":12,}});}
           }
         }
      }

       { svelteHTML.createElement("section", { "class":`section`,});
         { svelteHTML.createElement("div", { "class":`section-title`,});  }
         { svelteHTML.createElement("button", {     "class":`quick-link`,"onclick":() => openExternal('https://id.atlassian.com/manage-profile/security/api-tokens'),});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`key`,"size":13,}});}
           { svelteHTML.createElement("span", { "class":`ql-label`,});   }
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`external`,"size":12,}});}
         }
       }
     }
  }
 }


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const BrowserPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type BrowserPanel__SvelteComponent_ = ReturnType<typeof BrowserPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default BrowserPanel__SvelteComponent_;