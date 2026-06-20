<script lang="ts">
  // Full-screen guest view for a scoped share link (#/s/<sessionId>).
  // No rail, no navigator, no right panel — just the session header + terminal.
  //
  // The share token was captured from the URL fragment by the router (Task 3.1)
  // and stored in-memory; we read it here via getShareToken(sessionId).
  // All API calls use the scoped token directly (not the owner login token).
  //
  // Task 7.5: Email-OTP gate. When the session load returns a 403 with the
  // "email-OTP verification" signal, we show an OTP entry screen before attaching.
  // After verification, `getSharedSession` is retried. An "Extend" control allows
  // re-sending a fresh OTP to the locked original recipient.
  import { onMount } from 'svelte';
  import Terminal from '../../lib/components/Terminal.svelte';
  import { getSharedSession, openShareTerminalWs, verifyShareOtp, extendShare } from '../../lib/api/share';
  import { getShareToken } from '../../lib/router.svelte';
  import type { Session, SessionStatus } from '../../lib/api/types';
  import { ApiError } from '../../lib/api/client';
  import { ui } from '../../lib/stores/ui.svelte';

  interface Props {
    sessionId: string;
  }
  let { sessionId }: Props = $props();

  // The share token captured by the router from the URL fragment.
  // If it's missing the link is invalid / already stripped without a prior capture.
  const token = $derived(getShareToken(sessionId));

  // ── view states ────────────────────────────────────────────────────────────
  // 'loading'  — initial REST call in-flight
  // 'otp'      — OTP required (403 otp_pending); show the code entry form
  // 'ok'       — session loaded; show the terminal
  // 'error'    — irrecoverable error
  // 'extend'   — terminal ended, show "Extend" prompt to re-send OTP
  type ViewState = 'loading' | 'otp' | 'ok' | 'error' | 'extend';
  let viewState = $state<ViewState>('loading');

  let session = $state<Session | null>(null);
  let loadError = $state<string | null>(null);
  let liveStatus = $state<SessionStatus | null>(null);

  // ── OTP form state ─────────────────────────────────────────────────────────
  let otpInput = $state('');
  let otpError = $state<string | null>(null);
  let otpBusy = $state(false);
  let extendBusy = $state(false);
  let extendSent = $state(false);

  // ── load session (or detect OTP gate) ────────────────────────────────────
  async function loadSession(): Promise<void> {
    const t = token;
    if (!t) return;
    viewState = 'loading';
    loadError = null;
    session = null;

    try {
      session = await getSharedSession(sessionId, t);
      viewState = 'ok';
    } catch (e: unknown) {
      if (e instanceof ApiError && e.status === 403 && isOtpPending(e)) {
        // The share requires email-OTP verification before attaching.
        viewState = 'otp';
      } else {
        loadError = e instanceof Error ? e.message : String(e);
        viewState = 'error';
      }
    }
  }

  /** Recognise the OTP-pending 403 from the daemon's feature guard. */
  function isOtpPending(e: ApiError): boolean {
    // The daemon renders: {"code":"forbidden","message":"share requires email-OTP verification"}
    return e.message.includes('email-OTP') || e.message.includes('otp');
  }

  $effect(() => {
    const t = token;
    if (!t) return;
    // Re-run the load whenever the token changes (first mount or after extend).
    void loadSession();
  });

  // ── OTP verification ───────────────────────────────────────────────────────
  async function submitOtp(): Promise<void> {
    const t = token;
    if (!t || otpBusy) return;
    const code = otpInput.trim();
    if (code.length !== 6 || !/^\d{6}$/.test(code)) {
      otpError = 'Please enter the 6-digit code from your email.';
      return;
    }
    otpBusy = true;
    otpError = null;
    try {
      const resp = await verifyShareOtp(t, code);
      if (resp.verified) {
        // OTP accepted — reload the session (now unblocked).
        otpInput = '';
        await loadSession();
      } else {
        otpError = 'Incorrect code. Please try again.';
      }
    } catch (e: unknown) {
      if (e instanceof ApiError && e.status === 429) {
        otpError = 'Too many attempts. Please wait before trying again.';
      } else {
        otpError = e instanceof Error ? e.message : 'Verification failed. Please try again.';
      }
    } finally {
      otpBusy = false;
    }
  }

  // ── Extend (re-send OTP to locked recipient) ──────────────────────────────
  async function requestExtend(): Promise<void> {
    const t = token;
    if (!t || extendBusy) return;
    extendBusy = true;
    extendSent = false;
    otpError = null;
    try {
      await extendShare(t);
      extendSent = true;
      // After extend, show the OTP prompt again for the fresh code.
      otpInput = '';
      viewState = 'otp';
    } catch (e: unknown) {
      if (e instanceof ApiError && e.status === 429) {
        otpError = 'Too many extend attempts. Please wait before trying again.';
      } else {
        otpError = e instanceof Error ? e.message : 'Could not re-send the code. Please try again.';
      }
    } finally {
      extendBusy = false;
    }
  }

  // Called by Terminal when the WS status indicates the session ended or
  // the share OTP gate is re-engaged (the terminal can no longer attach).
  function onTermStatus(s: SessionStatus): void {
    liveStatus = s;
  }

  // When the terminal emits 'exited' or 'reconnectable', offer the Extend
  // control so the guest can request a fresh window.
  const termEnded = $derived(liveStatus === 'exited' || liveStatus === 'reconnectable');

  // Effective status for the header badge.
  const status = $derived<SessionStatus>(liveStatus ?? session?.status ?? 'idle');

  // Is this a viewer share (read-only)?  Safe default: yes (enforcement is in the daemon).
  let isViewer = $state(true);

  const statusLabel: Record<SessionStatus, string> = {
    running: 'running',
    working: 'working',
    idle: 'idle',
    exited: 'exited',
    reconnectable: 'reconnectable',
  };

  // Allow submitting OTP form via Enter key.
  function onOtpKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter') void submitOtp();
  }
</script>

<!-- ── No token ─────────────────────────────────────────────────────── -->
{#if !token}
  <div class="share-error" style={`zoom:${ui.zoom}`}>
    <div class="error-card">
      <div class="error-icon">&#9888;</div>
      <h2>Link invalid or expired</h2>
      <p>
        This share link is missing a token or has already expired.
        Ask the owner to send you a new link.
      </p>
    </div>
  </div>

<!-- ── Loading ──────────────────────────────────────────────────────── -->
{:else if viewState === 'loading'}
  <div class="share-error" style={`zoom:${ui.zoom}`}>
    <div class="error-card">
      <div class="sp-spinner" aria-label="Loading…"></div>
      <p class="dim">Connecting…</p>
    </div>
  </div>

<!-- ── OTP entry screen ──────────────────────────────────────────────── -->
{:else if viewState === 'otp'}
  <div class="share-error" style={`zoom:${ui.zoom}`}>
    <div class="error-card otp-card">
      <div class="otp-icon">&#9993;</div>
      <h2>Enter your access code</h2>
      <p>
        A 6-digit code was emailed to you. Enter it below to access the shared session.
        {#if extendSent}
          <br /><strong>A fresh code has been sent.</strong>
        {/if}
      </p>
      <div class="otp-input-row">
        <input
          class="otp-input"
          type="text"
          inputmode="numeric"
          pattern="[0-9]*"
          maxlength="6"
          placeholder="000000"
          autocomplete="one-time-code"
          bind:value={otpInput}
          onkeydown={onOtpKeydown}
          aria-label="One-time access code"
        />
        <button class="btn primary" disabled={otpBusy} onclick={submitOtp}>
          {otpBusy ? 'Verifying…' : 'Verify'}
        </button>
      </div>
      {#if otpError}
        <p class="otp-error">{otpError}</p>
      {/if}
      <div class="otp-extend-row">
        <span class="dim">Code expired or not received?</span>
        <button class="otp-link" disabled={extendBusy} onclick={requestExtend}>
          {extendBusy ? 'Sending…' : 'Re-send code'}
        </button>
      </div>
    </div>
  </div>

<!-- ── Error (irrecoverable) ─────────────────────────────────────────── -->
{:else if viewState === 'error'}
  <div class="share-error" style={`zoom:${ui.zoom}`}>
    <div class="error-card">
      <div class="error-icon">&#9888;</div>
      <h2>Could not load session</h2>
      <p>{loadError}</p>
      <p class="hint">The share link may have been revoked or may have expired.</p>
    </div>
  </div>

<!-- ── Main guest view: header + terminal (+ optional Extend overlay) ── -->
{:else}
  <div class="share-root" style={`zoom:${ui.zoom}`}>
    <header class="share-header">
      <span class="session-title">{session?.title ?? 'Loading…'}</span>
      <span class="header-spacer"></span>
      {#if session}
        <span class="status-badge status-{status}">{statusLabel[status] ?? status}</span>
      {/if}
      {#if isViewer}
        <span class="ro-badge" title="Viewer share — input disabled">read-only</span>
      {/if}
    </header>

    <div class="terminal-fill">
      {#if session}
        <!-- Pass shareToken so Terminal opens the WS with the otto-bearer
             subprotocol (Task 3.2). readOnly mirrors the viewer badge. -->
        <Terminal
          {sessionId}
          readOnly={isViewer}
          forceDark
          shareToken={token}
          onstatus={onTermStatus}
        />
      {:else}
        <div class="connecting-overlay">
          <span>Connecting…</span>
        </div>
      {/if}

      <!-- Extend overlay — shown when the terminal session has ended so the
           guest can request a fresh OTP window without reloading the page. -->
      {#if termEnded}
        <div class="extend-overlay">
          <div class="extend-card">
            <div class="extend-icon">&#8987;</div>
            <h3>Session window ended</h3>
            <p>
              Request a new access code to be emailed to the original recipient.
              Once you receive it, enter it below to re-attach.
            </p>
            {#if otpError}
              <p class="otp-error">{otpError}</p>
            {/if}
            <button class="btn primary" disabled={extendBusy} onclick={requestExtend}>
              {extendBusy ? 'Sending…' : 'Extend session'}
            </button>
          </div>
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  /* ---- error / OTP state ---- */
  .share-error {
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--bg);
  }
  .error-card {
    max-width: 380px;
    width: 92vw;
    padding: 32px 28px;
    border-radius: var(--radius-l);
    background: var(--surface);
    border: 1px solid var(--border);
    text-align: center;
    display: flex;
    flex-direction: column;
    gap: 12px;
    align-items: center;
  }
  .otp-card {
    max-width: 400px;
  }
  .error-icon {
    font-size: 32px;
    color: var(--status-exited);
  }
  .otp-icon {
    font-size: 36px;
    color: var(--accent);
  }
  .error-card h2 {
    font-size: 17px;
    font-weight: 600;
    margin: 0;
    color: var(--text);
  }
  .error-card p {
    font-size: 13px;
    color: var(--text-dim);
    margin: 0;
    line-height: 1.5;
  }
  .error-card .hint {
    font-size: 11px;
    opacity: 0.7;
  }
  .dim {
    color: var(--text-dim);
  }

  /* ── OTP input ── */
  .otp-input-row {
    display: flex;
    gap: 8px;
    width: 100%;
    justify-content: center;
  }
  .otp-input {
    width: 140px;
    text-align: center;
    font-size: 22px;
    font-family: monospace;
    letter-spacing: 0.18em;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    color: var(--text);
    outline: none;
    transition: border-color 120ms;
  }
  .otp-input:focus {
    border-color: var(--accent);
  }
  .otp-error {
    font-size: 12px;
    color: #ef4444;
    margin: 0;
  }
  .otp-extend-row {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
    flex-wrap: wrap;
    justify-content: center;
  }
  .otp-link {
    border: none;
    background: transparent;
    color: var(--accent);
    font-size: 12px;
    cursor: pointer;
    padding: 2px 4px;
    text-decoration: underline;
  }
  .otp-link:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  /* ── Spinner ── */
  .sp-spinner {
    width: 28px;
    height: 28px;
    border: 3px solid var(--border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  /* ---- main guest shell ---- */
  .share-root {
    height: 100%;
    display: flex;
    flex-direction: column;
    background: var(--bg);
  }

  /* Slim header — intentionally minimal for mobile. */
  .share-header {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
    min-height: 36px;
  }
  .session-title {
    font-size: 13px;
    font-weight: 500;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .header-spacer {
    flex: 1;
  }

  /* Status pill — mirrors the palette chip colours. */
  .status-badge {
    font-size: 10px;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    padding: 2px 8px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .status-badge.status-running  { color: var(--status-working); }
  .status-badge.status-working  { color: var(--status-working); }
  .status-badge.status-idle     { color: var(--text-dim); }
  .status-badge.status-exited   { color: var(--status-exited); }

  /* Read-only badge — subtle, top-right inside the header. */
  .ro-badge {
    font-size: 10px;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--text-dim);
    background: var(--surface-2);
    border: 1px solid var(--border);
    padding: 2px 7px;
    border-radius: 999px;
    flex-shrink: 0;
    opacity: 0.85;
  }

  /* Terminal fills the remaining space. */
  .terminal-fill {
    flex: 1;
    min-height: 0;
    position: relative;
  }

  /* Skeleton while waiting for the session REST call. */
  .connecting-overlay {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--text-dim);
    font-size: 13px;
    background: var(--bg);
  }

  /* ── Extend overlay (shown when terminal session ended) ── */
  .extend-overlay {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background: color-mix(in srgb, var(--bg) 88%, transparent);
    backdrop-filter: blur(4px);
    z-index: 10;
  }
  .extend-card {
    max-width: 360px;
    width: 92vw;
    padding: 28px 24px;
    border-radius: var(--radius-l);
    background: var(--surface);
    border: 1px solid var(--border);
    text-align: center;
    display: flex;
    flex-direction: column;
    gap: 12px;
    align-items: center;
  }
  .extend-icon {
    font-size: 30px;
    color: var(--text-dim);
  }
  .extend-card h3 {
    font-size: 16px;
    font-weight: 600;
    margin: 0;
    color: var(--text);
  }
  .extend-card p {
    font-size: 13px;
    color: var(--text-dim);
    margin: 0;
    line-height: 1.5;
  }
</style>
