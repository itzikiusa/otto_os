<script lang="ts">
  // Full-screen guest view for a scoped share link (#/s/<sessionId>).
  // No rail, no navigator, no right panel — just the session header + terminal.
  //
  // The share token was captured from the URL fragment by the router (Task 3.1)
  // and stored in-memory; we read it here via getShareToken(sessionId).
  // All API calls use the scoped token directly (not the owner login token).
  import Terminal from '../../lib/components/Terminal.svelte';
  import { getSharedSession, openShareTerminalWs } from '../../lib/api/share';
  import { getShareToken } from '../../lib/router.svelte';
  import type { Session, SessionStatus } from '../../lib/api/types';
  import { ui } from '../../lib/stores/ui.svelte';

  interface Props {
    sessionId: string;
  }
  let { sessionId }: Props = $props();

  // The share token captured by the router from the URL fragment.
  // If it's missing the link is invalid / already stripped without a prior capture.
  const token = $derived(getShareToken(sessionId));

  // Reactive session metadata load.
  let session = $state<Session | null>(null);
  let loadError = $state<string | null>(null);
  let liveStatus = $state<SessionStatus | null>(null);

  $effect(() => {
    const t = token;
    if (!t) return; // no token — render the error state below

    let cancelled = false;
    session = null;
    loadError = null;

    getSharedSession(sessionId, t)
      .then((s) => {
        if (!cancelled) session = s;
      })
      .catch((e: unknown) => {
        if (!cancelled) {
          loadError = e instanceof Error ? e.message : String(e);
        }
      });

    return () => { cancelled = true; };
  });

  // Effective status: prefer the live WS status, fall back to the REST field.
  const status = $derived<SessionStatus>(liveStatus ?? session?.status ?? 'idle');

  // Is this a viewer share (read-only)?  Determined from the session's share
  // role.  We don't have direct role info from the REST call — the Terminal
  // component drives its own WS and will receive a 403 on input if the scope
  // role is viewer.  To show the correct badge we default to read-only when
  // we can't confirm the role (safe: the badge is cosmetic; enforcement is in
  // the daemon).  For now we expose a prop so future callers can override it.
  //
  // In practice the share URL carries the raw token and the daemon's scope guard
  // enforces the cap. The UI always shows the read-only badge by default (safe
  // side); if the token role is editor, typing still works — only the badge
  // differs. A future Task 3.3 can pass the explicit role from the mint response.
  let isViewer = $state(true);

  // Label helpers for the status badge.
  const statusLabel: Record<SessionStatus, string> = {
    running: 'running',
    working: 'working',
    idle: 'idle',
    exited: 'exited',
    reconnectable: 'reconnectable',
  };

  function onTermStatus(s: SessionStatus): void {
    liveStatus = s;
  }
</script>

{#if !token}
  <!-- No token was captured — link is invalid, expired, or the visitor arrived
       here directly without the token segment. -->
  <div class="share-error" style={`zoom:${ui.zoom}`}>
    <div class="error-card">
      <div class="error-icon">⚠</div>
      <h2>Link invalid or expired</h2>
      <p>
        This share link is missing a token or has already expired.
        Ask the owner to send you a new link.
      </p>
    </div>
  </div>
{:else if loadError}
  <!-- Token present but the GET /sessions/{id} call failed — could be expired,
       revoked, or a network error. -->
  <div class="share-error" style={`zoom:${ui.zoom}`}>
    <div class="error-card">
      <div class="error-icon">⚠</div>
      <h2>Could not load session</h2>
      <p>{loadError}</p>
      <p class="hint">The share link may have been revoked or may have expired.</p>
    </div>
  </div>
{:else}
  <!-- Main guest view: slim header + full-screen terminal. -->
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
        <!-- Skeleton while the REST call is in-flight. -->
        <div class="connecting-overlay">
          <span>Connecting…</span>
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  /* ---- error state ---- */
  .share-error {
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--bg);
  }
  .error-card {
    max-width: 380px;
    padding: 32px 28px;
    border-radius: var(--radius-l);
    background: var(--surface);
    border: 1px solid var(--border);
    text-align: center;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .error-icon {
    font-size: 32px;
    color: var(--status-exited);
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
</style>
