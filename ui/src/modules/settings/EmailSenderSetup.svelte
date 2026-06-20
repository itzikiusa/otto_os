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
    try {
      const body: SetEmailSenderReq = { gmail_address: gmail, app_password: pw };
      status = await api.put<EmailSenderResp>('/email-sender', body);
      // Clear password field — it's write-only; never echoed back from the server.
      fPassword = '';
      toasts.success(
        'Email sender saved',
        status.verified
          ? 'Gmail SMTP verified — you can now create OTP-gated share links.'
          : 'Saved, but SMTP verification failed. Check your App Password.',
      );
    } catch (e) {
      toasts.error('Save failed', e instanceof Error ? e.message : String(e));
    } finally {
      saving = false;
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
      </div>
      {#if status?.gmail_address && !status.verified}
        <p class="hint dim">
          SMTP verification failed. Check that your App Password is correct and
          that you have 2-Step Verification enabled on your Google account.
        </p>
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
      <label for="es-pw">App Password</label>
      <input
        id="es-pw"
        class="input"
        type="password"
        placeholder="xxxx xxxx xxxx xxxx"
        autocomplete="new-password"
        bind:value={fPassword}
      />
      <span class="hint dim">
        16 characters, usually shown in groups of four. Never use your Google account
        password here — create a dedicated App Password.
      </span>
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
</style>
