<script lang="ts">
  // Processes panel (connection menu → Processes…): what is running on a MySQL /
  // PostgreSQL / ClickHouse / MongoDB server, with Refresh, and Kill behind a
  // confirm that names the query (PostgreSQL: Cancel the query, or Terminate
  // the connection). Listing is read-only (process-list.ts); Kill is a write, so it is not
  // offered on a Read-only connection (the server refuses it there too) and a
  // production connection takes the usual typed confirm after this one.
  import { untrack } from 'svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { api } from '../../lib/api/client';
  import type { Connection, QueryResult } from '../../lib/api/types';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { toastError } from '../../lib/toastError';
  import { loadErrorText } from '../../lib/loadError';
  import { database } from '../../lib/stores/database.svelte';
  import { isWriteBlocked } from '../../lib/stores/database/model';
  import { PROCESS_SQL, killChoices, killNotDone, killSql, parseProcesses, processEngine, processNotices, queryPreview, type KillChoice, type ProcessRow } from './process-list';

  let { conn, onclose }: { conn: Connection; onclose: () => void } = $props();

  const engine = $derived(processEngine(conn.kind));
  const canKill = $derived(!conn.read_only);
  const choices = $derived(engine ? killChoices(engine) : []);
  let rows = $state<ProcessRow[]>([]);
  let notices = $state<string[]>([]);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let killing = $state<string | null>(null);

  async function load(): Promise<void> {
    if (!engine || loading) return;
    loading = true;
    try {
      const res = await api.post<QueryResult>(`/connections/${conn.id}/db/query`, {
        statement: PROCESS_SQL[engine],
        read_only: true,
        max_rows: 500,
      });
      rows = parseProcesses(engine, res.columns, res.rows);
      notices = processNotices(engine, rows, res.message);
      error = null;
    } catch (e) {
      error = loadErrorText(e);
    } finally {
      loading = false;
    }
  }
  // Reload when the panel is pointed at another connection; load() reads its
  // own state, so it runs untracked (else every `loading` flip re-ran it).
  $effect(() => {
    void conn.id;
    untrack(() => void load());
  });

  async function kill(p: ProcessRow, c: KillChoice): Promise<void> {
    const sql = engine ? killSql(engine, p, c.action) : null;
    if (!engine || !sql || !canKill) return;
    const shown = p.hidden ? '(query hidden by PostgreSQL)' : queryPreview(p.query);
    const ok = await confirmer.ask(
      `${c.effect}\n\nOn “${conn.name}”${p.user ? `, run by ${p.user}` : ''}:\n${shown}\n\nOtto sends: ${sql}`,
      { title: c.title, confirmLabel: c.title, cancelLabel: engine === 'postgres' ? 'Keep running' : undefined, danger: true },
    );
    if (!ok) return;
    killing = p.id;
    const send = (confirm_write: boolean) => api.post<QueryResult>(`/connections/${conn.id}/db/query`, { statement: sql, confirm_write });
    try {
      let res: QueryResult;
      try {
        res = await send(false);
      } catch (e) {
        // Production: the server asks for the typed confirmation first.
        if (!isWriteBlocked(e)) throw e;
        if ((await database.confirmGuardedWrite(conn, e)) !== 'confirmed') return;
        res = await send(true);
      }
      const notDone = killNotDone(engine, p, res.rows);
      if (notDone) toasts.warn(`${c.title}: nothing to stop`, notDone);
      else toasts.success(`${c.title} sent`, `${p.id} on ${conn.name}`);
      await load();
    } catch (e) {
      // A refusal (no privilege, Read-only, production) carries the server's message.
      toastError(`Couldn’t ${c.label.toLowerCase()} ${p.id}`, e);
    } finally {
      killing = null;
    }
  }

  const hasDb = $derived(engine === 'postgres' || engine === 'mongodb');
  const seconds = (s: number | null): string => (s === null ? '' : s < 1 ? '<1 s' : `${Math.round(s)} s`);
</script>

<Modal title={`Processes · ${conn.name}`} width={760} {onclose}>
  <div class="pl-bar">
    <span class="pl-note">
      {rows.length} running{conn.read_only ? ' · Read-only connection: Kill is not available' : ''}
    </span>
    <button class="btn" type="button" onclick={() => void load()} disabled={loading} title="Refresh the list" data-testid="db-processes-refresh">
      <Icon name="refresh" size={13} /> Refresh
    </button>
  </div>
  {#if !engine}
    <EmptyState title="Not available for this database" body="The process list covers MySQL, PostgreSQL, ClickHouse and MongoDB connections." />
  {:else}
    <LoadState what="the process list" {loading} {error} empty={rows.length === 0} onretry={() => void load()} rows={4}>
      {#snippet emptyView()}<EmptyState title="Nothing running" body="Refresh to check again." />{/snippet}
      {#each notices as n (n)}<p class="pl-notice" role="note" data-testid="db-processes-notice"><Icon name="info" size={13} /> {n}</p>{/each}
      <div class="pl-scroll">
        <table class="pl-table" data-testid="db-processes">
          <thead>
            <tr><th>Id</th><th>User</th>{#if hasDb}<th>{engine === 'mongodb' ? 'Namespace' : 'Database'}</th>{/if}<th>Time</th><th>State</th><th>Query</th>{#if canKill}<th><span class="sr-only">Kill</span></th>{/if}</tr>
          </thead>
          <tbody>
            {#each rows as p (p.id)}
              <tr>
                <td class="mono">{p.id}</td>
                <td>{p.user}</td>
                {#if hasDb}<td>{p.database}</td>{/if}
                <td class="num">{seconds(p.seconds)}</td>
                <td>{p.state}</td>
                {#if p.hidden}
                  <td class="pl-hidden" title="PostgreSQL hides another role’s query from a login without pg_read_all_stats">Hidden by PostgreSQL</td>
                {:else}
                  <td class="mono pl-query" title={p.query} dir="ltr">{queryPreview(p.query, 160)}</td>
                {/if}
                {#if canKill}
                  <td>
                    <div class="pl-actions">
                      {#each choices as c (c.action)}
                        <button class="btn" type="button" disabled={killing !== null || !killSql(engine, p, c.action)} onclick={() => void kill(p, c)} aria-label={`${c.title} ${p.id}`} title={`${c.effect} (asks first)`} data-testid={`db-processes-${c.action}`}>{c.label}</button>
                      {/each}
                    </div>
                  </td>
                {/if}
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    </LoadState>
  {/if}
</Modal>

<style>
  .pl-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin-block-end: 8px;
  }
  .pl-note {
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .pl-scroll {
    max-block-size: 60vh;
    overflow: auto;
  }
  .pl-table {
    inline-size: 100%;
    border-collapse: collapse;
    font-size: var(--fs-s);
  }
  .pl-table th,
  .pl-table td {
    text-align: start;
    padding-block: 4px;
    padding-inline: 6px;
    border-block-end: 1px solid var(--border);
    vertical-align: top;
  }
  .pl-table th {
    color: var(--text-dim);
    font-weight: 500;
  }
  .num {
    text-align: end;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .pl-notice {
    display: flex;
    gap: 6px;
    align-items: baseline;
    margin-block: 0 8px;
    padding-block: 6px;
    padding-inline: 8px;
    border-radius: var(--radius-s);
    background: var(--info-soft);
    color: var(--text);
    font-size: var(--fs-s);
  }
  .pl-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .pl-hidden {
    color: var(--text-dim);
    font-style: italic;
  }
  .pl-query {
    max-inline-size: 320px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
