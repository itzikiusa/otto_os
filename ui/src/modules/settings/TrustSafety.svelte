<script lang="ts">
  // Trust & Safety Center (root only): a derived security-posture summary plus
  // the filterable, paged security audit log. Reads `GET /security-posture` and
  // `GET /audit-log`; writes nothing. user_id -> username is resolved from the
  // users list so entries read sensibly.
  import { api } from '../../lib/api/client';
  import type { AuditEntry, AuditLogResp, SecurityPostureResp, User } from '../../lib/api/types';
  import Icon from '../../lib/components/Icon.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import { toasts } from '../../lib/toast.svelte';

  const PAGE_SIZE = 100;

  // The actions we audit today; drives the filter dropdown. Free-form on the
  // wire, so an unknown action still renders — this is just the curated list.
  const KNOWN_ACTIONS: { value: string; label: string }[] = [
    { value: '', label: 'All actions' },
    { value: 'login.success', label: 'Login success' },
    { value: 'login.failure', label: 'Login failure' },
    { value: 'login.lockout', label: 'Login lockout' },
    { value: 'token.mint', label: 'API token minted' },
    { value: 'token.revoke', label: 'API token revoked' },
    { value: 'settings.change', label: 'Settings changed' },
    { value: 'network_listener.toggle', label: 'Network listener toggled' },
    { value: 'db.write_confirmed', label: 'DB write confirmed' },
  ];

  let posture: SecurityPostureResp | null = $state(null);
  let postureLoading = $state(true);

  let entries: AuditEntry[] = $state([]);
  let total = $state(0);
  let logLoading = $state(true);
  let logError = $state('');

  // Filters.
  let action = $state('');
  let fromDate = $state(''); // yyyy-mm-dd
  let toDate = $state(''); // yyyy-mm-dd
  let offset = $state(0);

  // user_id -> username, populated from /users (best-effort; root sees all).
  let usernames: Record<string, string> = $state({});

  $effect(() => {
    void loadPosture();
    void loadUsers();
  });

  // Re-fetch the log whenever a filter or the page changes.
  $effect(() => {
    // touch the reactive deps so the effect re-runs on change
    void action;
    void fromDate;
    void toDate;
    void offset;
    void loadLog();
  });

  async function loadPosture(): Promise<void> {
    postureLoading = true;
    try {
      posture = await api.get<SecurityPostureResp>('/security-posture');
    } catch (e) {
      toasts.error('Could not load security posture', e instanceof Error ? e.message : String(e));
    } finally {
      postureLoading = false;
    }
  }

  async function loadUsers(): Promise<void> {
    try {
      const users = await api.get<User[]>('/users');
      const map: Record<string, string> = {};
      for (const u of users) map[u.id] = u.username;
      usernames = map;
    } catch {
      // Non-fatal: entries still render with the raw id.
    }
  }

  function startOfDayIso(d: string): string | undefined {
    if (!d) return undefined;
    return new Date(`${d}T00:00:00`).toISOString();
  }
  function endOfDayIso(d: string): string | undefined {
    if (!d) return undefined;
    return new Date(`${d}T23:59:59.999`).toISOString();
  }

  async function loadLog(): Promise<void> {
    logLoading = true;
    logError = '';
    try {
      const params = new URLSearchParams();
      params.set('limit', String(PAGE_SIZE));
      params.set('offset', String(offset));
      if (action) params.set('action', action);
      const from = startOfDayIso(fromDate);
      const to = endOfDayIso(toDate);
      if (from) params.set('from', from);
      if (to) params.set('to', to);
      const resp = await api.get<AuditLogResp>(`/audit-log?${params.toString()}`);
      entries = resp.entries;
      total = resp.total;
    } catch (e) {
      logError = e instanceof Error ? e.message : String(e);
    } finally {
      logLoading = false;
    }
  }

  function resetFilters(): void {
    action = '';
    fromDate = '';
    toDate = '';
    offset = 0;
  }

  function actorLabel(e: AuditEntry): string {
    if (!e.user_id) return 'anonymous';
    return usernames[e.user_id] ? `@${usernames[e.user_id]}` : e.user_id;
  }

  function actionLabel(a: string): string {
    return KNOWN_ACTIONS.find((k) => k.value === a)?.label ?? a;
  }

  function fmtTime(ts: string): string {
    const d = new Date(ts);
    return Number.isNaN(d.getTime()) ? ts : d.toLocaleString();
  }

  function detailText(e: AuditEntry): string {
    if (e.detail == null) return '';
    try {
      return typeof e.detail === 'string' ? e.detail : JSON.stringify(e.detail);
    } catch {
      return '';
    }
  }

  const pageStart = $derived(total === 0 ? 0 : offset + 1);
  const pageEnd = $derived(Math.min(offset + entries.length, total));
  const canPrev = $derived(offset > 0);
  const canNext = $derived(offset + PAGE_SIZE < total);
</script>

<div class="page trust-page">
  <div class="page-header">
    <div>
      <h1>Trust &amp; Safety</h1>
      <div class="sub">Security posture and the append-only audit log.</div>
    </div>
    <div class="header-actions">
      <button
        class="btn"
        disabled={logLoading || postureLoading}
        onclick={() => {
          void loadPosture();
          void loadLog();
        }}
      >
        <Icon name="refresh" size={13} />
        Refresh
      </button>
    </div>
  </div>

  <!-- Security posture summary -->
  <section class="posture">
    {#if postureLoading}
      <Skeleton rows={1} height={64} />
    {:else if posture}
      <div class="cards">
        <div class="card" class:warn={posture.network_listener}>
          <div class="card-label">Network listener</div>
          <div class="card-value">
            {#if posture.network_listener}
              <Icon name="globe" size={14} /> On
              {#if posture.network_listener_port}
                <span class="card-note">:{posture.network_listener_port}</span>
              {/if}
            {:else}
              <Icon name="check" size={14} /> Off
            {/if}
          </div>
        </div>
        <div class="card" class:ok={posture.loopback_only}>
          <div class="card-label">Binding</div>
          <div class="card-value">
            {posture.loopback_only ? 'Loopback only (127.0.0.1)' : 'Network (0.0.0.0)'}
          </div>
        </div>
        <div class="card">
          <div class="card-label">Active API tokens</div>
          <div class="card-value">{posture.active_api_tokens}</div>
        </div>
      </div>
    {/if}
  </section>

  <!-- Audit log -->
  <div class="toolbar">
    <label class="field">
      <span>Action</span>
      <select class="input" bind:value={action} onchange={() => (offset = 0)}>
        {#each KNOWN_ACTIONS as a (a.value)}
          <option value={a.value}>{a.label}</option>
        {/each}
      </select>
    </label>
    <label class="field">
      <span>From</span>
      <input class="input" type="date" bind:value={fromDate} onchange={() => (offset = 0)} />
    </label>
    <label class="field">
      <span>To</span>
      <input class="input" type="date" bind:value={toDate} onchange={() => (offset = 0)} />
    </label>
    <button class="btn ghost" onclick={resetFilters} disabled={!action && !fromDate && !toDate}>
      Clear
    </button>
  </div>

  <div class="log-meta">
    <span>
      {#if total > 0}
        {pageStart}–{pageEnd} of {total}
      {:else}
        No entries
      {/if}
    </span>
    <div class="pager">
      <button class="btn ghost" disabled={!canPrev} onclick={() => (offset = Math.max(0, offset - PAGE_SIZE))}>
        Prev
      </button>
      <button class="btn ghost" disabled={!canNext} onclick={() => (offset = offset + PAGE_SIZE)}>
        Next
      </button>
    </div>
  </div>

  <div class="log-body">
    {#if logLoading}
      <Skeleton rows={8} height={34} />
    {:else if logError}
      <div class="empty error">{logError}</div>
    {:else if entries.length === 0}
      <div class="empty">No audit entries match these filters.</div>
    {:else}
      <table class="audit-table">
        <thead>
          <tr>
            <th class="col-time">Time</th>
            <th class="col-action">Action</th>
            <th class="col-actor">Actor</th>
            <th class="col-target">Target</th>
            <th class="col-ip">IP</th>
            <th class="col-detail">Detail</th>
          </tr>
        </thead>
        <tbody>
          {#each entries as e (e.id)}
            <tr>
              <td class="col-time mono">{fmtTime(e.ts)}</td>
              <td class="col-action">
                <span class="badge" class:danger={e.action === 'login.failure' || e.action === 'login.lockout'}>
                  {actionLabel(e.action)}
                </span>
              </td>
              <td class="col-actor mono">{actorLabel(e)}</td>
              <td class="col-target mono" title={e.target ?? ''}>{e.target ?? '—'}</td>
              <td class="col-ip mono">{e.ip ?? '—'}</td>
              <td class="col-detail mono" title={detailText(e)}>{detailText(e) || '—'}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </div>
</div>

<style>
  .trust-page {
    height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .header-actions {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .posture {
    padding: 0 24px 14px;
  }
  .cards {
    display: flex;
    flex-wrap: wrap;
    gap: 12px;
  }
  .card {
    flex: 1;
    min-width: 180px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m, 8px);
    padding: 12px 14px;
    background: var(--surface-2);
  }
  .card.warn {
    border-color: color-mix(in srgb, var(--warn, #d08400) 50%, var(--border));
  }
  .card.ok {
    border-color: color-mix(in srgb, var(--ok, #2f9e44) 40%, var(--border));
  }
  .card-label {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
    margin-bottom: 6px;
  }
  .card-value {
    font-size: 15px;
    font-weight: 600;
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .card-note {
    font-weight: 400;
    color: var(--text-dim);
  }
  .toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: end;
    gap: 10px;
    padding: 0 24px 12px;
    border-top: 1px solid var(--border);
    border-bottom: 1px solid var(--border);
    padding-top: 12px;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .field span {
    font-size: 11px;
    color: var(--text-dim);
  }
  .btn.ghost {
    background: transparent;
  }
  .log-meta {
    min-height: 32px;
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 12px;
    padding: 0 24px;
    color: var(--text-dim);
    font-size: 11.5px;
    border-bottom: 1px solid var(--border);
  }
  .pager {
    display: flex;
    gap: 6px;
  }
  .log-body {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 4px 24px 32px;
  }
  .empty {
    padding: 24px;
    color: var(--text-dim);
    font-size: 12.5px;
  }
  .empty.error {
    color: var(--err, #e03131);
  }
  .audit-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12px;
  }
  .audit-table th {
    text-align: left;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
    font-weight: 600;
    padding: 8px 8px;
    border-bottom: 1px solid var(--border);
    position: sticky;
    top: 0;
    background: var(--surface);
  }
  .audit-table td {
    padding: 7px 8px;
    border-bottom: 1px solid var(--border);
    vertical-align: top;
  }
  .col-target,
  .col-detail {
    max-width: 280px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .mono {
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, 'Liberation Mono', monospace;
    font-size: 11.5px;
  }
  .badge {
    display: inline-block;
    padding: 1px 7px;
    border-radius: 10px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    font-size: 11px;
  }
  .badge.danger {
    border-color: color-mix(in srgb, var(--err, #e03131) 45%, var(--border));
    color: var(--err, #e03131);
  }
</style>
