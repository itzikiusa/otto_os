<script lang="ts">
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import { sectionLabel } from './sections';
  import SectionIntro from './SectionIntro.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import Icon from '../../lib/components/Icon.svelte';
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
    void loadBaseUrl();
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
      toasts.error("Couldn't save the public link domain", loadErrorText(e));
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
      toasts.error("Couldn't save the email sender", loadErrorText(e));
    } finally {
      saving = false;
    }
  }

  /** Re-verify SMTP without changing the stored password. */
  async function reverify(): Promise<void> {
    verifying = true;
    smtpError = null;
    try {
      // PUT with no password triggers a re-check using the Keychain-stored value.
      status = await api.put<EmailSenderResp>('/email-sender', { gmail_address: fGmail.trim() });
      if (status.verified) {
        toasts.success('SMTP verified', 'Gmail connection is working.');
      } else {
        smtpError = 'Re-verification failed. Your App Password may have been revoked. Generate a new one in Google Account → Security → App passwords, then re-enter it below.';
        toasts.warn('SMTP still unverified', 'The error is shown under the sender address.');
      }
    } catch (e) {
      toasts.error("Couldn't verify the email sender", loadErrorText(e));
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
</script>

<div class="settings-section">
  <PageHeader title={sectionLabel('sharing')} subtitle="Emails one-time codes to guests of shared sessions" />
  <PageBody width="readable">
  <SectionIntro>Configure a Gmail sender so Otto can email a one-time code to each guest before they attach to a shared session. A leaked link alone is useless without the guest's mailbox.</SectionIntro>

  <!-- ── Gmail sender: status + setup form in one card ── -->
  <div class="section-title">Gmail sender</div>
  <LoadState what="the email sender" {loading} error={loadError} empty={!status} onretry={() => void load()} rows={2}>
  <div class="card s-card">
    <div class="status-row">
      <span class="status-address" class:unset={!status?.gmail_address} title={status?.gmail_address ?? undefined}>
        {status?.gmail_address ?? 'No sender configured'}
      </span>
      <!-- "Not configured" beside "No sender configured" said it twice. -->
      {#if status?.gmail_address}<span class={verifiedBadge}>{verifiedLabel}</span>{/if}
      {#if status?.gmail_address && !status.verified}
        <button class="btn small" disabled={verifying} onclick={reverify}>
          {verifying ? 'Verifying…' : 'Re-verify'}
        </button>
      {/if}
    </div>
    {#if smtpError}
      <div class="smtp-error" role="alert">
        <strong>SMTP error:</strong> {smtpError}
      </div>
    {/if}

    <p class="card-intro">
      Create a Gmail <strong>App Password</strong> in
      <a href="https://myaccount.google.com/apppasswords" target="_blank" rel="noopener noreferrer">
        Google Account → Security → App passwords
      </a>
      (needs 2-Step Verification) and paste it below. Otto keeps it in the macOS Keychain — never on
      disk or in its database.
    </p>

    <div class="field">
      <label for="es-gmail">Gmail address</label>
      <input
        id="es-gmail"
        class="input"
        type="email"
        placeholder="you@gmail.com"
        autocomplete="email"
        aria-invalid={!!gmailError}
        disabled={hasStoredPassword && !editingPassword}
        title={hasStoredPassword && !editingPassword ? 'Replace the app password to change the sender' : undefined}
        bind:value={fGmail}
        oninput={() => (gmailError = '')}
      />
      {#if gmailError}<span class="field-error">{gmailError}</span>{/if}
    </div>

    <div class="field">
      <label for="es-pw">App password</label>
      {#if hasStoredPassword && !editingPassword}
        <!-- Write-only: a password is stored, but never sent back. -->
        <div class="pw-set-row">
          <span class="stored"><Icon name="lock" size={12} /> Stored in Keychain</span>
          <button class="btn small ghost" onclick={() => (editingPassword = true)}>Replace…</button>
        </div>
      {:else}
        <input
          id="es-pw"
          class="input"
          type="password"
          placeholder="xxxx xxxx xxxx xxxx"
          autocomplete="new-password"
          maxlength={19}
          aria-invalid={!!pwError}
          bind:value={fPassword}
          oninput={() => (pwError = '')}
        />
        {#if pwError}
          <span class="field-error">{pwError}</span>
        {:else}
          <span class="hint">16 characters, with or without spaces. Never your Google account password.</span>
        {/if}
      {/if}
    </div>

    <div class="form-actions">
      {#if editingPassword}
        <button class="btn ghost" onclick={() => { editingPassword = false; fPassword = ''; pwError = ''; gmailError = ''; fGmail = status?.gmail_address ?? fGmail; }}>Cancel</button>
      {/if}
      <button
        class="btn primary"
        disabled={saving || (hasStoredPassword && !editingPassword)}
        title={hasStoredPassword && !editingPassword ? 'Replace the app password to save a new one' : undefined}
        onclick={save}
      >
        {saving ? 'Saving…' : 'Save and verify'}
      </button>
    </div>
  </div>
  </LoadState>

  <!-- ── Public link domain ── -->
  <div class="section-title">Public link domain</div>
  <div class="card s-card">
    {#if baseUrlError}
      <div class="inline-error" role="alert">
        <span>Couldn't read the saved domain: {baseUrlError}</span>
        <button class="btn small" onclick={() => void loadBaseUrl()}>Retry</button>
      </div>
    {/if}
    <div class="field">
      <label for="es-base-url">Domain for share links</label>
      <input
        id="es-base-url"
        class="input"
        type="url"
        placeholder="https://otto.example.com"
        aria-invalid={baseUrlInvalid}
        disabled={!!baseUrlError}
        bind:value={fBaseUrl}
      />
      {#if baseUrlInvalid}
        <span class="field-error">Start with https:// (or http://) followed by the host.</span>
      {:else}
        <span class="hint">
          Used in share links and in the link emailed with the code. Leave empty to use the host the
          request came in on (127.0.0.1 by default).
        </span>
      {/if}
    </div>
    <div class="form-actions">
      <button class="btn" disabled={savingBaseUrl || !baseUrlDirty || baseUrlInvalid || !!baseUrlError} onclick={saveBaseUrl}>
        {savingBaseUrl ? 'Saving…' : 'Save domain'}
      </button>
    </div>
  </div>

  <!-- ── How it works ── -->
  <div class="section-title">How it works</div>
  <ol class="how-list">
    <li>When you create a share link with a <strong>Recipient email</strong>, Otto emails a 6-digit code to that address.</li>
    <li>The guest opens the link and enters the code before the terminal attaches.</li>
    <li>Shares expire after at most 12 hours. <strong>Extend</strong> re-sends a fresh code to the same address.</li>
  </ol>
  </PageBody>
</div>

<style>
  /* Section chrome: shared PageHeader bar + scrolling PageBody. */
  .settings-section {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .s-card {
    padding: 14px 16px;
    max-width: var(--settings-col);
    margin-bottom: 8px;
  }
  .status-row {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    min-width: 0;
  }
  .status-address {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-m);
    font-weight: 600;
    color: var(--text);
  }
  .status-address.unset {
    font-weight: 500;
    color: var(--text-dim);
  }
  .chip-warn {
    color: var(--warning);
    border-color: color-mix(in srgb, var(--warning) 35%, transparent);
    background: var(--warning-soft);
  }
  .card-intro {
    font-size: var(--fs-s);
    line-height: 1.5;
    color: var(--text-dim);
    margin: 12px 0;
    padding-top: 12px;
    border-top: 1px solid var(--border);
  }
  .card-intro a {
    color: var(--accent-text);
    text-decoration: underline;
  }
  .how-list {
    font-size: var(--fs-s);
    line-height: 1.6;
    color: var(--text-dim);
    margin: 0;
    max-width: var(--settings-col);
    padding-inline-start: 18px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .pw-set-row {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    min-height: 27px;
  }
  .stored {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .form-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .field-error {
    font-size: var(--fs-xs);
    color: var(--danger);
  }
  .inline-error {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 10px;
    font-size: var(--fs-s);
    color: var(--danger);
  }
  .smtp-error {
    margin-top: 10px;
    padding: 8px 10px;
    border-radius: var(--radius-s);
    background: var(--danger-soft);
    border: 1px solid color-mix(in srgb, var(--danger) 30%, transparent);
    color: var(--text);
    font-size: var(--fs-s);
    line-height: 1.5;
  }
</style>
