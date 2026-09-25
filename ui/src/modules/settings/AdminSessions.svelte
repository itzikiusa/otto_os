<script lang="ts">
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import { sectionLabel } from './sections';
  import PageBody from '../../lib/components/PageBody.svelte';
  // Admin active-sessions overview: list every session daemon-wide; terminate
  // (kill the PTY, keep the row) or remove (delete the row + history), one at a
  // time or in bulk. "Remove all exited" prunes the background/ephemeral
  // sessions (insights, analysis, …) that otherwise accumulate without bound.
  import { api } from '../../lib/api/client';
  import type { AdminSessionRow, AdminSessionsResp } from '../../lib/api/types';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import StatusBadge from '../../lib/components/StatusBadge.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { sentenceCase, type Tone } from '../../lib/status';

  let sessions: AdminSessionRow[] = $state([]);
  let loading = $state(true);
  let loadError = $state('');
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
      loadError = '';
    } catch (e) {
      loadError = loadErrorText(e);
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
      `Terminate “${title || 'Untitled session'}”? Its process is killed and every viewer is disconnected. The session stays in the list (and its history is kept).`,
      { title: 'Terminate session', confirmLabel: 'Terminate', danger: true },
    );
    if (!ok) return;
    try {
      await act(id, 'terminate');
      toasts.success('Session terminated', title);
      await load();
    } catch (e) {
      toasts.error(`Couldn’t terminate “${title}”`, e instanceof Error ? e.message : String(e));
    }
  }

  async function remove(id: string, title: string): Promise<void> {
    const ok = await confirmer.ask(
      `Delete “${title || 'Untitled session'}”? The session and its history are permanently deleted.`,
      { title: 'Delete session', confirmLabel: 'Delete', danger: true },
    );
    if (!ok) return;
    try {
      await act(id, 'remove');
      toasts.success('Session deleted', title);
      await load();
    } catch (e) {
      toasts.error(`Couldn’t delete “${title}”`, e instanceof Error ? e.message : String(e));
    }
  }

  // Run `kind` over `ids` with bounded concurrency, after one confirm.
  async function runBulk(ids: string[], kind: 'terminate' | 'remove', label: string): Promise<void> {
    if (ids.length === 0) return;
    const ok = await confirmer.ask(
      `${label} ${ids.length} session${ids.length === 1 ? '' : 's'}?${
        kind === 'remove' ? ' They and their history are permanently deleted.' : ' Their processes are killed; the rows and history are kept.'
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
    const past = kind === 'remove' ? 'Deleted' : 'Terminated';
    if (failed === 0) toasts.success(`${past} ${done} session${done === 1 ? '' : 's'}`);
    else toasts.error(`Couldn’t ${label.toLowerCase()} ${failed} session${failed === 1 ? '' : 's'}`, `${done} succeeded, ${failed} failed. Refresh and try again.`);
    await load();
  }

  const bulkTerminate = (): Promise<void> => runBulk([...selected], 'terminate', 'Terminate');
  const bulkRemove = (): Promise<void> => runBulk([...selected], 'remove', 'Delete');
  const removeExited = (): Promise<void> =>
    runBulk(
      sessions.filter((s) => !s.live).map((s) => s.id),
      'remove',
      'Delete',
    );

  // One word + one tone per state (matches StatusDot's vocabulary): working
  // is the only green; idle is neutral; an exited PTY reads "Ended".
  function statusInfo(s: AdminSessionRow): { tone: Tone; label: string } {
    const st = s.status.toLowerCase();
    if (s.live && st === 'working') return { tone: 'success', label: 'Working' };
    if (s.live && st === 'running') return { tone: 'info', label: 'Running' };
    if (s.live && st === 'idle') return { tone: 'neutral', label: 'Idle' };
    if (st === 'reconnectable') return { tone: 'neutral', label: 'Suspended' };
    if (st === 'exited' || !s.live) return { tone: 'neutral', label: 'Ended' };
    return { tone: 'neutral', label: sentenceCase(st) };
  }
</script>

<div class="settings-section">
  <PageHeader title={sectionLabel('sessions')} subtitle="All sessions across all users">
    {#snippet actions()}
      {#if exitedCount > 0}
        <button class="btn small" data-overflow="-1" data-icon="trash" onclick={removeExited} disabled={bulkBusy || loading}>
          Delete all ended ({exitedCount})…
        </button>
      {/if}
      <button class="btn small" data-icon="refresh" onclick={load} disabled={loading || bulkBusy}>
        <Icon name="refresh" size={12} /> {loading && sessions.length ? 'Refreshing…' : 'Refresh'}
      </button>
    {/snippet}
  </PageHeader>
  <PageBody width="readable">
  <p class="section-intro"><strong>Terminate</strong> kills a live session's process and keeps its row and history; <strong>Delete</strong> removes the session and its history for good.</p>

  {#if selected.size > 0}
    <div class="bulk-bar" role="toolbar" aria-label="Selected sessions">
      <span class="bulk-count">{selected.size} selected</span>
      <button class="btn small ghost" onclick={() => (selected = new Set())} disabled={bulkBusy}>Clear selection</button>
      <button class="btn small" onclick={bulkTerminate} disabled={bulkBusy}>Terminate…</button>
      <button class="btn small danger" onclick={bulkRemove} disabled={bulkBusy}>Delete…</button>
    </div>
  {/if}

  <LoadState what="sessions" {loading} error={loadError} empty={sessions.length === 0} rows={4} onretry={() => void load()}>
    {#snippet emptyView()}
      <EmptyState variant="page" icon="terminal" title="No sessions" body="No user has an agent or connection session right now. New sessions appear here for every user." />
    {/snippet}
    <div class="card session-table">
      <div class="table-inner">
        <div class="session-head">
          <span class="col-sel">
            <input type="checkbox" checked={allSelected} onchange={toggleAll} aria-label="Select all sessions" />
          </span>
          <span class="col-owner">Owner</span>
          <span class="col-kind">Kind / provider</span>
          <span class="col-title">Title</span>
          <span class="col-status">Status</span>
          <span class="col-viewers">Viewers</span>
          <span class="col-action"><span class="sr-only">Actions</span></span>
        </div>
        {#each sessions as s (s.id)}
          {@const info = statusInfo(s)}
          <div class="session-row" class:row-sel={selected.has(s.id)}>
            <span class="col-sel">
              <input
                type="checkbox"
                checked={selected.has(s.id)}
                onchange={() => toggle(s.id)}
                aria-label={`Select ${s.title || 'untitled session'}`}
              />
            </span>
            <span class="col-owner ellip" title={s.owner_username}>
              <span class="owner-name">{s.owner_username}</span>
            </span>
            <span class="col-kind ellip" title={`${s.kind} · ${s.provider}`}>
              <span class="chip">{sentenceCase(s.kind)}</span>
              <span class="dim provider-name">{s.provider}</span>
            </span>
            <span class="col-title ellip" title={s.title}>{s.title || '—'}</span>
            <span class="col-status">
              <StatusBadge variant="text" tone={info.tone} label={info.label} title={`Stored status: ${s.status}`} />
            </span>
            <span class="col-viewers dim">{s.viewers > 0 ? s.viewers : '—'}</span>
            <span class="col-action">
              {#if s.live}
                <button
                  class="btn small ghost"
                  disabled={busy.has(s.id) || bulkBusy}
                  onclick={() => terminate(s.id, s.title)}
                >
                  {busy.has(s.id) ? 'Working…' : 'Terminate…'}
                </button>
              {/if}
              <button
                class="icon-btn danger-icon"
                disabled={busy.has(s.id) || bulkBusy}
                aria-label={`Delete ${s.title || 'untitled session'}`}
                title={`Delete ${s.title || 'untitled session'}`}
                onclick={() => remove(s.id, s.title)}
              >
                <Icon name="trash" size={14} />
              </button>
            </span>
          </div>
        {/each}
      </div>
    </div>
  </LoadState>
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
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
  .section-intro {
    margin: 0 0 14px;
    max-width: 78ch;
    font-size: var(--fs-s);
    line-height: 1.5;
    color: var(--text-dim);
  }
  .bulk-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0 0 10px;
    padding: 6px 8px 6px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
  }
  .bulk-count {
    font-size: var(--fs-s);
    color: var(--text-dim);
    margin-inline-end: auto;
  }
  /* Wide table scrolls inside its card, never the page. */
  .session-table {
    overflow-x: auto;
  }
  .table-inner {
    min-width: 720px;
  }
  .session-head,
  .session-row {
    display: grid;
    grid-template-columns: 28px 120px 150px minmax(0, 1fr) 100px 60px 128px;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
    min-height: 36px;
    font-size: var(--fs-s);
  }
  .session-head {
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-dim);
    border-bottom: 1px solid var(--border);
  }
  .session-row + .session-row {
    border-top: 1px solid var(--border);
  }
  .session-row:hover {
    background: var(--hover);
  }
  .session-row.row-sel {
    background: var(--accent-soft);
  }
  .col-sel {
    display: flex;
    align-items: center;
  }
  .owner-name {
    font-weight: 500;
  }
  .ellip {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .col-kind {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .provider-name {
    font-size: var(--fs-xs);
  }
  .dim {
    color: var(--text-dim);
  }
  .col-viewers {
    text-align: end;
    font-variant-numeric: tabular-nums;
  }
  .col-action {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    gap: 4px;
  }
  .danger-icon:hover:not(:disabled) {
    color: var(--danger);
  }
</style>
