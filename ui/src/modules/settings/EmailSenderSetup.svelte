<script lang="ts">
  // Settings → Sharing: configure a Gmail App Password sender for email-OTP shares.
  // The app password is write-only (never echoed back from the server); the form
  // always shows an empty password field so the user can update it without seeing the
  // old value.
  import { onMount } from 'svelte';
  import { api } from '../../lib/api/client';
  import type { SetEmailSenderReq, EmailSenderResp } from '../../lib/api/types';
  import { toasts } from '../../lib/toast.svelte';

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

  // ── load current sender on mount ──────────────────────────────────────────────
  onMount(() => {
    void load();
  });

  async function load(): Promise<void> {
    loading = true;
    try {
      status = await api.get<EmailSenderResp>('/email-sender');
      // Pre-fill the address so the user can update the password without re-typing it.
      if (status.gmail_address) fGmail = status.gmail_address;
    } catch {
      // Non-fatal: just show the empty form.
    } finally {
      loading = false;
    }
  }

  // ── save ──────────────────────────────────────────────────────────────────────
  async function save(): Promise<void> {
    const gmail = fGmail.trim();
    const pw = fPassword.trim();
    if (!gmail) {
      toasts.error('Missing address', 'Enter your Gmail address.');
      return;
    }
    if (!pw) {
      toasts.error('Missing app password', 'Enter the 16-character App Password.');
      return;
    }
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
        toasts.warn('Saved — SMTP unverified', 'See the error hint below the form.');
      }
    } catch (e) {
      toasts.error('Save failed', e instanceof Error ? e.message : String(e));
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
        toasts.warn('SMTP still unverified', 'See the error hint below.');
      }
    } catch (e) {
      toasts.error('Verify failed', e instanceof Error ? e.message : String(e));
    } finally {
      verifying = false;
    }
  }

  // ── badge helpers ─────────────────────────────────────────────────────────────
  const verifiedBadge = $derived(
    status?.verified ? 'badge verified' : status?.gmail_address ? 'badge unverified' : 'badge none',
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

<div class="page">
  <div class="page-header">
    <div>
      <h1>Sharing — Email Sender</h1>
      <div class="sub">
        Configure a Gmail sender so Otto can email one-time codes to guests before
        they attach to a shared session.
      </div>
    </div>
  </div>

  <!-- ── Status card ── -->
  <div class="section-title">Current sender</div>
  <div class="card pad">
    {#if loading}
      <span class="dim">Loading…</span>
    {:else}
      <div class="status-row">
        <span class="status-address">
          {status?.gmail_address ?? 'No sender configured'}
        </span>
        <span class={verifiedBadge}>{verifiedLabel}</span>
        {#if status?.gmail_address && !status.verified}
          <button class="btn small" disabled={verifying} onclick={reverify}>
            {verifying ? 'Verifying…' : 'Re-verify'}
          </button>
        {/if}
      </div>
      {#if status?.gmail_address}
        <p class="hint dim" style="margin-top: 6px">
          App password: <span class="pw-placeholder">●●●●&nbsp;●●●●&nbsp;●●●●&nbsp;●●●●</span>
          (stored in Keychain, never displayed)
        </p>
      {/if}
      {#if smtpError}
        <div class="smtp-error">
          <strong>SMTP error:</strong> {smtpError}
        </div>
      {/if}
    {/if}
  </div>

  <!-- ── Setup form ── -->
  <div class="section-title">Set up or update</div>
  <div class="card pad">
    <p class="card-intro dim">
      Create a Gmail <strong>App Password</strong> in
      <a href="https://myaccount.google.com/apppasswords" target="_blank" rel="noopener noreferrer">
        Google Account → Security → App passwords
      </a>
      (requires 2-Step Verification). Paste the 16-character password below.
      Otto stores it in the macOS Keychain — it is never written to disk or the DB.
    </p>

    <div class="field">
      <label for="es-gmail">Gmail address</label>
      <input
        id="es-gmail"
        class="input"
        type="email"
        placeholder="you@gmail.com"
        autocomplete="email"
        bind:value={fGmail}
      />
    </div>

    <div class="field">
      <label for="es-pw">
        App Password
        {#if hasStoredPassword && !editingPassword}
          <button class="inline-link" onclick={() => (editingPassword = true)}>Change</button>
        {/if}
      </label>
      {#if hasStoredPassword && !editingPassword}
        <!-- Placeholder affordance so the user knows a password is set. -->
        <div class="pw-set-row">
          <span class="pw-placeholder input-like">●●●●&nbsp;●●●●&nbsp;●●●●&nbsp;●●●●</span>
          <span class="hint dim">(16-char App Password stored in Keychain)</span>
        </div>
      {:else}
        <input
          id="es-pw"
          class="input"
          type="password"
          placeholder="xxxx xxxx xxxx xxxx  (16 characters)"
          autocomplete="new-password"
          maxlength={19}
          bind:value={fPassword}
        />
        <span class="hint dim">
          Enter exactly 16 characters (groups of 4, with or without spaces).
          Never use your Google account password — create a dedicated App Password.
        </span>
      {/if}
    </div>

    <button class="btn primary" disabled={saving} onclick={save}>
      {saving ? 'Saving…' : 'Save and verify'}
    </button>
  </div>

  <!-- ── How it works ── -->
  <div class="section-title">How it works</div>
  <div class="card pad">
    <ol class="how-list dim">
      <li>
        When you create a share link with a <strong>Recipient email</strong>, Otto
        emails a 6-digit code to that address.
      </li>
      <li>
        The guest opens the link and must enter the code before the terminal
        attaches. A leaked link alone is useless without the mailbox.
      </li>
      <li>
        Sessions expire (max 12 hours). Use <strong>Extend</strong> to re-send a
        fresh code to the same locked address.
      </li>
    </ol>
  </div>
</div>

<style>
  .status-row {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .status-address {
    font-size: 13px;
    color: var(--text);
    font-family: monospace;
  }

  /* Verified / unverified / not-configured badge */
  .badge {
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    padding: 2px 8px;
    border-radius: 999px;
    border: 1px solid var(--border);
    flex-shrink: 0;
  }
  .badge.verified {
    color: #22c55e;
    border-color: color-mix(in srgb, #22c55e 35%, transparent);
    background: color-mix(in srgb, #22c55e 10%, transparent);
  }
  .badge.unverified {
    color: #f59e0b;
    border-color: color-mix(in srgb, #f59e0b 35%, transparent);
    background: color-mix(in srgb, #f59e0b 10%, transparent);
  }
  .badge.none {
    color: var(--text-dim);
  }

  .card-intro {
    font-size: 12.5px;
    line-height: 1.55;
    margin: 0 0 14px;
  }
  .card-intro a {
    color: var(--accent);
    text-decoration: underline;
  }

  .how-list {
    font-size: 12.5px;
    line-height: 1.6;
    margin: 0;
    padding-left: 18px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .pw-placeholder {
    font-family: monospace;
    color: var(--text-dim);
    letter-spacing: 0.05em;
  }

  .pw-set-row {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }

  .input-like {
    display: inline-block;
    padding: 6px 10px;
    border-radius: var(--radius-s, 5px);
    border: 1px solid var(--border);
    background: var(--surface-2);
    font-size: 13px;
  }

  .inline-link {
    border: none;
    background: none;
    color: var(--accent);
    font-size: 11.5px;
    cursor: pointer;
    padding: 0 0 0 6px;
    text-decoration: underline;
  }

  .smtp-error {
    margin-top: 8px;
    padding: 8px 10px;
    border-radius: var(--radius-s, 5px);
    background: color-mix(in srgb, #ef4444 10%, transparent);
    border: 1px solid color-mix(in srgb, #ef4444 30%, transparent);
    color: var(--text);
    font-size: 12.5px;
    line-height: 1.5;
  }
</style>
