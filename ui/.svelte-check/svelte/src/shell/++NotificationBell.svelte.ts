///<reference types="svelte" />
;
import { plural } from '../lib/plural';
import Skeleton from '../lib/components/Skeleton.svelte';
import LoadState from '../lib/components/LoadState.svelte';
// Notification center bell: unread badge + an anchored popover of notices.
//
// It lives in the Navigator header / collapsed Rail (`side`: the panel opens
// beside the sidebar edge, a caret pointing back at the bell) and in the
// phone top bar (`below`: caret on top). The panel is portalled to <body>
// and positioned `fixed` from the button's rect: the sidebar's
// `backdrop-filter` makes it the containing block for fixed descendants, so
// an in-place panel would be clipped to the sidebar.
//
// Unread survives OPENING: rows keep their unread dot while the panel is up
// and are marked read when it closes (you've now seen them). Session notices
// arrive grouped per session (`notifications.rows`); a row is one button
// that runs the notice's action.
import { tick, untrack } from 'svelte';
import Icon from '../lib/components/Icon.svelte';
import ProviderIcon from '../lib/components/ProviderIcon.svelte';
import { notifications, type NoticeRow } from '../lib/stores/notifications.svelte';
import { ui } from '../lib/stores/ui.svelte';
import { ctxMenu } from '../lib/contextmenu.svelte';
import { confirmer } from '../lib/confirm.svelte';
import { copyWithToast } from '../lib/clipboard';
import type { Notice } from '../lib/api/types';
import { rel } from '../lib/stores/now.svelte';

;type $$ComponentProps =  { placement?: 'side' | 'below' };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { placement = 'side' }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();

  let open = $state(false);
  let btnEl = $state<HTMLButtonElement | null>(null);
  let panelEl = $state<HTMLElement | null>(null);
  let panelH = $state(0);
  /** `anchor` = the bell's centre along the caret edge, in panel coordinates. */
  let pos = $state({ top: 0, left: 0, width: 360, maxHeight: 400, anchor: 20 });
  /** Which panel edge carries the caret (physical, so RTL just flips it). */
  let caretSide = $state<'left' | 'right' | 'top'>('left');
  /** Notice ids whose full detail is expanded. */
  let expanded = $state<Set<string>>(new Set());

  const PANEL_W = 360;
  const EDGE = 8;
  /** Room between the anchor edge and the panel — the caret sits in it. */
  const GAP = 10;
  /** Keep the caret's centre this far along its edge, so the whole caret (a
   *  14px square → ~20px base) stays clear of the 12px rounded corners. */
  const CARET_MIN = 22;
  /** Bodies longer than this get a clamped preview + "Show details". */
  const LONG_BODY = 140;

  // Clamp the panel into the viewport (never flip without a floor) and cap its
  // height to the room left below its top, so the list always scrolls in view.
  function place(): void {
    if (!btnEl) return;
    const r = btnEl.getBoundingClientRect();
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const width = Math.min(PANEL_W, vw - EDGE * 2);
    const rtl = getComputedStyle(btnEl).direction === 'rtl';
    let left: number;
    let top: number;
    if (placement === 'side') {
      // Beside the sidebar's edge (not the button's) so the panel never covers
      // the Navigator it was opened from; the caret bridges the short gap back
      // to the bell, which sits at the header's inline-end.
      const edge = btnEl.closest('nav')?.getBoundingClientRect() ?? r;
      left = rtl ? edge.left - GAP - width : edge.right + GAP;
      // Line the panel's head up with the bell (caret ~24px down).
      top = r.top + r.height / 2 - 24;
      caretSide = rtl ? 'right' : 'left';
    } else {
      left = rtl ? r.left : r.right - width;
      top = r.bottom + GAP;
      caretSide = 'top';
    }
    left = Math.min(Math.max(left, EDGE), vw - width - EDGE);
    // A bell near the window top (the Navigator header) may pull the panel a
    // few px above the usual edge margin, so the caret can still meet the
    // bell's centre without landing on the rounded corner.
    const minTop =
      caretSide === 'top' ? EDGE : Math.max(2, Math.min(EDGE, r.top + r.height / 2 - CARET_MIN));
    top = Math.min(Math.max(top, minTop), Math.max(minTop, vh - EDGE - 200));
    const maxHeight = Math.min(vh * 0.72, vh - top - EDGE);
    const anchor =
      caretSide === 'top' ? r.left + r.width / 2 - left : r.top + r.height / 2 - top;
    pos = { top, left, width, maxHeight, anchor };
  }

  // Caret offset along its edge, kept clear of the rounded corners (and of the
  // real rendered height, which is often below the max).
  const caretAt = $derived.by(() => {
    const span = caretSide === 'top' ? pos.width : panelH || pos.maxHeight;
    return Math.min(Math.max(pos.anchor, CARET_MIN), Math.max(CARET_MIN, span - CARET_MIN));
  });

  /** ui.modalCount right after we registered — anything above it is a layer
   *  opened on top of this panel. */
  let baseModals = 0;

  function isTopLayer(): boolean {
    return !ctxMenu.open && !confirmer.open && !ui.paletteOpen && ui.modalCount <= baseModals;
  }

  $effect(() => {
    if (!open) return;
    // untracked: place() reads/writes caretSide + pos; this effect keys on `open`.
    untrack(place);
    // Register as an overlay so the live browser's native webview (which
    // paints above the HTML) hides while the panel is up. untrack: pushModal
    // reads modalCount — see Modal.svelte.
    untrack(() => {
      ui.pushModal();
      baseModals = ui.modalCount;
    });
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === 'Escape' && isTopLayer()) {
        e.preventDefault();
        close();
      } else if (e.key === 'Tab' && isTopLayer()) {
        trapTab(e);
      }
    };
    window.addEventListener('resize', place);
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('resize', place);
      window.removeEventListener('keydown', onKey);
      untrack(() => ui.popModal());
    };
  });

  // A sheet opened above us (palette, a Modal) would sit UNDER this panel's
  // layer — step aside instead of covering it.
  $effect(() => {
    if (open && (ui.modalCount > baseModals || ui.paletteOpen)) untrack(() => close(false));
  });

  // Focus the first row (else the first control) once the panel is painted.
  $effect(() => {
    if (!open) return;
    void tick().then(() => {
      if (!panelEl) return;
      const first =
        panelEl.querySelector<HTMLElement>('.nb-row') ??
        panelEl.querySelector<HTMLElement>('button:not([disabled])');
      first?.focus();
    });
  });

  const FOCUSABLE = 'button:not([disabled]), [href], [tabindex]:not([tabindex="-1"])';
  function trapTab(e: KeyboardEvent): void {
    if (!panelEl) return;
    const els = Array.from(panelEl.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
      (el) => el.offsetParent !== null,
    );
    if (els.length === 0) return;
    const idx = els.indexOf(document.activeElement as HTMLElement);
    if (e.shiftKey && idx <= 0) {
      e.preventDefault();
      els[els.length - 1].focus();
    } else if (!e.shiftKey && (idx === -1 || idx === els.length - 1)) {
      e.preventDefault();
      els[0].focus();
    }
  }

  // Move a node to <body> for its lifetime (escapes the sidebar's containing block).
  function portal(node: HTMLElement): { destroy(): void } {
    document.body.appendChild(node);
    return { destroy: () => node.remove() };
  }

  // Load notices once on mount.
  $effect(() => {
    void notifications.ensureLoaded();
  });

  function toggle(): void {
    if (open) close();
    else {
      expanded = new Set();
      open = true;
    }
  }

  /** Close the panel; what was on screen is now seen → mark it read. Only
   *  the notices this client holds are marked (not the server-wide read-all),
   *  so one the daemon created but hasn't pushed yet stays unread. Focus
   *  returns to the bell unless the close was a row action that moved on. */
  function close(restoreFocus = true): void {
    if (!open) return;
    open = false;
    void notifications.markSeenRead();
    if (restoreFocus) btnEl?.focus();
  }

  function actionLabel(notice: Notice): string | null {
    switch (notice.action?.type) {
      case 'open_url':
        return 'Open link';
      case 'open_session':
        return 'Go to session';
      case 'reauth':
        return 'Re-authenticate';
      case 'open_route':
        return notice.title.includes('waiting for your approval') ? 'Review' : 'Open';
      default:
        return null;
    }
  }

  function isLong(n: Notice): boolean {
    return n.body.length > LONG_BODY || n.body.includes('\n');
  }

  function toggleDetails(id: string): void {
    const next = new Set(expanded);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    expanded = next;
  }

  function activate(row: NoticeRow): void {
    const n = row.latest;
    if (n.action) {
      void notifications.markManyRead(row.notices.map((x) => x.id));
      close(false);
      void notifications.runAction(n);
    } else if (row.sessionId === null && isLong(n)) {
      toggleDetails(n.id);
      void notifications.markRead(n.id);
    } else {
      void notifications.markManyRead(row.notices.map((x) => x.id));
    }
  }

  function rowLabel(row: NoticeRow): string {
    const parts = [row.title, row.text];
    // The severity glyph is aria-hidden — say it in words instead.
    if (row.severity === 'error') parts.unshift('Error');
    else if (row.severity === 'warn') parts.unshift('Warning');
    if (row.notices.length > 1) parts.push(`${row.notices.length} notifications`);
    if (row.unread) parts.push('unread');
    const act = actionLabel(row.latest);
    return act ? `${parts.join(', ')}. ${act}` : parts.join(', ');
  }

  async function copyDetail(n: Notice): Promise<void> {
    await copyWithToast(n.body, 'the notification');
  }

  function openMore(e: MouseEvent): void {
    // Anchor under the ⋯ button (showAt handles keyboard activation & RTL).
    ctxMenu.showAt(e.currentTarget as HTMLElement, [
      {
        label: 'Clear all notifications…',
        icon: 'trash',
        danger: true,
        disabled: notifications.notices.length === 0,
        action: () => void clearAll(),
      },
    ]);
  }

  async function clearAll(): Promise<void> {
    // The confirm sheet renders under this layer — step aside first.
    close();
    const n = notifications.notices.length;
    const ok = await confirmer.ask(
      `Delete all ${plural(n, 'notification')}? This can’t be undone.`,
      { title: 'Clear all notifications', confirmLabel: 'Clear all', danger: true },
    );
    if (ok) await notifications.clear();
  }

  const SECTION: Record<NoticeRow['bucket'], string> = { 0: 'Needs you', 1: 'Alerts', 2: 'Recent' };
  const sections = $derived.by(() => {
    const out: { bucket: NoticeRow['bucket']; rows: NoticeRow[] }[] = [];
    for (const r of notifications.rows) {
      const last = out[out.length - 1];
      if (last && last.bucket === r.bucket) last.rows.push(r);
      else out.push({ bucket: r.bucket, rows: [r] });
    }
    return out;
  });

  const unreadCount = $derived(notifications.unreadRows);
  const badge = $derived(unreadCount > 99 ? '99+' : String(unreadCount));
  const allRead = $derived(notifications.notices.every((n) => n.read));
;
async () => {

 { svelteHTML.createElement("div", { "class":`bell-wrap`,});
   { const $$_button1 = svelteHTML.createElement("button", {               "class":`icon-btn bell-btn`,"onclick":toggle,"aria-label":unreadCount > 0 ? `Notifications, ${unreadCount} unread` : 'Notifications',"aria-haspopup":`dialog`,"aria-expanded":open,"title":unreadCount > 0 ? `Notifications, ${unreadCount} unread` : 'Notifications',});btnEl = $$_button1;unreadCount > 0;
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`bell`,"size":14,}});}
    if(unreadCount > 0){
       { svelteHTML.createElement("span", {   "class":`count-bubble sev-${notifications.unreadSeverity ?? 'info'}`,"aria-hidden":`true`,});badge; }
    }
   }

  if(open){
     {const $$action_0 = __sveltets_2_ensureAction(portal(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {  "class":`bell-layer`,});
      
       { svelteHTML.createElement("div", {         "class":`bell-backdrop`,"onclick":() => close(),"oncontextmenu":(e) => { e.preventDefault(); close(); },"role":`presentation`,}); }

       { svelteHTML.createElement("div", {     "class":`nb-pop`,"style":`top:${pos.top}px;left:${pos.left}px;width:${pos.width}px;max-height:${pos.maxHeight}px`,});
         { svelteHTML.createElement("span", {       "class":`nb-caret at-${caretSide} glass-raised`,"style":`--caret:${caretAt}px`,"aria-hidden":`true`,}); }
         { const $$_div3 = svelteHTML.createElement("div", {         "class":`panel glass-raised`,"role":`dialog`,"aria-label":`Notifications`,});panelEl = $$_div3;panelH= $$_div3.offsetHeight;
           { svelteHTML.createElement("header", { "class":`panel-head`,});
             { svelteHTML.createElement("span", { "class":`panel-title`,});  }
             { svelteHTML.createElement("span", { "class":`grow`,}); }
             { svelteHTML.createElement("button", {       "class":`btn ghost small`,"onclick":() => notifications.markAllRead(),"disabled":allRead,});
                
             }
             { svelteHTML.createElement("button", {         "class":`btn ghost small nb-more-btn`,"onclick":openMore,"aria-label":`More notification actions`,"title":`More notification actions`,});
               { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":13,}});}
             }
           }

           { svelteHTML.createElement("div", { "class":`panel-list`,});
            if(notifications.error && notifications.notices.length === 0){
               { svelteHTML.createElement("div", {   "class":`panel-empty nb-error`,"role":`alert`,});
                 { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":24,}});}
                 { svelteHTML.createElement("p", {});   }
                 { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => notifications.load(),"disabled":notifications.loading,});
                   { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});} 
                 }
               }
            } else if (!notifications.settled){
               { svelteHTML.createElement("div", { "class":`panel-empty`,});
                 { svelteHTML.createElement("span", {     "class":`spinner`,"style":`--spinner-size: 18px`,"aria-hidden":`true`,}); }
                 { const $$_notelekS6C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS6C({ target: __sveltets_2_any(), props: {      "rows":3,"height":36,"label":`notifications`,}});}
               }
            } else if (notifications.rows.length === 0){
               { svelteHTML.createElement("div", { "class":`panel-empty`,});
                 { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`bell`,"size":24,}});}
                 { svelteHTML.createElement("p", {});    }
               }
            }else{
              
               { const $$_etatSdaoL5C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL5C({ target: __sveltets_2_any(), props: {         children:() => { return __sveltets_2_any(0); },"what":`notifications`,"error":notifications.error,"loading":notifications.loading,"onretry":() => notifications.load(),}});
                 for(let sec of __sveltets_2_ensureArray(sections)){sec.bucket;
                if(sections.length > 1){
                   { svelteHTML.createElement("div", { "class":`nb-section`,});SECTION[sec.bucket]; }
                }
                   for(let row of __sveltets_2_ensureArray(sec.rows)){row.key;
                  const n = row.latest;
                  const long = row.sessionId === null && isLong(n);
                  const act = actionLabel(n);
                   { svelteHTML.createElement("div", {  "class":`nb-item`,});row.unread;
                     { svelteHTML.createElement("button", {         "class":`nb-row`,"onclick":() => activate(row),"aria-label":rowLabel(row),"title":act ?? (long ? (expanded.has(n.id) ? 'Hide details' : 'Show details') : undefined),});
                       { svelteHTML.createElement("span", {   "class":`nb-dot`,"aria-hidden":`true`,}); }
                       { svelteHTML.createElement("span", {   "class":`nb-lead`,"aria-hidden":`true`,});
                        if(row.provider){
                           { const $$_nocIredivorP9C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP9C({ target: __sveltets_2_any(), props: {    "provider":row.provider,"size":16,}});}
                        } else if (row.severity === 'error'){
                           { svelteHTML.createElement("span", { "class":`nb-sev err`,}); { const $$_nocI10C = __sveltets_2_ensureComponent(Icon); new $$_nocI10C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":14,}});} }
                        } else if (row.severity === 'warn'){
                           { svelteHTML.createElement("span", { "class":`nb-sev warn`,}); { const $$_nocI10C = __sveltets_2_ensureComponent(Icon); new $$_nocI10C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":14,}});} }
                        } else if (row.sessionId){
                           { const $$_nocI9C = __sveltets_2_ensureComponent(Icon); new $$_nocI9C({ target: __sveltets_2_any(), props: {    "name":`command`,"size":14,}});}
                        }else{
                           { const $$_nocI9C = __sveltets_2_ensureComponent(Icon); new $$_nocI9C({ target: __sveltets_2_any(), props: {    "name":n.kind === 'credential' ? 'key' : 'bell',"size":14,}});}
                        }
                       }
                       { svelteHTML.createElement("span", { "class":`nb-main`,});
                         { svelteHTML.createElement("span", { "class":`nb-line`,});
                           { svelteHTML.createElement("span", { "class":`nb-title`,});row.title; }
                          if(row.provider && row.severity !== 'info'){
                             { svelteHTML.createElement("span", { "class":`nb-sev ${row.severity === 'error' ? 'err' : 'warn'}`,});
                               { const $$_nocI11C = __sveltets_2_ensureComponent(Icon); new $$_nocI11C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}
                             }
                          }
                          if(row.notices.length > 1){
                             { svelteHTML.createElement("span", { "class":`nb-count`,}); row.notices.length; }
                          }
                           { svelteHTML.createElement("time", {     "class":`nb-time`,"datetime":n.created_at,"title":new Date(n.created_at).toLocaleString(),});rel(n.created_at, { compact: true }); }
                         }
                        if(row.text){
                           { svelteHTML.createElement("span", {    "class":`nb-text`,"title":row.sessionId ? row.text : undefined,});row.sessionId !== null;
                            row.text;
                           }
                        }
                       }
                     }
                    if(long){
                       { svelteHTML.createElement("button", {       "class":`nb-disclose`,"aria-expanded":expanded.has(n.id),"onclick":() => toggleDetails(n.id),});
                         { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":expanded.has(n.id) ? 'chevronUp' : 'chevronDown',"size":12,}});}
                        expanded.has(n.id) ? 'Hide details' : 'Show details';
                       }
                      if(expanded.has(n.id)){
                         { svelteHTML.createElement("div", { "class":`nb-detail`,});
                           { svelteHTML.createElement("pre", {});n.body; }
                           { svelteHTML.createElement("button", {   "class":`btn ghost small nb-copy`,"onclick":() => copyDetail(n),});
                             { const $$_nocI9C = __sveltets_2_ensureComponent(Icon); new $$_nocI9C({ target: __sveltets_2_any(), props: {    "name":`copy`,"size":12,}});} 
                           }
                         }
                      }
                    }
                     { svelteHTML.createElement("button", {         "class":`nb-dismiss`,"onclick":() =>
                        row.notices.length > 1
                          ? notifications.dismissMany(row.notices.map((x) => x.id))
                          : notifications.dismiss(n.id),"aria-label":`Dismiss ${row.title}`,"title":`Dismiss ${row.title}`,});
                       { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
                     }
                   }
                }
              }
               LoadState}
            }
           }
         }
       }
     }}
  }
 }


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const NotificationBell__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type NotificationBell__SvelteComponent_ = ReturnType<typeof NotificationBell__SvelteComponent_>;
/*Ωignore_endΩ*/export default NotificationBell__SvelteComponent_;