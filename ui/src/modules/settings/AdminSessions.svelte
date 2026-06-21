<script lang="ts">
  // Admin active-sessions overview: list every session daemon-wide; terminate
  // (kill the PTY, keep the row) or remove (delete the row + history), one at a
  // time or in bulk. "Remove all exited" prunes the background/ephemeral
  // sessions (insights, analysis, …) that otherwise accumulate without bound.
  import { api } from '../../lib/api/client';
  import type { AdminSessionRow, AdminSessionsResp } from '../../lib/api/types';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';

  let sessions: AdminSessionRow[] = $state([]);
  let loading = $state(true);
  let busy: Set<string> = $state(new Set());
  let selected: Set<string> = $state(new Set());
  let bulkBusy = $state(false);

  const exitedCount = $derived(sessions.filter((s) => !s.live).length);
  const allSelected = $derived(sessions.length > 0 && selected.size === sessions.length);

  $effect(() => {
    void load();
  });

  async function load(): Promise<void> {
    loading = true;
    try {
      const resp = await api.get<AdminSessionsResp>('/admin/sessions');
      sessions = resp.sessions;
      // Drop selections for rows that no longer exist.
      const ids = new Set(sessions.map((s) => s.id));
      selected = new Set([...selected].filter((id) => ids.has(id)));
    } catch (e) {
      toasts.error('Could not load sessions', e instanceof Error ? e.message : String(e));
    } finally {
      loading = false;
    }
  }

  function toggle(id: string): void {
    const next = new Set(selected);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    selected = next;
  }
  function toggleAll(): void {
    selected = allSelected ? new Set() : new Set(sessions.map((s) => s.id));
  }

  async function act(id: string, kind: 'terminate' | 'remove'): Promise<void> {
    busy = new Set([...busy, id]);
    try {
      await api.post(`/admin/sessions/${id}/${kind}`, {});
    } finally {
      busy = new Set([...busy].filter((x) => x !== id));
    }
  }

  async function terminate(id: string, title: string): Promise<void> {
    const ok = await confirmer.ask(
      `Terminate session "${title}"? This kills the PTY and disconnects all viewers.`,
      { title: 'Terminate session', confirmLabel: 'Terminate', danger: true },
    );
    if (!ok) return;
    try {
      await act(id, 'terminate');
      toasts.success('Session terminated', title);
      await load();
    } catch (e) {
      toasts.error('Terminate failed', e instanceof Error ? e.message : String(e));
    }
  }

  async function remove(id: string, title: string): Promise<void> {
    const ok = await confirmer.ask(
      `Remove session "${title}"? This permanently deletes the session and its history.`,
      { title: 'Remove session', confirmLabel: 'Remove', danger: true },
    );
    if (!ok) return;
    try {
      await act(id, 'remove');
      toasts.success('Session removed', title);
      await load();
    } catch (e) {
      toasts.error('Remove failed', e instanceof Error ? e.message : String(e));
    }
  }

  // Run `kind` over `ids` with bounded concurrency, after one confirm.
  async function runBulk(ids: string[], kind: 'terminate' | 'remove', label: string): Promise<void> {
    if (ids.length === 0) return;
    const ok = await confirmer.ask(
      `${label} ${ids.length} session${ids.length === 1 ? '' : 's'}?${
        kind === 'remove' ? ' This permanently deletes them and their history.' : ''
      }`,
      { title: `${label} sessions`, confirmLabel: label, danger: true },
    );
    if (!ok) return;
    bulkBusy = true;
    let done = 0;
    let failed = 0;
    const queue = [...ids];
    const worker = async (): Promise<void> => {
      while (queue.length) {
        const id = queue.shift();
        if (id === undefined) break;
        try {
          await act(id, kind);
          done++;
        } catch {
          failed++;
        }
      }
    };
    await Promise.all(Array.from({ length: 4 }, worker));
    bulkBusy = false;
    selected = new Set();
    if (failed === 0) toasts.success(`${label}d ${done} session${done === 1 ? '' : 's'}`);
    else toasts.error(`${label} partly failed`, `${done} ok, ${failed} failed`);
    await load();
  }

  const bulkTerminate = (): Promise<void> => runBulk([...selected], 'terminate', 'Terminate');
  const bulkRemove = (): Promise<void> => runBulk([...selected], 'remove', 'Remove');
  const removeExited = (): Promise<void> =>
    runBulk(
      sessions.filter((s) => !s.live).map((s) => s.id),
      'remove',
      'Remove',
    );

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
      <div class="sub">
        All sessions across all users. <strong>Terminate</strong> keeps the row; <strong>Remove</strong>
        deletes it (and its history).
      </div>
    </div>
    <div class="header-actions">
      {#if exitedCount > 0}
        <button class="btn small" onclick={removeExited} disabled={bulkBusy || loading}>
          Remove all exited ({exitedCount})
        </button>
      {/if}
      <button class="btn" onclick={load} disabled={loading || bulkBusy}>Refresh</button>
    </div>
  </div>

  {#if selected.size > 0}
    <div class="bulk-bar">
      <span class="bulk-count">{selected.size} selected</span>
      <button class="btn small danger" onclick={bulkTerminate} disabled={bulkBusy}>Terminate</button>
      <button class="btn small danger" onclick={bulkRemove} disabled={bulkBusy}>Remove</button>
      <button class="btn small" onclick={() => (selected = new Set())} disabled={bulkBusy}>Clear</button>
    </div>
  {/if}

  {#if loading}
    <Skeleton rows={4} height={44} />
  {:else if sessions.length === 0}
    <div class="empty dim">No sessions found.</div>
  {:else}
    <div class="card session-table">
      <div class="session-head">
        <span class="col-sel">
          <input type="checkbox" checked={allSelected} onchange={toggleAll} aria-label="Select all" />
        </span>
        <span class="col-owner">Owner</span>
        <span class="col-kind">Kind / Provider</span>
        <span class="col-title">Title</span>
        <span class="col-status">Status</span>
        <span class="col-viewers">Viewers</span>
        <span class="col-action"></span>
      </div>
      {#each sessions as s (s.id)}
        <div class="session-row" class:row-sel={selected.has(s.id)}>
          <span class="col-sel">
            <input
              type="checkbox"
              checked={selected.has(s.id)}
              onchange={() => toggle(s.id)}
              aria-label="Select session"
            />
          </span>
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
            {#if s.live}
              <button
                class="btn small danger"
                disabled={busy.has(s.id) || bulkBusy}
                onclick={() => terminate(s.id, s.title)}
              >
                {busy.has(s.id) ? '…' : 'Terminate'}
              </button>
            {/if}
            <button
              class="btn small"
              disabled={busy.has(s.id) || bulkBusy}
              onclick={() => remove(s.id, s.title)}
            >
              {busy.has(s.id) ? '…' : 'Remove'}
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

  .header-actions {
    display: flex;
    gap: 8px;
    align-items: center;
  }

  .bulk-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0 0 10px;
    padding: 8px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 4px);
    background: var(--surface-2);
  }
  .bulk-count {
    font-size: 12.5px;
    color: var(--text-dim);
    margin-inline-end: auto;
  }

  .session-table {
    overflow: hidden;
  }

  .session-head,
  .session-row {
    display: grid;
    grid-template-columns: 28px 120px 120px 1fr 110px 56px 152px;
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
  .session-row.row-sel {
    background: color-mix(in srgb, var(--accent) 8%, transparent);
  }

  .col-sel {
    display: flex;
    align-items: center;
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
    margin-inline-end: 4px;
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
    gap: 6px;
  }
</style>
