<script lang="ts">
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import { sectionLabel } from './sections';
  import PageBody from '../../lib/components/PageBody.svelte';
  // Trust & Safety Center (root only): a derived security-posture summary plus
  // the filterable, paged security audit log. Reads `GET /security-posture` and
  // `GET /audit-log`; writes nothing. user_id -> username is resolved from the
  // users list so entries read sensibly.
  import { api } from '../../lib/api/client';
  import type { AuditEntry, AuditLogResp, SecurityPostureResp, User } from '../../lib/api/types';
  import Icon from '../../lib/components/Icon.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { copyAsJson } from '../../lib/components/exporters';
  import LoadState from '../../lib/components/LoadState.svelte';
  import StatusBadge from '../../lib/components/StatusBadge.svelte';
  import { loadErrorText } from '../../lib/loadError';

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
  let postureError = $state('');

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
      postureError = '';
    } catch (e) {
      // Inline, with Retry — a toast vanished and left an empty strip.
      postureError = loadErrorText(e);
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
      logError = loadErrorText(e);
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

  // Quick-preset helpers: set from/to to the last N days from now.
  function toYmd(d: Date): string {
    // yyyy-mm-dd in local time — matches the date input value format.
    const y = d.getFullYear();
    const m = String(d.getMonth() + 1).padStart(2, '0');
    const day = String(d.getDate()).padStart(2, '0');
    return `${y}-${m}-${day}`;
  }

  function applyPreset(days: number): void {
    const now = new Date();
    const from = new Date(now);
    from.setDate(from.getDate() - days);
    fromDate = toYmd(from);
    toDate = toYmd(now);
    offset = 0;
  }

  // Per-entry copy: tracks which entry is being copied (for brief feedback).
  let copyingId: string | null = $state(null);

  async function copyEntry(e: AuditEntry): Promise<void> {
    try {
      await copyAsJson(e);
      copyingId = e.id;
      // Brief check mark so the user sees it landed.
      setTimeout(() => {
        if (copyingId === e.id) copyingId = null;
      }, 1200);
    } catch {
      toasts.error('Couldn’t copy the entry', 'The clipboard write was blocked.');
    }
  }

  const filtered = $derived(!!action || !!fromDate || !!toDate);

  function actorLabel(e: AuditEntry): string {
    if (!e.user_id) return 'Anonymous';
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

<div class="settings-section trust-section">
  <PageHeader title={sectionLabel('trust-safety')} subtitle="Security posture and the append-only audit log">
    {#snippet actions()}
      <button
        class="btn small"
        data-icon="refresh"
        disabled={logLoading || postureLoading}
        onclick={() => {
          void loadPosture();
          void loadLog();
        }}
      >
        <Icon name="refresh" size={12} />
        {logLoading || postureLoading ? 'Refreshing…' : 'Refresh'}
      </button>
    {/snippet}
  </PageHeader>
  <PageBody padded={false} fill>

  <!-- Security posture summary -->
  <section class="posture" aria-label="Security posture">
    {#if postureLoading && !posture}
      <Skeleton rows={1} height={64} />
    {:else if postureError && !posture}
      <LoadState what="the security posture" error={postureError} empty variant="compact" onretry={() => void loadPosture()} />
    {:else if posture}
      <div class="cards">
        <div class="card">
          <div class="card-label">Network listener</div>
          <div class="card-value">
            {#if posture.network_listener}
              <StatusBadge tone="warning" label={posture.network_listener_port ? `On · port ${posture.network_listener_port}` : 'On'} />
            {:else}
              <StatusBadge tone="success" label="Off" />
            {/if}
          </div>
          <div class="card-note">{posture.network_listener ? 'Reachable from your network' : 'Only this Mac can connect'}</div>
        </div>
        <div class="card">
          <div class="card-label">Binding</div>
          <div class="card-value mono-val">{posture.loopback_only ? '127.0.0.1' : '0.0.0.0'}</div>
          <div class="card-note">{posture.loopback_only ? 'Loopback only' : 'All network interfaces'}</div>
        </div>
        <div class="card">
          <div class="card-label">Active API tokens</div>
          <div class="card-value num">{posture.active_api_tokens}</div>
          <div class="card-note">Personal access tokens that can call the API</div>
        </div>
      </div>
    {/if}
  </section>

  <!-- Audit log -->
  <div class="toolbar" role="group" aria-label="Filter the audit log">
    <label class="tfield">
      <span>Action</span>
      <select class="input" bind:value={action} onchange={() => (offset = 0)}>
        {#each KNOWN_ACTIONS as a (a.value)}
          <option value={a.value}>{a.label}</option>
        {/each}
      </select>
    </label>
    <label class="tfield">
      <span>From</span>
      <input class="input" type="date" bind:value={fromDate} onchange={() => (offset = 0)} />
    </label>
    <label class="tfield">
      <span>To</span>
      <input class="input" type="date" bind:value={toDate} onchange={() => (offset = 0)} />
    </label>
    <!-- Quick time-range presets -->
    <div class="segmented presets" role="group" aria-label="Quick range">
      <button onclick={() => applyPreset(1)}>Last 24h</button>
      <button onclick={() => applyPreset(7)}>Last 7 days</button>
      <button onclick={() => applyPreset(30)}>Last 30 days</button>
    </div>
    {#if filtered}
      <button class="btn small ghost" onclick={resetFilters}>Clear filters</button>
    {/if}
  </div>

  <!-- The count bar only while there is something to count; an empty log says
     so once, in the body below. -->
  {#if total > 0}
  <div class="log-meta">
    <span aria-live="polite">
      {pageStart}–{pageEnd} of {total} entr{total === 1 ? 'y' : 'ies'}
    </span>
    {#if total > PAGE_SIZE}
      <div class="pager">
        <button class="btn small ghost" disabled={!canPrev} onclick={() => (offset = Math.max(0, offset - PAGE_SIZE))}>
          <Icon name="chevronLeft" size={12} /> Previous
        </button>
        <button class="btn small ghost" disabled={!canNext} onclick={() => (offset = offset + PAGE_SIZE)}>
          Next <Icon name="chevronRight" size={12} />
        </button>
      </div>
    {/if}
  </div>
  {/if}

  <div class="log-body">
    {#if logLoading && entries.length === 0}
      <Skeleton rows={8} height={34} />
    {:else if logError}
      <LoadState what="the audit log" error={logError} empty onretry={() => void loadLog()} />
    {:else if entries.length === 0}
      {#if filtered}
        <div class="empty">
          No audit entries match these filters.
          <button class="btn small ghost" onclick={resetFilters}>Clear filters</button>
        </div>
      {:else}
        <div class="empty">No audit entries yet. Sign-ins, token changes and security settings changes are recorded here.</div>
      {/if}
    {:else}
      <table class="audit-table">
        <thead>
          <tr>
            <th class="col-time" scope="col">Time</th>
            <th class="col-action" scope="col">Action</th>
            <th class="col-actor" scope="col">Actor</th>
            <th class="col-target" scope="col">Target</th>
            <th class="col-ip" scope="col">IP</th>
            <th class="col-detail" scope="col">Detail</th>
            <th class="col-copy" scope="col"><span class="sr-only">Copy</span></th>
          </tr>
        </thead>
        <tbody>
          {#each entries as e (e.id)}
            <tr>
              <td class="col-time mono">{fmtTime(e.ts)}</td>
              <td class="col-action">
                <span class="chip" class:bad={e.action === 'login.failure' || e.action === 'login.lockout'}>
                  {actionLabel(e.action)}
                </span>
              </td>
              <td class="col-actor mono" title={e.user_id ?? undefined}>{actorLabel(e)}</td>
              <td class="col-target mono" title={e.target ?? ''}>{e.target ?? '—'}</td>
              <td class="col-ip mono">{e.ip ?? '—'}</td>
              <td class="col-detail mono" title={detailText(e)}>{detailText(e) || '—'}</td>
              <td class="col-copy">
                <button
                  class="icon-btn copy-btn"
                  title={copyingId === e.id ? 'Copied' : 'Copy entry as JSON'}
                  aria-label={copyingId === e.id ? 'Copied' : 'Copy entry as JSON'}
                  onclick={() => void copyEntry(e)}
                >
                  <Icon name={copyingId === e.id ? 'check' : 'copy'} size={14} />
                </button>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </div>
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
  .posture {
    padding: 16px 20px 14px;
  }
  .cards {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
    gap: 12px;
  }
  .card {
    min-width: 0;
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .card-label {
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-dim);
  }
  .card-value {
    font-size: var(--fs-l);
    font-weight: 600;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-height: 24px;
  }
  .card-value.mono-val {
    font-family: var(--font-mono);
    font-size: var(--fs-m);
  }
  .card-value.num {
    font-size: var(--fs-xl);
    font-variant-numeric: tabular-nums;
  }
  .card-note {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: 8px 12px;
    padding: 12px 20px;
    border-top: 1px solid var(--border);
    border-bottom: 1px solid var(--border);
  }
  .tfield {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .tfield span {
    font-size: var(--fs-xs);
    font-weight: 500;
    color: var(--text-dim);
  }
  .presets {
    align-self: flex-end;
    margin-bottom: 1px;
  }
  .log-meta {
    min-height: 32px;
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 12px;
    padding: 0 20px;
    color: var(--text-dim);
    font-size: var(--fs-s);
    border-bottom: 1px solid var(--border);
  }
  .pager {
    display: flex;
    gap: 4px;
  }
  .log-body {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 4px 20px 32px;
  }
  .empty {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 24px 4px;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .audit-table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--fs-s);
  }
  .audit-table th {
    text-align: start;
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
    font-weight: 600;
    padding: 8px;
    border-bottom: 1px solid var(--border);
    position: sticky;
    top: 0;
    z-index: 1;
    background: var(--bg);
  }
  .audit-table td {
    padding: 6px 8px;
    border-bottom: 1px solid var(--border);
    vertical-align: middle;
  }
  .audit-table tbody tr:hover {
    background: var(--hover);
  }
  .col-target,
  .col-detail {
    max-width: 280px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .col-time {
    white-space: nowrap;
  }
  .mono {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
  }
  /* Per-row copy-JSON button: quiet until the row is hovered or focused. */
  .col-copy {
    width: 32px;
    padding: 0 4px !important;
  }
  .copy-btn {
    opacity: 0;
  }
  tr:hover .copy-btn,
  .copy-btn:focus-visible {
    opacity: 1;
  }
  @media (hover: none) {
    .copy-btn {
      opacity: 1;
    }
  }
</style>
