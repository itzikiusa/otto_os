<script lang="ts">
  // Admin active-sessions overview: list all sessions daemon-wide, terminate any.
  import { api } from '../../lib/api/client';
  import type { AdminSessionRow, AdminSessionsResp } from '../../lib/api/types';
  import { toasts } from '../../lib/toast.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';

  let sessions: AdminSessionRow[] = $state([]);
  let loading = $state(true);
  let terminating: Set<string> = $state(new Set());

  $effect(() => {
    void load();
  });

  async function load(): Promise<void> {
    loading = true;
    try {
      const resp = await api.get<AdminSessionsResp>('/admin/sessions');
      sessions = resp.sessions;
    } catch (e) {
      toasts.error('Could not load sessions', e instanceof Error ? e.message : String(e));
    } finally {
      loading = false;
    }
  }

  async function terminate(id: string, title: string): Promise<void> {
    const confirmed = window.confirm(`Terminate session "${title}"? This will kill the PTY and disconnect all viewers.`);
    if (!confirmed) return;

    terminating = new Set([...terminating, id]);
    try {
      await api.post(`/admin/sessions/${id}/terminate`, {});
      toasts.success('Session terminated', title);
      // Refresh the list after a brief moment so the terminated session status updates.
      await load();
    } catch (e) {
      toasts.error('Terminate failed', e instanceof Error ? e.message : String(e));
    } finally {
      terminating = new Set([...terminating].filter((x) => x !== id));
    }
  }

  function statusClass(s: AdminSessionRow): string {
    if (s.live) return 'live';
    if (s.status === 'exited') return 'exited';
    return '';
  }
</script>

<div class="page">
  <div class="page-header">
    <div>
      <h1>Sessions</h1>
      <div class="sub">All active sessions across all users. Admins and root may terminate any session.</div>
    </div>
    <button class="btn" onclick={load} disabled={loading}>Refresh</button>
  </div>

  {#if loading}
    <Skeleton rows={4} height={44} />
  {:else if sessions.length === 0}
    <div class="empty dim">No sessions found.</div>
  {:else}
    <div class="card session-table">
      <div class="session-head">
        <span class="col-owner">Owner</span>
        <span class="col-kind">Kind / Provider</span>
        <span class="col-title">Title</span>
        <span class="col-status">Status</span>
        <span class="col-viewers">Viewers</span>
        <span class="col-action"></span>
      </div>
      {#each sessions as s (s.id)}
        <div class="session-row">
          <span class="col-owner">
            <span class="owner-name">{s.owner_username}</span>
          </span>
          <span class="col-kind">
            <span class="chip-kind">{s.kind}</span>
            <span class="dim provider-name">{s.provider}</span>
          </span>
          <span class="col-title" title={s.title}>{s.title || '—'}</span>
          <span class="col-status">
            <span class="status-badge {statusClass(s)}">
              {#if s.live}
                <span class="live-dot" aria-label="live"></span>
              {/if}
              {s.status}
            </span>
          </span>
          <span class="col-viewers dim">{s.viewers > 0 ? s.viewers : '—'}</span>
          <span class="col-action">
            <button
              class="btn small danger"
              disabled={terminating.has(s.id)}
              onclick={() => terminate(s.id, s.title)}
            >
              {terminating.has(s.id) ? 'Terminating…' : 'Terminate'}
            </button>
          </span>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .empty {
    padding: 24px 0;
    text-align: center;
    font-size: 13px;
  }

  .session-table {
    overflow: hidden;
  }

  .session-head,
  .session-row {
    display: grid;
    grid-template-columns: 130px 130px 1fr 130px 64px 110px;
    align-items: center;
    gap: 8px;
    padding: 8px 14px;
    font-size: 12.5px;
  }

  .session-head {
    font-size: 10.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-dim);
    border-bottom: 1px solid var(--border);
  }

  .session-row + .session-row {
    border-top: 1px solid var(--border);
  }

  .owner-name {
    font-weight: 500;
  }

  .col-title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .chip-kind {
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    padding: 1px 5px;
    border-radius: 3px;
    background: var(--surface-2);
    color: var(--text-dim);
    margin-right: 4px;
  }

  .provider-name {
    font-size: 11.5px;
  }

  .status-badge {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 11.5px;
  }

  .status-badge.live {
    color: var(--green, #3fb950);
  }

  .status-badge.exited {
    color: var(--text-dim);
  }

  .live-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--green, #3fb950);
    flex-shrink: 0;
    animation: pulse 2s ease-in-out infinite;
  }

  @keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.4; }
  }

  .col-viewers {
    text-align: center;
  }

  .col-action {
    display: flex;
    justify-content: flex-end;
  }
</style>
