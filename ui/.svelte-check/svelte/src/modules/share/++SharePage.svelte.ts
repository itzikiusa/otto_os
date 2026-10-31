///<reference types="svelte" />
;// Full-screen guest view for a scoped share link (#/s/<sessionId>).
// No rail, no navigator, no right panel — just the session header + terminal.
//
// The share token was captured from the URL fragment by the router (Task 3.1)
// and stored in-memory (mirrored into this tab's sessionStorage so a reload
// keeps access); we read it here via getShareToken(sessionId).
// All API calls use the scoped token directly (not the owner login token).
//
// Task 7.5: Email-OTP gate. When the session load returns a 403 with the
// "email-OTP verification" signal, we show an OTP entry screen before attaching.
// After verification, `getSharedSession` is retried. An "Extend" control allows
// re-sending a fresh OTP to the locked original recipient.

import { onDestroy } from 'svelte';
import { announceShown } from '../../lib/announce.svelte';
import { loadErrorText } from '../../lib/loadError';
import Terminal from '../../lib/components/Terminal.svelte';
import { PRIMARY_SCROLLBACK } from '../../lib/components/termFlow';
import Icon from '../../lib/components/Icon.svelte';
import { getSharedSession, getShareWhoami, verifyShareOtp, extendShare } from '../../lib/api/share';
import { forgetStoredShareToken, getShareToken } from '../../lib/router.svelte';
import type { Session, SessionStatus } from '../../lib/api/types';
import { ApiError } from '../../lib/api/client';
import { ui } from '../../lib/stores/ui.svelte';
;

  interface Props {
    sessionId: string;
  };;

  // ── view states ────────────────────────────────────────────────────────────
  // 'loading'  — initial REST call in-flight
  // 'otp'      — OTP required (403 otp_pending); show the code entry form
  // 'ok'       — session loaded; show the terminal
  // 'error'    — irrecoverable error
  // 'extend'   — terminal ended, show "Extend" prompt to re-send OTP
  type ViewState = 'loading' | 'otp' | 'ok' | 'error' | 'extend';;function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { sessionId }: Props = $props();

  // The share token captured by the router from the URL fragment.
  // If it's missing the link is invalid / already stripped without a prior capture.
  const token = $derived(getShareToken(sessionId));

  let viewState = $state<ViewState>('loading');

  let session = $state<Session | null>(null);
  let loadError = $state<string | null>(null);
  /** Human cause for the error card (content.md §4) — the raw text is only the
   *  dim detail line under it. */
  let loadCause = $state('');
  let liveStatus = $state<SessionStatus | null>(null);

  // ── OTP form state ─────────────────────────────────────────────────────────
  let otpInput = $state('');
  let otpError = $state<string | null>(null);
  let otpBusy = $state(false);
  let extendBusy = $state(false);
  let extendSent = $state(false);

  /** Why the OTP card shows: the first visit, or a lapsed access window. */
  let otpReason = $state<'first' | 'lapsed'>('first');

  // Requests belong to one link/load. A delayed reply from a prior link or
  // access window must never change the currently attached session or role.
  let generation = 0;
  onDestroy(() => { generation++; });
  function isCurrent(seq: number, id: string, bearer: string): boolean {
    return seq === generation && id === sessionId && bearer === token;
  }

  // ── load session (or detect OTP gate) ────────────────────────────────────
  async function loadSession(): Promise<void> {
    const t = token;
    if (!t) return;
    const id = sessionId;
    const seq = ++generation;
    viewState = 'loading';
    isViewer = true;
    otpBusy = false;
    extendBusy = false;
    loadError = null;
    session = null;
    liveStatus = null;

    try {
      const loaded = await getSharedSession(id, t);
      if (!isCurrent(seq, id, t)) return;
      let viewer = true;
      // An Editor link may type. A failed / older-daemon whoami keeps the
      // safe read-only default (the daemon enforces the role either way).
      try {
        viewer = (await getShareWhoami(t)).role !== 'editor';
      } catch {
        viewer = true;
      }
      if (!isCurrent(seq, id, t)) return;
      session = loaded;
      isViewer = viewer;
      viewState = 'ok';
    } catch (e: unknown) {
      if (!isCurrent(seq, id, t)) return;
      if (e instanceof ApiError && e.status === 403 && isOtpPending(e)) {
        // The share requires email-OTP verification before attaching.
        viewState = 'otp';
      } else {
        loadError = loadErrorText(e);
        loadCause = shareErrorCause(e);
        linkDead = isDeadLink(e);
        if (linkDead) forgetStoredShareToken(id);
        viewState = 'error';
      }
    }
  }

  /** A revoked / expired / unknown link: retrying can never succeed, so the
   *  error card drops its Retry (it only invited a pointless loop). */
  let linkDead = $state(false);
  function isDeadLink(e: unknown): boolean {
    return e instanceof ApiError && [401, 403, 404, 410].includes(e.status);
  }

  /** What a guest can act on: a dead link vs. a host that's unreachable/busy. */
  function shareErrorCause(e: unknown): string {
    if (e instanceof ApiError) {
      if (isDeadLink(e)) {
        return 'This share link was revoked or has expired. Ask the person who shared it for a new link.';
      }
      return 'The host had a problem opening this session. Try again in a moment.';
    }
    if (e instanceof TypeError) return 'Can’t reach the host. Check your connection, then try again.';
    return 'Something went wrong opening this session. Try again in a moment.';
  }

  /** Recognise the OTP-pending 403 from the daemon's feature guard. */
  function isOtpPending(e: ApiError): boolean {
    // The daemon renders: {"code":"forbidden","message":"share requires email-OTP verification"}
    return e.message.includes('email-OTP') || e.message.includes('otp');
  }

  $effect(() => {
    const t = token;
    // Reset link-local form state even when navigating to an invalid link.
    generation++;
    otpInput = '';
    otpError = null;
    otpBusy = false;
    extendBusy = false;
    extendSent = false;
    otpReason = 'first';
    if (!t) return;
    void loadSession();
  });

  // While attached, re-check access quietly (every minute, and when the tab
  // comes back into focus): an OTP window that lapsed needs a fresh code, and
  // a revoked/expired link must say so — not leave a terminal that silently
  // stopped updating.
  let termView = $state<{ reconnect: () => void } | null>(null);
  async function recheckAccess(): Promise<void> {
    const t = token;
    if (!t || viewState !== 'ok') return;
    const id = sessionId;
    const seq = generation;
    try {
      await getSharedSession(id, t);
      // The link answers again: a terminal whose reconnects gave up during
      // a long daemon restart picks back up on its own (S14-303).
      if (isCurrent(seq, id, t) && viewState === 'ok') termView?.reconnect();
    } catch (e: unknown) {
      if (!isCurrent(seq, id, t) || viewState !== 'ok') return;
      if (e instanceof ApiError && e.status === 403 && isOtpPending(e)) {
        otpReason = 'lapsed';
        otpInput = '';
        otpError = null;
        viewState = 'otp';
      } else if (isDeadLink(e)) {
        loadError = null;
        loadCause = shareErrorCause(e);
        // Revoked mid-view: no Retry (it can never succeed), and a reload
        // must not resurrect the stored token.
        linkDead = true;
        forgetStoredShareToken(id);
        viewState = 'error';
      }
      // A network blip keeps the terminal (it reconnects on its own).
    }
  }
  $effect(() => {
    if (viewState !== 'ok') return;
    const timer = setInterval(() => void recheckAccess(), 60_000);
    const onFocus = (): void => void recheckAccess();
    window.addEventListener('focus', onFocus);
    return () => {
      clearInterval(timer);
      window.removeEventListener('focus', onFocus);
    };
  });

  // ── OTP verification ───────────────────────────────────────────────────────
  async function submitOtp(): Promise<void> {
    const t = token;
    if (!t || otpBusy || extendBusy) return;
    const id = sessionId;
    const seq = generation;
    const code = otpInput.trim();
    if (code.length !== 6 || !/^\d{6}$/.test(code)) {
      otpError = 'Enter the 6-digit code from your email.';
      return;
    }
    otpBusy = true;
    otpError = null;
    try {
      const resp = await verifyShareOtp(t, code);
      if (!isCurrent(seq, id, t)) return;
      if (resp.verified) {
        // OTP accepted — reload the session (now unblocked).
        otpInput = '';
        otpReason = 'first';
        await loadSession();
      } else {
        otpError = 'That code didn’t match. Check the latest email and try again.';
      }
    } catch (e: unknown) {
      if (!isCurrent(seq, id, t)) return;
      if (e instanceof ApiError && e.status === 429) {
        otpError = 'Too many attempts. Wait a minute, then try again.';
      } else if (e instanceof ApiError && e.status === 401) {
        otpError = 'That code is wrong or has expired. Check the latest email, or re-send a code.';
      } else if (e instanceof TypeError) {
        otpError = 'Can’t reach the host. Check your connection, then try again.';
      } else {
        otpError = 'Couldn’t verify the code. Try again in a moment.';
      }
    } finally {
      if (isCurrent(seq, id, t)) otpBusy = false;
    }
  }

  // ── Extend (re-send OTP to locked recipient) ──────────────────────────────
  async function requestExtend(): Promise<void> {
    const t = token;
    if (!t || extendBusy || otpBusy) return;
    const id = sessionId;
    const seq = generation;
    extendBusy = true;
    extendSent = false;
    otpError = null;
    try {
      await extendShare(t);
      if (!isCurrent(seq, id, t)) return;
      extendSent = true;
      // After extend, show the OTP prompt again for the fresh code.
      otpInput = '';
      viewState = 'otp';
    } catch (e: unknown) {
      if (!isCurrent(seq, id, t)) return;
      if (e instanceof ApiError && e.status === 429) {
        otpError = 'Too many requests. Wait a minute, then re-send the code.';
      } else if (e instanceof ApiError && e.status === 400) {
        otpError = 'A new code can’t be sent for this link. Ask the person who shared it for a new link.';
      } else {
        otpError = 'Couldn’t send a new code. Try again in a moment.';
      }
    } finally {
      if (isCurrent(seq, id, t)) extendBusy = false;
    }
  }

  // Called by Terminal when the WS status indicates the session ended or
  // the share OTP gate is re-engaged (the terminal can no longer attach).
  function onTermStatus(s: SessionStatus): void {
    liveStatus = s;
  }

  // The shared PROCESS stopped (not the access window — that is caught by
  // recheckAccess and routed to the OTP card). A new code can't bring an ended
  // session back, so the overlay only says what happened and offers Reconnect.
  const termEnded = $derived(liveStatus === 'exited');

  // Effective status for the header badge.
  const status = $derived<SessionStatus>(liveStatus ?? session?.status ?? 'idle');

  // Is this a viewer share (read-only)? Safe default: yes until `/share/whoami`
  // says Editor (enforcement is in the daemon either way).
  let isViewer = $state(true);

  const statusLabel: Record<SessionStatus, string> = {
    running: 'Running',
    working: 'Working',
    idle: 'Idle',
    exited: 'Ended',
    reconnectable: 'Disconnected',
  };

  // Allow submitting OTP form via Enter key.
  function onOtpKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter') void submitOtp();
  }
;
async () => {


if(!token){
   { svelteHTML.createElement("div", {   "class":`share-error`,"style":`zoom:${ui.zoom}`,});
     { svelteHTML.createElement("div", {   "class":`error-card`,"role":`alert`,});
       { svelteHTML.createElement("div", { "class":`error-icon`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":26,}});} }
       { svelteHTML.createElement("h2", {});       }
       { svelteHTML.createElement("p", {});
                   
                        
       }
     }
   }


} else if (viewState === 'loading'){
   { svelteHTML.createElement("div", {   "class":`share-error`,"style":`zoom:${ui.zoom}`,});
    
     { svelteHTML.createElement("div", { "class":`error-card`,});
       { svelteHTML.createElement("div", {    "class":`spinner`,"aria-hidden":`true`,});__sveltets_2_ensureType(String, Number, " "); }
       {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('p'),('Connecting to the shared session…')));{ svelteHTML.createElement("p", __sveltets_2_union($$action_0), {   "class":`dim`,});     }}
     }
   }


} else if (viewState === 'otp'){
   { svelteHTML.createElement("div", {   "class":`share-error`,"style":`zoom:${ui.zoom}`,});
     { svelteHTML.createElement("div", { "class":`error-card otp-card`,});
       { svelteHTML.createElement("div", { "class":`otp-icon`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`mail`,"size":26,}});} }
       { svelteHTML.createElement("h2", {});otpReason === 'lapsed' ? 'Your access window ended' : 'Enter your access code'; }
       { svelteHTML.createElement("p", {});
        if(otpReason === 'lapsed'){                 }else{              }
        if(extendSent){
           { svelteHTML.createElement("br", {});} { svelteHTML.createElement("strong", {});      }
        }
       }
       { svelteHTML.createElement("div", { "class":`otp-input-row`,});
         { svelteHTML.createElement("input", {                      "dir":`ltr`,"class":`otp-input`,"type":`text`,"inputmode":`numeric`,"pattern":`[0-9]*`,"maxlength":6,"placeholder":`000000`,"autocomplete":`one-time-code`,"bind:value":otpInput,"onkeydown":onOtpKeydown,"aria-label":`One-time access code`,});/*Ωignore_startΩ*/() => otpInput = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":otpBusy || extendBusy,"onclick":submitOtp,});
          otpBusy ? 'Verifying…' : 'Verify';
         }
       }
      if(otpError){
         { svelteHTML.createElement("p", {   "class":`otp-error`,"role":`alert`,});otpError; }
      }
       { svelteHTML.createElement("div", { "class":`otp-extend-row`,});
         { svelteHTML.createElement("span", { "class":`dim`,});     }
         { svelteHTML.createElement("button", {     "class":`otp-link`,"disabled":extendBusy || otpBusy,"onclick":requestExtend,});
          extendBusy ? 'Sending…' : 'Re-send code';
         }
       }
     }
   }


} else if (viewState === 'error'){
   { svelteHTML.createElement("div", {   "class":`share-error`,"style":`zoom:${ui.zoom}`,});
     { svelteHTML.createElement("div", {   "class":`error-card`,"role":`alert`,});
       { svelteHTML.createElement("div", { "class":`error-icon`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":26,}});} }
       { svelteHTML.createElement("h2", {});    }
       { svelteHTML.createElement("p", {});loadCause; }
      if(loadError){ { svelteHTML.createElement("p", { "class":`hint`,});loadError; }}
      if(!linkDead){
         { svelteHTML.createElement("button", {   "class":`btn ec-retry`,"onclick":() => void loadSession(),}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":13,}});}  }
      }
     }
   }


}else{
   { svelteHTML.createElement("div", {   "class":`share-root`,"style":`zoom:${ui.zoom}`,});
     { svelteHTML.createElement("header", { "class":`share-header`,});
       { svelteHTML.createElement("span", {   "class":`session-title`,"title":session?.title ?? '',});session?.title ?? 'Loading session…'; }
       { svelteHTML.createElement("span", { "class":`header-spacer`,}); }
      if(session){
         { svelteHTML.createElement("span", {   "class":`status-badge status-${status}`,"title":`Session status`,});
           { svelteHTML.createElement("span", {   "class":`status-dot`,"aria-hidden":`true`,}); }statusLabel[status] ?? status;
         }
      }
      
      if(session && !isViewer){
         { svelteHTML.createElement("span", {   "class":`role-chip`,"title":`This link lets you type into the shared terminal. What you type runs on the host.`,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":12,}});}   
         }
      }
     }

     { svelteHTML.createElement("div", { "class":`terminal-fill`,});
      if(session){
        
         { const $$_lanimreT2C = __sveltets_2_ensureComponent(Terminal); const $$_lanimreT2 = new $$_lanimreT2C({ target: __sveltets_2_any(), props: {             sessionId,"readOnly":isViewer,"forceDark":true,"preferDom":true,"shareToken":token,"onstatus":onTermStatus,"scrollback":PRIMARY_SCROLLBACK,}});termView = $$_lanimreT2;}
      }else{
         { svelteHTML.createElement("div", { "class":`connecting-overlay`,});
           { svelteHTML.createElement("span", {});  }
         }
      }

      
      if(termEnded){
         {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),({ text: 'This session has ended. Reconnect to pick it up again if the host restarts it.', assertive: true })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {   "class":`extend-overlay`,});
           { svelteHTML.createElement("div", { "class":`extend-card`,});
             { svelteHTML.createElement("div", { "class":`extend-icon`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`clock`,"size":26,}});} }
             { svelteHTML.createElement("h3", {});    }
             { svelteHTML.createElement("p", {});
                            
                   
             }
             { svelteHTML.createElement("button", {   "class":`btn primary`,"onclick":() => void loadSession(),});
               { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":13,}});} 
             }
           }
         }}
      }
     }
   }
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SharePage__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SharePage__SvelteComponent_ = ReturnType<typeof SharePage__SvelteComponent_>;
/*Ωignore_endΩ*/export default SharePage__SvelteComponent_;