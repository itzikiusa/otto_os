///<reference types="svelte" />
;
import PageHeader from '../../lib/components/PageHeader.svelte';
import { sectionLabel } from './sections';
import SectionIntro from './SectionIntro.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
import Icon from '../../lib/components/Icon.svelte';
import ActiveShareLinks from './ActiveShareLinks.svelte';
// Settings → Sharing: configure a Gmail App Password sender for email-OTP shares.
// The app password is write-only (never echoed back from the server); the form
// always shows an empty password field so the user can update it without seeing the
// old value.
import { onMount } from 'svelte';
import { api } from '../../lib/api/client';
import type { SetEmailSenderReq, EmailSenderResp } from '../../lib/api/types';
import { toasts } from '../../lib/toast.svelte';
import { loadErrorText } from '../../lib/loadError';
import LoadState from '../../lib/components/LoadState.svelte';
import { guardUnsaved } from '../../lib/leaveGuard';
import { auth } from '../../lib/stores/auth.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // ── state ─────────────────────────────────────────────────────────────────────
  let status = $state<EmailSenderResp | null>(null);
  let loading = $state(true);
  let saving = $state(false);
  /** true while a "Re-verify" SMTP check is running (does NOT update the password). */
  let verifying = $state(false);
  /** Actionable SMTP error message from the last save/verify attempt. */
  let smtpError = $state<string | null>(null);
  /** Whether the password field shows a real input (vs. the ●●●● placeholder). */
  let editingPassword = $state(false);

  // Form fields
  let fGmail = $state('');
  let fPassword = $state('');

  // ── public link domain (share_base_url) ─────────────────────────────────────────
  /** The operator-configured public domain used to build share links + the link
   *  emailed with the OTP code. Empty ⇒ links fall back to the request host. */
  let fBaseUrl = $state('');
  /** The value last loaded/saved — Save stays disabled until the field differs. */
  let savedBaseUrl = $state('');
  let savingBaseUrl = $state(false);
  /** Set when the saved domain couldn't be read: an empty box would otherwise
   *  look like "not set", one Save away from clearing the real value. */
  let baseUrlError = $state('');
  /** Set when the sender status couldn't be read (inline, with Retry). */
  let loadError = $state('');
  /** Inline field errors, shown on Save (not as toasts). */
  let gmailError = $state('');
  let pwError = $state('');

  // ── load current sender on mount ──────────────────────────────────────────────
  onMount(() => {
    void load();
    // The public link domain lives in the daemon settings (`GET|PUT /settings`,
    // root-only): other members never see a card that can only fail.
    if (auth.isRoot) void loadBaseUrl();
  });

  async function load(): Promise<void> {
    loading = true;
    loadError = '';
    try {
      status = await api.get<EmailSenderResp>('/email-sender');
      // Pre-fill the address so the user can update the password without re-typing it.
      if (status.gmail_address) fGmail = status.gmail_address;
    } catch (e) {
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  async function loadBaseUrl(): Promise<void> {
    baseUrlError = '';
    try {
      const settings = await api.get<Record<string, unknown>>('/settings');
      const v = settings.share_base_url;
      fBaseUrl = savedBaseUrl = typeof v === 'string' ? v : '';
    } catch (e) {
      baseUrlError = loadErrorText(e);
    }
  }

  const baseUrlDirty = $derived(fBaseUrl.trim() !== savedBaseUrl.trim());
  // Typed-but-unsaved edits (an app password, or a changed domain) ask
  // before a navigation drops them.
  $effect(() =>
    guardUnsaved(() => !saving && !savingBaseUrl && (fPassword.trim() !== '' || baseUrlDirty), {
      what: 'the sharing settings',
    }),
  );
  const baseUrlInvalid = $derived(fBaseUrl.trim() !== '' && !/^https?:\/\/[^\s/]+/i.test(fBaseUrl.trim()));

  async function saveBaseUrl(): Promise<void> {
    if (baseUrlInvalid) return;
    savingBaseUrl = true;
    try {
      const v = fBaseUrl.trim();
      await api.put('/settings', { share_base_url: v });
      fBaseUrl = savedBaseUrl = v;
      toasts.success('Public link domain saved', v ? 'New share links use this domain.' : 'Share links use the request host again.');
    } catch (e) {
      toasts.error("Couldn’t save the public link domain", loadErrorText(e));
    } finally {
      savingBaseUrl = false;
    }
  }

  // ── save ──────────────────────────────────────────────────────────────────────
  async function save(): Promise<void> {
    const gmail = fGmail.trim();
    const pw = fPassword.trim();
    gmailError = gmail ? '' : 'Enter the Gmail address Otto sends from.';
    pwError = pw.replace(/\s/g, '').length === 16 ? '' : 'Enter the 16-character App Password (spaces are fine).';
    if (gmailError || pwError) return;
    saving = true;
    smtpError = null;
    try {
      const body: SetEmailSenderReq = { gmail_address: gmail, app_password: pw };
      status = await api.put<EmailSenderResp>('/email-sender', body);
      // Clear the password field and exit edit mode — write-only; never echoed back.
      fPassword = '';
      editingPassword = false;
      if (status.verified) {
        toasts.success('Email sender saved', 'Gmail SMTP verified — you can now create OTP-gated share links.');
      } else {
        smtpError = 'SMTP verification failed. Check that (1) the password is exactly 16 characters with no spaces, (2) it was generated for "Mail" not another app, and (3) 2-Step Verification is still enabled on your Google account.';
        toasts.warn('Saved — SMTP unverified', 'The error is shown under the sender address.');
      }
    } catch (e) {
      toasts.error("Couldn’t save the email sender", loadErrorText(e));
    } finally {
      saving = false;
    }
  }

  /** Re-verify SMTP without changing the stored password. */
  async function reverify(): Promise<void> {
    verifying = true;
    smtpError = null;
    try {
      // A dedicated verify route re-checks with the Keychain-stored password
      // (a PUT without `app_password` is rejected — the password is required).
      status = await api.post<EmailSenderResp>('/email-sender/verify', {});
      if (status.verified) {
        toasts.success('SMTP verified', 'Gmail connection is working.');
      } else {
        smtpError = 'Re-verification failed. Your App Password may have been revoked. Generate a new one in Google Account → Security → App passwords, then re-enter it below.';
        toasts.warn('SMTP still unverified', 'The error is shown under the sender address.');
      }
    } catch (e) {
      toasts.error("Couldn’t verify the email sender", loadErrorText(e));
    } finally {
      verifying = false;
    }
  }

  // ── badge helpers ─────────────────────────────────────────────────────────────
  const verifiedBadge = $derived(
    status?.verified ? 'chip ok' : status?.gmail_address ? 'chip chip-warn' : 'chip',
  );
  const verifiedLabel = $derived(
    status?.verified
      ? 'Verified'
      : status?.gmail_address
        ? 'Unverified'
        : 'Not configured',
  );

  /** true when a password is stored server-side (but we never receive it back). */
  const hasStoredPassword = $derived(!!(status?.gmail_address));
;
async () => {

 { svelteHTML.createElement("div", { "class":`settings-section`,});
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {    "title":sectionLabel('sharing'),"subtitle":`Emails one-time codes to guests of shared sessions`,}});}
   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"width":`readable`,}});
   { const $$_ortnInoitceS2C = __sveltets_2_ensureComponent(SectionIntro); new $$_ortnInoitceS2C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});                               SectionIntro}

  
   { const $$_skniLerahSevitcA2C = __sveltets_2_ensureComponent(ActiveShareLinks); new $$_skniLerahSevitcA2C({ target: __sveltets_2_any(), props: {}});}

  
   { svelteHTML.createElement("div", { "class":`section-title`,});  }
   { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {          children:() => { return __sveltets_2_any(0); },"what":`the email sender`,loading,"error":loadError,"empty":!status,"onretry":() => void load(),"rows":2,}});
   { svelteHTML.createElement("div", { "class":`card s-card`,});
     { svelteHTML.createElement("div", { "class":`status-row`,});
       { svelteHTML.createElement("span", {    "class":`status-address`,"title":status?.gmail_address ?? undefined,});!status?.gmail_address;
        status?.gmail_address ?? 'No sender configured';
       }
      
      if(status?.gmail_address){ { svelteHTML.createElement("span", { "class":verifiedBadge,});verifiedLabel; }}
      if(status?.gmail_address && !status.verified){
         { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":verifying,"onclick":reverify,});
          verifying ? 'Verifying…' : 'Re-verify';
         }
      }
     }
    if(smtpError){
       { svelteHTML.createElement("div", {   "class":`smtp-error`,"role":`alert`,});
         { svelteHTML.createElement("strong", {});  } smtpError;
       }
    }

     { svelteHTML.createElement("p", { "class":`card-intro`,});
          { svelteHTML.createElement("strong", {});  } 
       { svelteHTML.createElement("a", {     "href":`https://myaccount.google.com/apppasswords`,"target":`_blank`,"rel":`noopener noreferrer`,});
              
       }
                      
          
     }

     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`es-gmail`,});  }
       { svelteHTML.createElement("input", {                        "dir":`ltr`,"id":`es-gmail`,"class":`input`,"type":`email`,"placeholder":`you@gmail.com`,"autocomplete":`email`,"aria-invalid":!!gmailError,"disabled":hasStoredPassword && !editingPassword,"title":hasStoredPassword && !editingPassword ? 'Replace the app password to change the sender' : undefined,"aria-describedby":hasStoredPassword && !editingPassword ? 'es-gmail-help' : undefined,"bind:value":fGmail,"oninput":() => (gmailError = ''),});/*Ωignore_startΩ*/() => fGmail = __sveltets_2_any(null);/*Ωignore_endΩ*/}
      if(hasStoredPassword && !editingPassword){
         { svelteHTML.createElement("span", {   "id":`es-gmail-help`,"class":`hint`,});            }
      }
      if(gmailError){ { svelteHTML.createElement("span", { "class":`field-error`,});gmailError; }}
     }

     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`es-pw`,});  }
      if(hasStoredPassword && !editingPassword){
        
         { svelteHTML.createElement("div", { "class":`pw-set-row`,});
           { svelteHTML.createElement("span", { "class":`stored`,}); { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`lock`,"size":12,}});}    }
           { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => (editingPassword = true),});  }
         }
      }else{
         { svelteHTML.createElement("input", {                   "id":`es-pw`,"class":`input`,"type":`password`,"placeholder":`xxxx xxxx xxxx xxxx`,"autocomplete":`new-password`,"maxlength":19,"aria-invalid":!!pwError,"bind:value":fPassword,"oninput":() => (pwError = ''),});/*Ωignore_startΩ*/() => fPassword = __sveltets_2_any(null);/*Ωignore_endΩ*/}
        if(pwError){
           { svelteHTML.createElement("span", { "class":`field-error`,});pwError; }
        }else{
           { svelteHTML.createElement("span", { "class":`hint`,});           }
        }
      }
     }

     { svelteHTML.createElement("div", { "class":`form-actions`,});
      if(editingPassword){
         { svelteHTML.createElement("button", {   "class":`btn ghost`,"onclick":() => { editingPassword = false; fPassword = ''; pwError = ''; gmailError = ''; fGmail = status?.gmail_address ?? fGmail; },});  }
      }
       { svelteHTML.createElement("button", {         "class":`btn primary`,"disabled":saving || (hasStoredPassword && !editingPassword),"title":hasStoredPassword && !editingPassword ? 'Replace the app password to save a new one' : undefined,"onclick":save,});
        saving ? 'Saving…' : 'Save and verify';
       }
     }
   }
   LoadState}

  
  if(auth.isRoot){
   { svelteHTML.createElement("div", { "class":`section-title`,});   }
   { svelteHTML.createElement("div", { "class":`card s-card`,});
    if(baseUrlError){
       { svelteHTML.createElement("div", { "class":`domain-error`,});
         { const $$_etatSdaoL4C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL4C({ target: __sveltets_2_any(), props: {        "variant":`compact`,"what":`the saved domain`,"error":baseUrlError,"empty":true,"onretry":() => void loadBaseUrl(),}});}
       }
    }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`es-base-url`,});    }
       { svelteHTML.createElement("input", {                "dir":`ltr`,"id":`es-base-url`,"class":`input`,"type":`url`,"placeholder":`https://otto.example.com`,"aria-invalid":baseUrlInvalid,"disabled":!!baseUrlError,"bind:value":fBaseUrl,});/*Ωignore_startΩ*/() => fBaseUrl = __sveltets_2_any(null);/*Ωignore_endΩ*/}
      if(baseUrlInvalid){
         { svelteHTML.createElement("span", { "class":`field-error`,});         }
      }else{
         { svelteHTML.createElement("span", { "class":`hint`,});
                            
                
         }
      }
     }
     { svelteHTML.createElement("div", { "class":`form-actions`,});
       { svelteHTML.createElement("button", {     "class":`btn`,"disabled":savingBaseUrl || !baseUrlDirty || baseUrlInvalid || !!baseUrlError,"onclick":saveBaseUrl,});
        savingBaseUrl ? 'Saving…' : 'Save domain';
       }
     }
   }
  }

  
   { svelteHTML.createElement("div", { "class":`section-title`,});   }
   { svelteHTML.createElement("ol", { "class":`how-list`,});
     { svelteHTML.createElement("li", {});         { svelteHTML.createElement("strong", {});  }         }
     { svelteHTML.createElement("li", {});             }
     { svelteHTML.createElement("li", {});        { svelteHTML.createElement("strong", {});  }         }
   }
   PageBody}
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const EmailSenderSetup__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type EmailSenderSetup__SvelteComponent_ = ReturnType<typeof EmailSenderSetup__SvelteComponent_>;
/*Ωignore_endΩ*/export default EmailSenderSetup__SvelteComponent_;