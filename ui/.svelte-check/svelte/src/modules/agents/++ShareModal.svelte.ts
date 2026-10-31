///<reference types="svelte" />
;// "Share this session" modal: mint a scoped share link (viewer or editor,
// with a fixed TTL), display the URL + a QR code for phone hand-off, and
// list/revoke existing active shares for the session.
// Task 7.5: adds recipient email + duration (≤12h) for OTP-gated shares and
// shows a hint when the owner has no verified email sender.

import { onMount } from 'svelte';
import { toastError } from '../../lib/toastError';
import QRCode from 'qrcode';
import Modal from '../../lib/components/Modal.svelte';
import Icon from '../../lib/components/Icon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import { api } from '../../lib/api/client';
import type { CreateShareReq, CreateShareResp, ShareInfo, EmailSenderResp } from '../../lib/api/types';
import { toasts } from '../../lib/toast.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { confirmOutward } from '../../lib/confirmOutward';
import { router } from '../../lib/router.svelte';
import { copyTextOrThrow } from '../../lib/clipboard';
;

  interface Props {
    sessionId: string;
    onclose: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { sessionId, onclose }: Props = $props();

  // ── form state ──────────────────────────────────────────────────────────────
  let role = $state<'viewer' | 'editor'>('viewer');
  let ttlSecs = $state(3600);
  let label = $state('');
  /** Recipient email for OTP-gated shares (Task 7.5). Empty = no OTP gate. */
  let recipientEmail = $state('');
  /** Duration in seconds for OTP-gated shares; capped at 43200 (12h). */
  let durationSecs = $state(3600);

  // ── email sender state (needed to warn when no verified sender exists) ────────
  let emailSender = $state<EmailSenderResp | null>(null);
  let emailSenderLoading = $state(true);

  // ── minted-link state (shown after POST) ────────────────────────────────────
  let mintedUrl = $state<string | null>(null);
  /** Whether another device can open the minted link (false ⇒ loopback-only:
   *  no Public link domain and no network listener — S20-01). */
  let mintedRemote = $state(true);
  /** `lan`: the link is the LAN listener (same Wi-Fi only, self-signed
   *  certificate) — S20-303. */
  let mintedLan = $state(false);
  /** Which share the URL/QR panel shows — revoking ANOTHER link keeps it. */
  let mintedShareId: string | null = null;
  let qrCanvas: HTMLCanvasElement | null = $state(null);

  // ── existing-shares state ────────────────────────────────────────────────────
  let shares = $state<ShareInfo[]>([]);
  let sharesLoading = $state(false);
  /** Cause of the last failed list load; null = ok. A failed load must never read as "no shares". */
  let sharesError = $state<string | null>(null);

  // ── busy flags ────────────────────────────────────────────────────────────────
  let generating = $state(false);
  let revoking = $state<Record<string, boolean>>({});
  let revokingAll = $state(false);

  // ── fetch existing shares + email sender status on mount ───────────────────
  onMount(() => {
    void loadShares();
    void loadEmailSender();
  });

  async function loadEmailSender(): Promise<void> {
    emailSenderLoading = true;
    try {
      emailSender = await api.get<EmailSenderResp>('/email-sender');
    } catch {
      // Non-fatal: just hide the OTP hint.
      emailSender = null;
    } finally {
      emailSenderLoading = false;
    }
  }

  /** True when the owner has a working verified email sender. */
  const hasSender = $derived(emailSender?.verified === true);

  /** Derive whether the OTP branch is active (email field has content). */
  const isOtpShare = $derived(recipientEmail.trim() !== '');

  async function loadShares(): Promise<void> {
    sharesLoading = true;
    sharesError = null;
    try {
      const resp = await api.get<{ shares: ShareInfo[] }>(`/sessions/${encodeURIComponent(sessionId)}/shares`);
      shares = resp.shares;
    } catch (e) {
      // Not fatal for minting a new link, but the list says it couldn't load.
      sharesError = loadErrorText(e);
    } finally {
      sharesLoading = false;
    }
  }

  // ── render QR code into the canvas whenever mintedUrl changes ───────────────
  $effect(() => {
    if (!mintedUrl || !qrCanvas) return;
    void QRCode.toCanvas(qrCanvas, mintedUrl, {
      width: 200,
      margin: 1,
      color: { dark: '#000000', light: '#ffffff' },
    });
  });

  // ── generate a new share link ────────────────────────────────────────────────
  async function generate(): Promise<void> {
    if (generating) return;
    const emailTo = recipientEmail.trim();
    // Every link is outward-facing (anyone with the URL can watch), so viewer
    // links confirm too — not only editor and emailed-code links.
    const windowLabel = (emailTo ? DURATION_OPTIONS : TTL_OPTIONS).find(
      (o) => o.secs === (emailTo ? durationSecs : ttlSecs),
    )?.label;
    const expiry = windowLabel ? windowLabel.replace(' (max)', '') : 'the chosen time';
    const can = role === 'editor' ? 'type commands into this terminal' : 'watch this terminal (read-only)';
    const ok = await confirmOutward({
      verb: emailTo ? 'Send code' : role === 'editor' ? 'Create editor link' : 'Create viewer link',
      title: emailTo ? 'Send the access code?' : role === 'editor' ? 'Create an editor link?' : 'Create a viewer link?',
      where: emailTo
        ? `A share link for this session; a 6-digit code is emailed to ${emailTo}`
        : 'A new share link for this session (anyone with the URL can open it)',
      what: `${role === 'editor' ? 'Editor' : 'Viewer'} access — they can ${can}. Expires after ${expiry}; you can revoke it at any time.`,
      who: emailTo ? `${emailTo}, who must enter the code to attach.` : 'Anyone who gets the link.',
      danger: role === 'editor',
    });
    if (!ok) return;
    generating = true;
    mintedUrl = null;
    try {
      const email = recipientEmail.trim();
      const body: CreateShareReq = {
        role,
        // Plain share: use ttlSecs; OTP share: duration_secs governs the window.
        ...(email
          ? { recipient_email: email, duration_secs: Math.min(durationSecs, 43200) }
          : { ttl_secs: ttlSecs }),
        label: label.trim() || undefined,
      };
      const resp = await api.post<CreateShareResp>(
        `/sessions/${encodeURIComponent(sessionId)}/share`,
        body,
      );
      mintedUrl = resp.url;
      // Older daemons omit the flag — treat absence as reachable (old behaviour).
      mintedRemote = resp.reachable_remotely !== false;
      mintedLan = resp.reach === 'lan';
      mintedShareId = resp.info.id;
      // Optimistically prepend the new share to the list.
      shares = [resp.info, ...shares];
      toasts.success(
        'Share link created',
        email
          ? `A 6-digit code was emailed to ${email}. The recipient must enter it before attaching.`
          : mintedRemote
            ? 'Copy the URL or scan the QR code.'
            : 'This link only works on this Mac — see the note below.',
      );
    } catch (e) {
      toastError('Couldn’t create share link', e);
    } finally {
      generating = false;
    }
  }

  // ── copy URL to clipboard ────────────────────────────────────────────────────
  async function copyUrl(): Promise<void> {
    if (!mintedUrl) return;
    try {
      await copyTextOrThrow(mintedUrl);
      toasts.success('Link copied', 'The share link is in your clipboard.');
    } catch {
      toasts.error('Couldn’t copy the link', 'The clipboard isn’t available here. Select the link and copy it manually.');
    }
  }

  // ── revoke a single share ────────────────────────────────────────────────────
  async function revoke(shareId: string): Promise<void> {
    const ok = await confirmer.ask(
      'Revoke this share link? Guests attached through it are disconnected, and the link can’t be used again.',
      { title: 'Revoke share link', confirmLabel: 'Revoke link', danger: true },
    );
    if (!ok) return;
    revoking = { ...revoking, [shareId]: true };
    try {
      await api.del(`/auth/shares/${encodeURIComponent(shareId)}`);
      shares = shares.filter((s) => s.id !== shareId);
      // If the just-revoked share is the one we just minted, clear the URL panel
      // (the old check looked the id up AFTER filtering it out, so revoking any
      // older link also wiped the fresh URL + QR).
      if (shareId === mintedShareId) {
        mintedUrl = null;
        mintedShareId = null;
      }
      toasts.success('Share revoked');
    } catch (e) {
      toastError('Couldn’t revoke the share link', e);
    } finally {
      const next = { ...revoking };
      delete next[shareId];
      revoking = next;
    }
  }

  // ── revoke all shares ────────────────────────────────────────────────────────
  // "Revoke all" sits under THIS session's list, so it revokes exactly those
  // links, one DELETE each. `POST /auth/shares/revoke-all` is caller-wide — it
  // silently killed the links of every OTHER session too.
  async function revokeAll(): Promise<void> {
    if (revokingAll || shares.length === 0) return;
    const n = shares.length;
    const ok = await confirmer.ask(
      `Revoke ${n === 1 ? 'the share link' : `all ${n} share links`} for this session? Guests attached through ${n === 1 ? 'it' : 'them'} are disconnected, and ${n === 1 ? 'the link' : 'the links'} can’t be used again.`,
      { title: 'Revoke share links', confirmLabel: n === 1 ? 'Revoke link' : `Revoke ${n} links` },
    );
    if (!ok) return;
    revokingAll = true;
    const failed: string[] = [];
    for (const share of [...shares]) {
      try {
        await api.del(`/auth/shares/${encodeURIComponent(share.id)}`);
        shares = shares.filter((s) => s.id !== share.id);
      } catch (e) {
        failed.push(e instanceof Error ? e.message : String(e));
      }
    }
    if (failed.length === 0) {
      mintedUrl = null;
      toasts.success('Share links revoked');
    } else {
      toasts.error('Some links were not revoked', `${failed.length} of ${n} failed: ${failed[0]}`);
    }
    revokingAll = false;
  }

  // ── helpers ───────────────────────────────────────────────────────────────────
  function fmtExpiry(expiresAt: string): string {
    const d = new Date(expiresAt);
    if (isNaN(d.getTime())) return expiresAt;
    const now = Date.now();
    const diff = d.getTime() - now;
    if (diff <= 0) return 'expired';
    const h = Math.floor(diff / 3_600_000);
    const m = Math.floor((diff % 3_600_000) / 60_000);
    if (h >= 24) return `expires in ~${Math.ceil(h / 24)}d`;
    if (h > 0) return `expires in ${h}h ${m}m`;
    return `expires in ${m}m`;
  }

  // Plain share TTL options (max 24h per base design).
  const TTL_OPTIONS = [
    { label: '1 hour', secs: 3600 },
    { label: '4 hours', secs: 14400 },
    { label: '12 hours', secs: 43200 },
    { label: '24 hours', secs: 86400 },
  ];

  // OTP-gated share duration options (hard max 12h per spec).
  const DURATION_OPTIONS = [
    { label: '30 minutes', secs: 1800 },
    { label: '1 hour', secs: 3600 },
    { label: '4 hours', secs: 14400 },
    { label: '12 hours (max)', secs: 43200 },
  ];
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":`Share this session`,"width":500,onclose,}});
  
   { svelteHTML.createElement("div", { "class":`sm-body`,});
     { svelteHTML.createElement("div", { "class":`sm-row`,});
       { svelteHTML.createElement("label", {   "class":`sm-label`,"for":`sm-role`,});  }
       { svelteHTML.createElement("select", {     "id":`sm-role`,"class":`sm-select`,"bind:value":role,});/*Ωignore_startΩ*/() => role = __sveltets_2_any(null);/*Ωignore_endΩ*/
         { svelteHTML.createElement("option", { "value":`viewer`,});        }
         { svelteHTML.createElement("option", { "value":`editor`,});        }
       }
     }

    
     { svelteHTML.createElement("div", { "class":`sm-row`,});
       { svelteHTML.createElement("label", {   "class":`sm-label`,"for":`sm-recipient`,});
         
         { svelteHTML.createElement("span", { "class":`sm-optional`,});      }
       }
      if(!emailSenderLoading && !hasSender){
         { svelteHTML.createElement("div", { "class":`sm-sender-warn`,});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}
             
           { svelteHTML.createElement("a", {     "href":`#/settings/sharing`,"onclick":(e) => { e.preventDefault(); onclose(); router.go('settings/sharing'); },});
                  
           }
             
         }
      }
       { svelteHTML.createElement("input", {                  "dir":`ltr`,"id":`sm-recipient`,"class":`sm-input`,"type":`email`,"placeholder":`guest@example.com — leave blank for a plain link`,"autocomplete":`off`,"bind:value":recipientEmail,"disabled":!hasSender && !emailSenderLoading,"title":!hasSender && !emailSenderLoading ? 'Set up a verified email sender first (Settings → Sharing)' : undefined,});/*Ωignore_startΩ*/() => recipientEmail = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }

    
    if(isOtpShare){
       { svelteHTML.createElement("div", { "class":`sm-row`,});
         { svelteHTML.createElement("label", {   "class":`sm-label`,"for":`sm-duration`,});    }
         { svelteHTML.createElement("select", {     "id":`sm-duration`,"class":`sm-select`,"bind:value":durationSecs,});/*Ωignore_startΩ*/() => durationSecs = __sveltets_2_any(null);/*Ωignore_endΩ*/
             for(let opt of __sveltets_2_ensureArray(DURATION_OPTIONS)){opt.secs;
             { svelteHTML.createElement("option", { "value":opt.secs,});opt.label; }
          }
         }
         { svelteHTML.createElement("span", { "class":`sm-note`,});
                    
              
         }
       }
    }else{
       { svelteHTML.createElement("div", { "class":`sm-row`,});
         { svelteHTML.createElement("label", {   "class":`sm-label`,"for":`sm-ttl`,});   }
         { svelteHTML.createElement("select", {     "id":`sm-ttl`,"class":`sm-select`,"bind:value":ttlSecs,});/*Ωignore_startΩ*/() => ttlSecs = __sveltets_2_any(null);/*Ωignore_endΩ*/
             for(let opt of __sveltets_2_ensureArray(TTL_OPTIONS)){opt.secs;
             { svelteHTML.createElement("option", { "value":opt.secs,});opt.label; }
          }
         }
       }
    }

     { svelteHTML.createElement("div", { "class":`sm-row`,});
       { svelteHTML.createElement("label", {   "class":`sm-label`,"for":`sm-label`,});  }
       { svelteHTML.createElement("input", {              "dir":`auto`,"id":`sm-label`,"class":`sm-input`,"type":`text`,"placeholder":`e.g. "for Alice"`,"maxlength":80,"bind:value":label,});/*Ωignore_startΩ*/() => label = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }

    if(isOtpShare && hasSender){
       { svelteHTML.createElement("p", { "class":`sm-otp-note`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`mail`,"size":12,}});}
                    
                      
       }
    }

     { svelteHTML.createElement("button", {     "class":`btn primary sm-generate`,"disabled":generating,"onclick":generate,});
      generating ? 'Generating…' : role === 'editor' ? 'Create editor link' : 'Generate link';
     }

    
    if(mintedUrl){
       { svelteHTML.createElement("div", { "class":`sm-result`,});
         { svelteHTML.createElement("div", { "class":`sm-url-row`,});
           { svelteHTML.createElement("span", {   "class":`sm-url`,"title":mintedUrl,});mintedUrl; }
           { svelteHTML.createElement("button", {     "class":`btn sm-copy`,"onclick":copyUrl,"title":`Copy to clipboard`,});
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`fetch`,"size":13,}});}
            
           }
         }

        if(mintedRemote){
           { svelteHTML.createElement("div", { "class":`sm-qr-wrap`,});
             { const $$_canvas4 = svelteHTML.createElement("canvas", {   "class":`sm-qr`,});qrCanvas = $$_canvas4; }
             { svelteHTML.createElement("p", { "class":`sm-qr-hint`,});      }
           }
          if(mintedLan){
            
             { svelteHTML.createElement("p", {     "class":`sm-role-note`,"role":`note`,...__sveltets_2_empty({"data-testid":`share-lan-only`}),});
               { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`info`,"size":12,}});}
                          
             }
          }
        }else{
          
           { svelteHTML.createElement("div", {     "class":`sm-sender-warn sm-local-warn`,"role":`note`,...__sveltets_2_empty({"data-testid":`share-local-only`}),});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}
             { svelteHTML.createElement("span", {});
                     
               { svelteHTML.createElement("a", {     "href":`#/settings/sharing`,"onclick":(e) => { e.preventDefault(); onclose(); router.go('settings/sharing'); },});
                    
               }
                      
             }
           }
        }

        if(role === 'viewer'){
           { svelteHTML.createElement("p", { "class":`sm-role-note`,});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`eye`,"size":12,}});}          
           }
        }else{
           { svelteHTML.createElement("p", { "class":`sm-role-note editor`,});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":12,}});}        
           }
        }
       }
    }

    
     { svelteHTML.createElement("div", { "class":`sm-shares-head`,});
       { svelteHTML.createElement("span", { "class":`sm-section-label`,});   }
       { svelteHTML.createElement("span", { "class":`sm-spacer`,}); }
      if(shares.length > 0){
         { svelteHTML.createElement("button", {       "class":`sm-link-btn danger`,"disabled":revokingAll,"onclick":revokeAll,});
          revokingAll ? 'Revoking…' : 'Revoke all';
         }
      }
     }

     { svelteHTML.createElement("div", { "class":`sm-shares-list`,});
      if((sharesLoading || sharesError) && shares.length === 0){
         { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {            "what":`share links`,"variant":`compact`,"loading":sharesLoading,"error":sharesError,"empty":true,"onretry":loadShares,}});}
      } else if (shares.length === 0){
         { svelteHTML.createElement("div", { "class":`sm-empty`,});       }
      }else{
           for(let share of __sveltets_2_ensureArray(shares)){share.id;
           { svelteHTML.createElement("div", { "class":`sm-share-row`,});
             { svelteHTML.createElement("div", { "class":`sm-share-info`,});
               { svelteHTML.createElement("span", { "class":`sm-share-prefix`,});share.token_prefix;  }
              if(share.label){
                 { svelteHTML.createElement("span", { "class":`sm-share-label`,});share.label; }
              }
               { svelteHTML.createElement("span", {  "class":`sm-share-role`,});share.role === 'editor';
                share.role === 'editor' ? 'Can type' : 'View only';
               }
             }
             { svelteHTML.createElement("div", { "class":`sm-share-meta`,});
               { svelteHTML.createElement("span", {    "class":`sm-share-expiry`,"title":share.dormant
                  ? 'Window lapsed — the link holder can still request a new code until it is revoked'
                  : undefined,});share.dormant ? 'dormant (revivable)' : fmtExpiry(share.expires_at); }
               { svelteHTML.createElement("button", {         "class":`btn sm-revoke-btn`,"disabled":revoking[share.id],"onclick":() => revoke(share.id),"title":`Revoke this link`,});
                revoking[share.id] ? '…' : 'Revoke';
               }
             }
           }
        }
      }
     }
   }
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ShareModal__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ShareModal__SvelteComponent_ = ReturnType<typeof ShareModal__SvelteComponent_>;
/*Ωignore_endΩ*/export default ShareModal__SvelteComponent_;