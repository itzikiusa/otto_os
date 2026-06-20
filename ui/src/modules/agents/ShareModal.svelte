<script lang="ts">
  // "Share this session" modal: mint a scoped share link (viewer or editor,
  // with a fixed TTL), display the URL + a QR code for phone hand-off, and
  // list/revoke existing active shares for the session.
  // Task 7.5: adds recipient email + duration (≤12h) for OTP-gated shares and
  // shows a hint when the owner has no verified email sender.
  import { onMount } from 'svelte';
  import QRCode from 'qrcode';
  import Modal from '../../lib/components/Modal.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { api } from '../../lib/api/client';
  import type { CreateShareReq, CreateShareResp, ShareInfo, EmailSenderResp } from '../../lib/api/types';
  import { toasts } from '../../lib/toast.svelte';
  import { router } from '../../lib/router.svelte';

  interface Props {
    sessionId: string;
    onclose: () => void;
  }
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
  let mintedToken = $state<string | null>(null);
  let qrCanvas: HTMLCanvasElement | null = $state(null);

  // ── existing-shares state ────────────────────────────────────────────────────
  let shares = $state<ShareInfo[]>([]);
  let sharesLoading = $state(false);

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
    try {
      const resp = await api.get<{ shares: ShareInfo[] }>(`/sessions/${encodeURIComponent(sessionId)}/shares`);
      shares = resp.shares;
    } catch {
      // Not fatal — the list is informational only; the mint flow still works.
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
    generating = true;
    mintedUrl = null;
    mintedToken = null;
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
      mintedToken = resp.token;
      // Optimistically prepend the new share to the list.
      shares = [resp.info, ...shares];
      toasts.success(
        'Share link created',
        email
          ? `A 6-digit code was emailed to ${email}. The recipient must enter it before attaching.`
          : 'Copy the URL or scan the QR code.',
      );
    } catch (e) {
      toasts.error('Could not create share link', e instanceof Error ? e.message : String(e));
    } finally {
      generating = false;
    }
  }

  // ── copy URL to clipboard ────────────────────────────────────────────────────
  async function copyUrl(): Promise<void> {
    if (!mintedUrl) return;
    try {
      await navigator.clipboard.writeText(mintedUrl);
      toasts.success('Copied!', 'Share link is in your clipboard.');
    } catch {
      toasts.error('Copy failed', 'Could not access clipboard.');
    }
  }

  // ── revoke a single share ────────────────────────────────────────────────────
  async function revoke(shareId: string): Promise<void> {
    revoking = { ...revoking, [shareId]: true };
    try {
      await api.del(`/auth/shares/${encodeURIComponent(shareId)}`);
      shares = shares.filter((s) => s.id !== shareId);
      // If the just-revoked share is the one we just minted, clear the URL panel.
      if (mintedUrl) {
        const minted = shares.find((s) => s.id === shareId);
        if (!minted) {
          mintedUrl = null;
          mintedToken = null;
        }
      }
      toasts.success('Share revoked');
    } catch (e) {
      toasts.error('Revoke failed', e instanceof Error ? e.message : String(e));
    } finally {
      const next = { ...revoking };
      delete next[shareId];
      revoking = next;
    }
  }

  // ── revoke all shares ────────────────────────────────────────────────────────
  async function revokeAll(): Promise<void> {
    if (revokingAll) return;
    revokingAll = true;
    try {
      await api.post('/auth/shares/revoke-all');
      shares = [];
      mintedUrl = null;
      mintedToken = null;
      toasts.success('All share links revoked');
    } catch (e) {
      toasts.error('Revoke all failed', e instanceof Error ? e.message : String(e));
    } finally {
      revokingAll = false;
    }
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
</script>

<Modal title="Share this session" width={500} {onclose}>
  <!-- ── Form ─────────────────────────────────────────────────────────── -->
  <div class="sm-body">
    <div class="sm-row">
      <label class="sm-label" for="sm-role">Permission</label>
      <select id="sm-role" class="sm-select" bind:value={role}>
        <option value="viewer">Viewer — read-only, can watch but not type</option>
        <option value="editor">Editor — can type commands in the terminal</option>
      </select>
    </div>

    <!-- ── OTP gate: recipient email ────────────────────────────────────── -->
    <div class="sm-row">
      <label class="sm-label" for="sm-recipient">
        Recipient email
        <span class="sm-optional">(optional — adds OTP email gate)</span>
      </label>
      {#if !emailSenderLoading && !hasSender}
        <div class="sm-sender-warn">
          <Icon name="warn" size={12} />
          No verified email sender.
          <!-- svelte-ignore a11y_invalid_attribute -->
          <a
            href="#"
            onclick={(e) => { e.preventDefault(); onclose(); router.go('settings/sharing'); }}
          >
            Set one up in Settings → Sharing
          </a>
          before creating OTP-gated links.
        </div>
      {/if}
      <input
        id="sm-recipient"
        class="sm-input"
        type="email"
        placeholder="guest@example.com — leave blank for a plain link"
        autocomplete="off"
        bind:value={recipientEmail}
        disabled={!hasSender && !emailSenderLoading}
        title={!hasSender && !emailSenderLoading ? 'Set up a verified email sender first (Settings → Sharing)' : undefined}
      />
    </div>

    <!-- Duration (only meaningful for OTP shares) ─────────────────────── -->
    {#if isOtpShare}
      <div class="sm-row">
        <label class="sm-label" for="sm-duration">Session window (max 12h)</label>
        <select id="sm-duration" class="sm-select" bind:value={durationSecs}>
          {#each DURATION_OPTIONS as opt (opt.secs)}
            <option value={opt.secs}>{opt.label}</option>
          {/each}
        </select>
        <span class="sm-note">
          How long the guest may stay attached after entering the code.
          Each extension re-starts this window.
        </span>
      </div>
    {:else}
      <div class="sm-row">
        <label class="sm-label" for="sm-ttl">Link expires after</label>
        <select id="sm-ttl" class="sm-select" bind:value={ttlSecs}>
          {#each TTL_OPTIONS as opt (opt.secs)}
            <option value={opt.secs}>{opt.label}</option>
          {/each}
        </select>
      </div>
    {/if}

    <div class="sm-row">
      <label class="sm-label" for="sm-label">Label (optional)</label>
      <input
        id="sm-label"
        class="sm-input"
        type="text"
        placeholder='e.g. "for Alice"'
        maxlength="80"
        bind:value={label}
      />
    </div>

    {#if isOtpShare && hasSender}
      <p class="sm-otp-note">
        <Icon name="mail" size={12} />
        The recipient will be emailed a 6-digit code they must enter before the
        terminal attaches. The code is locked to their address for the lifetime of the link.
      </p>
    {/if}

    <button class="btn primary sm-generate" disabled={generating} onclick={generate}>
      {generating ? 'Generating…' : 'Generate link'}
    </button>

    <!-- ── Minted link + QR ──────────────────────────────────────────── -->
    {#if mintedUrl}
      <div class="sm-result">
        <div class="sm-url-row">
          <span class="sm-url" title={mintedUrl}>{mintedUrl}</span>
          <button class="btn sm-copy" onclick={copyUrl} title="Copy to clipboard">
            <Icon name="fetch" size={13} />
            Copy
          </button>
        </div>

        <div class="sm-qr-wrap">
          <canvas bind:this={qrCanvas} class="sm-qr"></canvas>
          <p class="sm-qr-hint">Scan to open on your phone</p>
        </div>

        {#if role === 'viewer'}
          <p class="sm-role-note">
            <Icon name="eye" size={12} /> Read-only link — the guest can watch but not type.
          </p>
        {:else}
          <p class="sm-role-note editor">
            <Icon name="edit" size={12} /> Editor link — the guest can type commands.
          </p>
        {/if}
      </div>
    {/if}

    <!-- ── Active shares list ────────────────────────────────────────── -->
    <div class="sm-shares-head">
      <span class="sm-section-label">Active share links</span>
      <span class="sm-spacer"></span>
      {#if shares.length > 0}
        <button
          class="sm-link-btn danger"
          disabled={revokingAll}
          onclick={revokeAll}
        >
          {revokingAll ? 'Revoking…' : 'Revoke all'}
        </button>
      {/if}
    </div>

    <div class="sm-shares-list">
      {#if sharesLoading}
        <div class="sm-empty">Loading…</div>
      {:else if shares.length === 0}
        <div class="sm-empty">No active share links for this session.</div>
      {:else}
        {#each shares as share (share.id)}
          <div class="sm-share-row">
            <div class="sm-share-info">
              <span class="sm-share-prefix">{share.token_prefix}…</span>
              {#if share.label}
                <span class="sm-share-label">{share.label}</span>
              {/if}
              <span class="sm-share-role" class:editor={share.role === 'editor'}>
                {share.role}
              </span>
            </div>
            <div class="sm-share-meta">
              <span class="sm-share-expiry">{fmtExpiry(share.expires_at)}</span>
              <button
                class="btn sm-revoke-btn"
                disabled={revoking[share.id]}
                onclick={() => revoke(share.id)}
                title="Revoke this link"
              >
                {revoking[share.id] ? '…' : 'Revoke'}
              </button>
            </div>
          </div>
        {/each}
      {/if}
    </div>
  </div>
</Modal>

<style>
  .sm-body {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  /* ── Form rows ── */
  .sm-row {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .sm-label {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  .sm-optional {
    font-weight: 400;
    text-transform: none;
    letter-spacing: 0;
    font-style: italic;
  }
  .sm-sender-warn {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: 11.5px;
    color: #f59e0b;
    padding: 4px 0;
  }
  .sm-sender-warn a {
    color: var(--accent);
    text-decoration: underline;
    cursor: pointer;
  }
  .sm-note {
    font-size: 11px;
    color: var(--text-dim);
    line-height: 1.45;
  }
  .sm-otp-note {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    font-size: 11.5px;
    color: var(--text-dim);
    background: color-mix(in srgb, var(--accent) 6%, var(--surface-2));
    border: 1px solid color-mix(in srgb, var(--accent) 20%, transparent);
    border-radius: var(--radius-s);
    padding: 8px 10px;
    margin: 0;
    line-height: 1.5;
  }
  .sm-select,
  .sm-input {
    width: 100%;
    box-sizing: border-box;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    color: var(--text);
    font-size: 13px;
    padding: 7px 10px;
    appearance: auto;
  }
  .sm-select:focus,
  .sm-input:focus {
    outline: none;
    border-color: var(--accent);
  }
  .sm-generate {
    align-self: flex-start;
    margin-top: 2px;
  }

  /* ── Minted result ── */
  .sm-result {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: color-mix(in srgb, var(--accent) 6%, var(--surface-2));
  }
  .sm-url-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .sm-url {
    flex: 1;
    min-width: 0;
    font-size: 11.5px;
    color: var(--accent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: monospace;
  }
  .sm-copy {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: 12px;
    padding: 5px 10px;
  }
  .sm-qr-wrap {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
  }
  .sm-qr {
    border-radius: var(--radius-s);
    display: block;
  }
  .sm-qr-hint {
    font-size: 11px;
    color: var(--text-dim);
    margin: 0;
  }
  .sm-role-note {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: 11.5px;
    color: var(--text-dim);
    margin: 0;
  }
  .sm-role-note.editor {
    color: color-mix(in srgb, #f59e0b 80%, var(--text));
  }

  /* ── Shares section header ── */
  .sm-shares-head {
    display: flex;
    align-items: center;
    margin-top: 4px;
  }
  .sm-section-label {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  .sm-spacer {
    flex: 1;
  }
  .sm-link-btn {
    border: none;
    background: transparent;
    font-size: 12px;
    cursor: pointer;
    padding: 2px 4px;
    border-radius: var(--radius-s);
    color: var(--accent);
  }
  .sm-link-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .sm-link-btn.danger {
    color: #ef4444;
  }
  .sm-link-btn.danger:hover:not(:disabled) {
    background: color-mix(in srgb, #ef4444 12%, transparent);
  }

  /* ── Shares list ── */
  .sm-shares-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    padding: 4px;
    max-height: 200px;
    overflow-y: auto;
  }
  .sm-empty {
    padding: 14px;
    text-align: center;
    font-size: 12px;
    color: var(--text-dim);
  }
  .sm-share-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    border-radius: var(--radius-s);
    font-size: 12px;
  }
  .sm-share-row:hover {
    background: var(--surface-2);
  }
  .sm-share-info {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 1;
    min-width: 0;
    overflow: hidden;
  }
  .sm-share-prefix {
    font-family: monospace;
    font-size: 11px;
    color: var(--text-dim);
    white-space: nowrap;
    flex-shrink: 0;
  }
  .sm-share-label {
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sm-share-role {
    flex-shrink: 0;
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    padding: 1px 5px;
    border-radius: 99px;
    background: color-mix(in srgb, var(--text-dim) 15%, transparent);
    color: var(--text-dim);
  }
  .sm-share-role.editor {
    background: color-mix(in srgb, #f59e0b 18%, transparent);
    color: #f59e0b;
  }
  .sm-share-meta {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-shrink: 0;
  }
  .sm-share-expiry {
    font-size: 11px;
    color: var(--text-dim);
    white-space: nowrap;
  }
  .sm-revoke-btn {
    font-size: 11px;
    padding: 3px 8px;
    color: #ef4444;
    border-color: color-mix(in srgb, #ef4444 35%, transparent);
  }
  .sm-revoke-btn:hover:not(:disabled) {
    background: color-mix(in srgb, #ef4444 10%, transparent);
  }
  .sm-revoke-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
</style>
