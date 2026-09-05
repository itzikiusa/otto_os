<script lang="ts">
  import { resourceAccess } from '../../lib/stores/resource-access.svelte';
  // Athena: three-pane like the DB Explorer — catalog tree (databases → tables
  // → columns) feeding `CodeEditor` sql completion, an editor with workgroup /
  // database selectors + Run (⌘↵, Edit-gated), results through the shared
  // `ResultsGrid` (`connectionId={null}` — Athena rows are never editable), a
  // status bar (state · scanned bytes · $5/TB estimate · Cancel) and a History
  // tab. Status is polled every 1 s while QUEUED/RUNNING.
  import { onDestroy, untrack } from 'svelte';
  import type { Completion, CompletionContext, CompletionResult } from '@codemirror/autocomplete';
  import { aws } from '../../lib/stores/aws.svelte';
  import { awsApi, isLoginRequired } from '../../lib/api/aws';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { copyTextOrThrow } from '../../lib/clipboard';
  import CodeEditor from '../../lib/components/CodeEditor.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import ResultsGrid from '../database/ResultsGrid.svelte';
  import { athenaCostUsd, fmtAgo, fmtBytes, fmtMs } from './util';
  import type {
    AthenaExecution,
    AthenaQueryState,
    AthenaTable,
    AwsAccount,
    QueryResult,
  } from '../../lib/api/types';

  interface Props {
    account: AwsAccount;
    onsignin: () => void;
  }
  let { account, onsignin }: Props = $props();

  $effect(() => { void resourceAccess.load('aws_account', account.id); });
  const canRun = $derived(resourceAccess.can('aws_account', account.id, 'athena_query', 'aws_athena', 'edit'));
  const catalog = $derived(aws.athena[account.id] ?? null);
  let catLoading = $state(false);
  let catError = $state('');
  let treeFilter = $state('');
  let openDbs = $state<Record<string, boolean>>({});
  let openTables = $state<Record<string, boolean>>({});
  let tablesLoading = $state<Record<string, boolean>>({});

  // Editor state — the SQL text is remembered per account. The view is
  // remounted per account (AwsPage {#key}s on account+service), so reading the
  // initial account id for the storage keys is intentional.
  // svelte-ignore state_referenced_locally
  const acctId = account.id;
  const SQL_KEY = `otto_aws_athena_sql_${acctId}`;
  const WG_KEY = `otto_aws_athena_wg_${acctId}`;
  const DB_KEY = `otto_aws_athena_db_${acctId}`;
  let sql = $state(localStorage.getItem(SQL_KEY) ?? '');
  let workgroup = $state(localStorage.getItem(WG_KEY) ?? '');
  let database = $state(localStorage.getItem(DB_KEY) ?? '');
  let editorSel = $state<{ text: string; cursor: number }>({ text: '', cursor: 0 });

  // Execution state.
  let qid = $state<string | null>(null);
  let qstate = $state<AthenaQueryState | null>(null);
  let qreason = $state('');
  let scanned = $state(0);
  let execMs = $state(0);
  let result = $state<QueryResult | null>(null);
  let resultError = $state<string | null>(null);
  let ranSql = $state('');
  let submitting = $state(false);
  let pollTimer: ReturnType<typeof setTimeout> | null = null;
  const running = $derived(qstate === 'QUEUED' || qstate === 'RUNNING' || submitting);

  type Tab = 'results' | 'history';
  let tab = $state<Tab>('results');
  let history = $state<AthenaExecution[]>([]);
  let historyLoading = $state(false);
  let historyError = $state('');

  async function loadCatalog(): Promise<void> {
    catLoading = true;
    try {
      const c = await aws.loadAthenaCatalog(account.id);
      catError = '';
      if (!workgroup && c.workgroups.length) workgroup = c.workgroups.find((w) => w.name === 'primary')?.name ?? c.workgroups[0].name;
      if (!database && c.databases.length) database = c.databases[0];
    } catch (e) {
      catError = e instanceof Error ? e.message : String(e);
    } finally {
      catLoading = false;
    }
  }

  async function toggleDb(db: string): Promise<void> {
    const open = !openDbs[db];
    openDbs = { ...openDbs, [db]: open };
    if (open && !catalog?.tables[db] && !tablesLoading[db]) {
      tablesLoading = { ...tablesLoading, [db]: true };
      try {
        await aws.loadAthenaTables(account.id, db);
      } catch (e) {
        toasts.error(`Couldn't list tables in ${db}`, e instanceof Error ? e.message : String(e));
      } finally {
        tablesLoading = { ...tablesLoading, [db]: false };
      }
    }
  }

  $effect(() => {
    untrack(() => {
      if (!catalog) void loadCatalog();
      else {
        if (!workgroup && catalog.workgroups.length) workgroup = catalog.workgroups[0].name;
        if (!database && catalog.databases.length) database = catalog.databases[0];
      }
    });
  });

  // Persist the editor + selectors.
  $effect(() => {
    localStorage.setItem(SQL_KEY, sql);
  });
  $effect(() => {
    if (workgroup) localStorage.setItem(WG_KEY, workgroup);
    if (database) localStorage.setItem(DB_KEY, database);
  });

  const treeDbs = $derived.by(() => {
    const q = treeFilter.trim().toLowerCase();
    const dbs = catalog?.databases ?? [];
    if (!q) return dbs;
    // Match a database by name OR by any loaded table name inside it.
    return dbs.filter(
      (d) => d.toLowerCase().includes(q) || (catalog?.tables[d] ?? []).some((t) => t.name.toLowerCase().includes(q)),
    );
  });
  function treeTables(db: string): AthenaTable[] {
    const q = treeFilter.trim().toLowerCase();
    const ts = catalog?.tables[db] ?? [];
    return q && !db.toLowerCase().includes(q) ? ts.filter((t) => t.name.toLowerCase().includes(q)) : ts;
  }

  // ── completion from the tree ──
  const KEYWORDS = [
    'SELECT', 'FROM', 'WHERE', 'GROUP BY', 'ORDER BY', 'LIMIT', 'JOIN', 'LEFT JOIN', 'INNER JOIN', 'ON', 'AS',
    'AND', 'OR', 'NOT', 'IN', 'IS NULL', 'IS NOT NULL', 'DISTINCT', 'COUNT', 'SUM', 'AVG', 'MIN', 'MAX',
    'HAVING', 'UNION ALL', 'WITH', 'CASE', 'WHEN', 'THEN', 'ELSE', 'END', 'CAST', 'DATE', 'TIMESTAMP',
    'SHOW TABLES', 'SHOW DATABASES', 'DESCRIBE', 'CREATE TABLE', 'MSCK REPAIR TABLE', 'UNNEST', 'CROSS JOIN',
    'date_trunc', 'date_add', 'from_unixtime', 'to_unixtime', 'json_extract_scalar', 'approx_distinct',
    'regexp_like', 'split_part', 'lower', 'upper', 'coalesce', 'try_cast', 'element_at', 'cardinality',
  ];
  const TOKEN_RE = /[\w.]*$/;
  function completionSource(ctx: CompletionContext): CompletionResult | null {
    const before = ctx.matchBefore(TOKEN_RE);
    const word = before?.text ?? '';
    if (!ctx.explicit && word.length === 0) return null;
    const dot = word.lastIndexOf('.');
    const qualifier = dot >= 0 ? word.slice(0, dot) : '';
    const from = before ? before.from + (dot >= 0 ? dot + 1 : 0) : ctx.pos;
    const options: Completion[] = [];
    const cat = catalog;
    if (cat) {
      if (qualifier) {
        // `db.` → tables of that db; `table.` / `db.table.` → columns.
        const q = qualifier.split('.');
        const dbName = q.length === 2 ? q[0] : cat.databases.includes(q[0]) ? q[0] : database;
        const tableName = q.length === 2 ? q[1] : cat.databases.includes(q[0]) ? null : q[0];
        if (tableName) {
          const t = (cat.tables[dbName] ?? []).find((x) => x.name === tableName);
          for (const c of t?.columns ?? []) options.push({ label: c.name, type: 'property', detail: c.type, boost: 2 });
        } else {
          for (const t of cat.tables[dbName] ?? []) options.push({ label: t.name, type: 'class', detail: t.type ?? 'table', boost: 2 });
        }
      } else {
        for (const d of cat.databases) options.push({ label: d, type: 'namespace', detail: 'database' });
        for (const t of cat.tables[database] ?? []) {
          options.push({ label: t.name, type: 'class', detail: `table · ${database}`, boost: 1 });
          for (const c of t.columns) options.push({ label: c.name, type: 'property', detail: `${t.name}.${c.type}` });
        }
        for (const [db, ts] of Object.entries(cat.tables)) {
          if (db === database) continue;
          for (const t of ts) options.push({ label: t.name, type: 'class', detail: `table · ${db}`, apply: `${db}.${t.name}` });
        }
      }
    }
    if (!qualifier) for (const k of KEYWORDS) options.push({ label: k, type: 'keyword', boost: -1 });
    return options.length ? { from, options, validFor: /^[\w]*$/ } : null;
  }

  // ── run / poll ──
  function stopPoll(): void {
    if (pollTimer) clearTimeout(pollTimer);
    pollTimer = null;
  }

  async function run(): Promise<void> {
    if (!canRun || running) return;
    const text = (editorSel.text.trim() || sql).trim();
    if (!text) return;
    submitting = true;
    resultError = null;
    result = null;
    qreason = '';
    scanned = 0;
    execMs = 0;
    ranSql = text;
    tab = 'results';
    try {
      const r = await awsApi.athenaQuery(account.id, {
        sql: text,
        database: database || undefined,
        workgroup: workgroup || undefined,
      });
      qid = r.query_execution_id;
      qstate = 'QUEUED';
      schedulePoll(0);
    } catch (e) {
      qstate = 'FAILED';
      resultError = e instanceof Error ? e.message : String(e);
    } finally {
      submitting = false;
    }
  }

  function schedulePoll(ms: number): void {
    stopPoll();
    pollTimer = setTimeout(() => void poll(), ms);
  }

  async function poll(): Promise<void> {
    if (!qid) return;
    const id = qid;
    try {
      const s = await awsApi.athenaStatus(account.id, id);
      if (qid !== id) return; // a newer query superseded this one
      qstate = s.state;
      scanned = s.stats?.data_scanned_bytes ?? 0;
      execMs = s.stats?.execution_ms ?? 0;
      if (s.state === 'QUEUED' || s.state === 'RUNNING') {
        schedulePoll(1000);
        return;
      }
      if (s.state === 'SUCCEEDED') {
        result = s.result ?? { columns: [], rows: [], stats: { duration_ms: execMs, row_count: 0 }, truncated: false };
      } else {
        qreason = s.reason ?? '';
        resultError = `${s.state}${s.reason ? `: ${s.reason}` : ''}`;
      }
      void loadHistory();
    } catch (e) {
      if (qid !== id) return;
      qstate = 'FAILED';
      resultError = e instanceof Error ? e.message : String(e);
    }
  }

  async function cancel(): Promise<void> {
    if (!qid) return;
    try {
      await awsApi.athenaCancel(account.id, qid);
      toasts.info('Cancel requested');
    } catch (e) {
      toasts.error('Cancel failed', e instanceof Error ? e.message : String(e));
    }
  }

  async function loadHistory(): Promise<void> {
    historyLoading = true;
    try {
      history = (await awsApi.athenaHistory(account.id, workgroup || undefined, 50)).executions;
      historyError = '';
    } catch (e) {
      historyError = e instanceof Error ? e.message : String(e);
    } finally {
      historyLoading = false;
    }
  }

  /** Open a past execution: put its SQL in the editor and fetch its result. */
  function openExecution(x: AthenaExecution): void {
    sql = x.query;
    stopPoll();
    qid = x.id;
    qstate = x.state;
    ranSql = x.query;
    result = null;
    resultError = null;
    scanned = x.data_scanned_bytes ?? 0;
    execMs = x.execution_ms ?? 0;
    tab = 'results';
    schedulePoll(0);
  }

  function insertTable(db: string, t: AthenaTable): void {
    sql = `SELECT *\nFROM ${db}.${t.name}\nLIMIT 100;`;
    database = db;
  }

  async function copy(text: string, what: string): Promise<void> {
    try {
      await copyTextOrThrow(text);
      toasts.success(`Copied ${what}`);
    } catch (e) {
      toasts.error('Copy failed', e instanceof Error ? e.message : String(e));
    }
  }

  function tableMenu(e: MouseEvent | KeyboardEvent, db: string, t: AthenaTable): void {
    ctxMenu.show(e, [
      { label: 'SELECT * … LIMIT 100', icon: 'play', action: () => insertTable(db, t) },
      { label: 'Insert name', icon: 'edit', action: () => (sql = `${sql}${sql && !sql.endsWith(' ') ? ' ' : ''}${db}.${t.name}`) },
      { label: 'Copy qualified name', icon: 'copy', action: () => void copy(`${db}.${t.name}`, 'name') },
      { separator: true },
      { label: 'DESCRIBE', icon: 'info', action: () => { sql = `DESCRIBE ${db}.${t.name};`; } },
    ]);
  }

  onDestroy(stopPoll);

  const loginNeeded = $derived(isLoginRequired(new Error(catError)));
  let treeOpen = $state(!viewport.isMobile);
</script>

<div class="ath" class:mobile={viewport.isMobile}>
  {#if treeOpen}
    <aside class="tree" aria-label="Athena catalog">
      <div class="tree-head">
        <label class="tf">
          <Icon name="search" size={12} />
          <input type="search" bind:value={treeFilter} placeholder="Filter catalog…" aria-label="Filter catalog" />
        </label>
        <button class="icon-btn" onclick={() => void loadCatalog()} title="Reload catalog" aria-label="Reload catalog" disabled={catLoading}><Icon name="refresh" size={12} /></button>
        {#if viewport.isMobile}
          <button class="icon-btn" onclick={() => (treeOpen = false)} aria-label="Hide catalog"><Icon name="x" size={12} /></button>
        {/if}
      </div>
      {#if catLoading && !catalog}
        <div class="pad"><Skeleton rows={6} /></div>
      {:else if catError && !catalog}
        <EmptyState icon="db" title="Catalog unavailable" body={catError} actionLabel={loginNeeded ? 'Sign in' : 'Retry'} onaction={loginNeeded ? onsignin : () => void loadCatalog()} />
      {:else}
        <ul class="dbs">
          {#each treeDbs as db (db)}
            {@const open = openDbs[db] === true || treeFilter.trim() !== ''}
            <li>
              <button class="node" class:cur={db === database} onclick={() => void toggleDb(db)} aria-expanded={open}>
                <Icon name={open ? 'chevronDown' : 'chevronRight'} size={11} />
                <Icon name="db" size={12} />
                <span class="nlabel">{db}</span>
              </button>
              {#if open}
                {#if tablesLoading[db]}
                  <div class="sub dim">loading…</div>
                {:else if !catalog?.tables[db]}
                  <div class="sub"><button class="link" onclick={() => void toggleDb(db)}>load tables</button></div>
                {:else}
                  <ul class="tables">
                    {#each treeTables(db) as t (t.name)}
                      {@const tk = `${db}.${t.name}`}
                      {@const topen = openTables[tk] === true}
                      <li>
                        <button
                          class="node"
                          onclick={() => (openTables = { ...openTables, [tk]: !topen })}
                          ondblclick={() => insertTable(db, t)}
                          oncontextmenu={(e) => tableMenu(e, db, t)}
                          aria-expanded={topen}
                          title={`${tk} · double-click to SELECT`}
                        >
                          <Icon name={topen ? 'chevronDown' : 'chevronRight'} size={11} />
                          <Icon name="grid" size={12} />
                          <span class="nlabel">{t.name}</span>
                          <span class="dim cnt">{t.columns.length}</span>
                        </button>
                        {#if topen}
                          <ul class="cols">
                            {#each t.columns as c (c.name)}
                              <li class="col"><span class="mono">{c.name}</span><span class="dim">{c.type}</span></li>
                            {/each}
                          </ul>
                        {/if}
                      </li>
                    {:else}
                      <li class="sub dim">no tables</li>
                    {/each}
                  </ul>
                {/if}
              {/if}
            </li>
          {:else}
            <li class="sub dim">{treeFilter ? 'No matches.' : 'No databases.'}</li>
          {/each}
        </ul>
      {/if}
    </aside>
  {/if}

  <section class="main">
    <div class="bar">
      {#if viewport.isMobile && !treeOpen}
        <button class="icon-btn" onclick={() => (treeOpen = true)} aria-label="Show catalog" title="Catalog"><Icon name="sidebar" size={13} /></button>
      {/if}
      <label class="sel"><span class="lbl">Workgroup</span>
        <select bind:value={workgroup} aria-label="Workgroup">
          {#each catalog?.workgroups ?? [] as w (w.name)}<option value={w.name} disabled={w.state !== 'ENABLED'}>{w.name}{w.output_location ? '' : ' (no output location)'}</option>{/each}
        </select>
      </label>
      <label class="sel"><span class="lbl">Database</span>
        <select bind:value={database} aria-label="Database">
          {#each catalog?.databases ?? [] as d (d)}<option value={d}>{d}</option>{/each}
        </select>
      </label>
      <span class="spacer"></span>
      {#if running && qid}
        <button class="ghost sm" onclick={() => void cancel()}><Icon name="x" size={12} /> Cancel</button>
      {/if}
      <button
        class="primary sm"
        onclick={() => void run()}
        disabled={!canRun || running || !sql.trim()}
        title={canRun ? 'Run (⌘↵) — runs the selection when there is one' : 'Needs Edit on Athena'}
        data-testid="athena-run"
      >
        <Icon name="play" size={12} /> {running ? 'Running…' : editorSel.text.trim() ? 'Run selection' : 'Run'}
      </button>
    </div>

    <div class="editor">
      <CodeEditor
        path={`athena-${account.id}.sql`}
        content={sql}
        root=""
        language="sql"
        readOnly={false}
        minimal={true}
        completionSource={completionSource}
        onchange={(v) => (sql = v)}
        onsubmit={() => void run()}
        onselect={(s) => (editorSel = s)}
      />
    </div>

    <div class="status" role="status">
      {#if qstate}
        <span class="st {qstate.toLowerCase()}">{qstate}</span>
        <span class="dim">scanned <strong>{fmtBytes(scanned)}</strong></span>
        <span class="dim">≈ <strong>{athenaCostUsd(scanned)}</strong> <span title="$5 per TB scanned, 10 MB minimum">@ $5/TB</span></span>
        {#if execMs}<span class="dim">{fmtMs(execMs)}</span>{/if}
        {#if qid}<button class="link mono" onclick={() => qid && void copy(qid, 'execution id')} title="Copy execution id">{qid.slice(0, 8)}…</button>{/if}
      {:else}
        <span class="dim">{canRun ? 'Write a query and press ⌘↵.' : 'You can browse the catalog and history; running queries needs Edit on Athena.'}</span>
      {/if}
      <span class="spacer"></span>
      <div class="tabs" role="tablist">
        <button role="tab" aria-selected={tab === 'results'} class:on={tab === 'results'} onclick={() => (tab = 'results')}>Results</button>
        <button role="tab" aria-selected={tab === 'history'} class:on={tab === 'history'} onclick={() => { tab = 'history'; void loadHistory(); }}>History</button>
      </div>
    </div>

    <div class="results">
      {#if tab === 'results'}
        {#if !qstate && !result}
          <EmptyState icon="db" title="No results yet" body="Run a query to see rows here." />
        {:else}
          <ResultsGrid {result} error={resultError} statement={ranSql} connectionId={null} running={running} />
        {/if}
      {:else if historyLoading && history.length === 0}
        <div class="pad"><Skeleton rows={6} /></div>
      {:else if historyError}
        <EmptyState icon="clock" title="History unavailable" body={historyError} actionLabel="Retry" onaction={() => void loadHistory()} />
      {:else if history.length === 0}
        <EmptyState icon="clock" title="No recent executions" body={`Nothing has run in workgroup ${workgroup || '—'} lately.`} />
      {:else}
        <table class="hist">
          <thead><tr><th>State</th><th>Query</th><th class="hide-sm">Submitted</th><th class="num hide-sm">Scanned</th><th class="num hide-sm">Time</th></tr></thead>
          <tbody>
            {#each history as x (x.id)}
              <tr class="trow" tabindex="0" onclick={() => openExecution(x)} onkeydown={(e) => { if (e.key === 'Enter') openExecution(x); }} oncontextmenu={(e) => ctxMenu.show(e, [
                { label: 'Load into editor', icon: 'edit', action: () => { sql = x.query; } },
                { label: 'Open result', icon: 'play', action: () => openExecution(x) },
                { label: 'Copy SQL', icon: 'copy', action: () => void copy(x.query, 'SQL') },
                { label: 'Copy execution id', icon: 'copy', action: () => void copy(x.id, 'id') },
              ])}>
                <td><span class="st {x.state.toLowerCase()}">{x.state}</span></td>
                <td class="q mono" title={x.query}>{x.query.replace(/\s+/g, ' ').slice(0, 160)}</td>
                <td class="dim hide-sm">{fmtAgo(x.submitted_at)}</td>
                <td class="num mono hide-sm">{fmtBytes(x.data_scanned_bytes)}</td>
                <td class="num mono hide-sm">{fmtMs(x.execution_ms)}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </div>
  </section>
</div>

<style>
  .ath {
    flex: 1;
    min-height: 0;
    position: relative; /* scopes the phone catalog overlay to this pane */
    display: grid;
    grid-template-columns: minmax(200px, 240px) minmax(0, 1fr);
  }
  .ath.mobile {
    grid-template-columns: minmax(0, 1fr);
  }
  .ath.mobile .tree {
    position: absolute;
    inset: 0;
    z-index: 5;
    background: var(--surface);
  }
  .tree {
    border-right: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow: hidden;
    background: var(--bg-sidebar);
    font-size: 12.5px;
  }
  .tree-head {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 8px;
    border-bottom: 1px solid var(--border);
  }
  .tf {
    display: flex;
    align-items: center;
    gap: 4px;
    flex: 1;
    height: 26px;
    padding: 0 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--bg);
    color: var(--text-dim);
  }
  .tf input {
    flex: 1;
    min-width: 0;
    border: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: 12px;
    outline: none;
  }
  .dbs,
  .tables,
  .cols {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .dbs {
    overflow: auto;
    flex: 1;
    padding: 4px 0;
  }
  .node {
    display: flex;
    align-items: center;
    gap: 4px;
    width: 100%;
    padding: 3px 8px;
    border: 0;
    background: transparent;
    color: var(--text);
    text-align: left;
    cursor: pointer;
    font: inherit;
    font-size: 12.5px;
  }
  .node:hover {
    background: var(--surface-2);
  }
  .node.cur .nlabel {
    font-weight: 600;
  }
  .nlabel {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .cnt {
    font-size: 10.5px;
  }
  .tables .node {
    padding-left: 22px;
  }
  .cols {
    padding: 0 0 2px 44px;
  }
  .col {
    display: flex;
    justify-content: space-between;
    gap: 8px;
    font-size: 11.5px;
    padding: 1px 8px 1px 0;
    overflow: hidden;
  }
  .col span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sub {
    padding: 2px 8px 2px 24px;
    font-size: 12px;
  }
  .dim {
    color: var(--text-dim);
  }
  .pad {
    padding: 10px;
  }
  .main {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    position: relative;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
    flex-wrap: wrap;
    background: var(--surface);
  }
  .sel {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 12px;
  }
  .lbl {
    color: var(--text-dim);
  }
  .sel select {
    height: 26px;
    max-width: 200px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--bg);
    color: var(--text);
    font: inherit;
    font-size: 12px;
  }
  .spacer {
    flex: 1;
  }
  .editor {
    height: 34%;
    min-height: 120px;
    max-height: 50vh;
    border-bottom: 1px solid var(--border);
    overflow: hidden;
  }
  .status {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 10px;
    min-height: 32px;
    border-bottom: 1px solid var(--border);
    font-size: 12px;
    flex-wrap: wrap;
    background: var(--surface);
  }
  .st {
    font-size: 10.5px;
    font-weight: 700;
    padding: 1px 7px;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text-dim);
    letter-spacing: 0.04em;
  }
  .st.succeeded {
    color: var(--status-working);
    background: color-mix(in srgb, var(--status-working) 16%, transparent);
  }
  .st.running,
  .st.queued {
    color: var(--status-warn);
    background: color-mix(in srgb, var(--status-warn) 16%, transparent);
  }
  .st.failed,
  .st.cancelled {
    color: var(--status-exited);
    background: color-mix(in srgb, var(--status-exited) 16%, transparent);
  }
  .tabs {
    display: flex;
    gap: 2px;
  }
  .tabs button {
    padding: 7px 10px;
    border: 0;
    border-bottom: 2px solid transparent;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    font-size: 12.5px;
  }
  .tabs button.on {
    color: var(--text);
    border-bottom-color: var(--accent);
  }
  .results {
    flex: 1;
    min-height: 0;
    overflow: auto;
    display: flex;
    flex-direction: column;
  }
  .hist {
    width: 100%;
    border-collapse: collapse;
    font-size: 12.5px;
  }
  .hist th {
    position: sticky;
    top: 0;
    z-index: 1;
    background: var(--surface);
    text-align: left;
    font-weight: 600;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
  }
  .hist td {
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 480px;
  }
  .hist .num {
    text-align: right;
  }
  .trow {
    cursor: pointer;
  }
  .trow:hover,
  .trow:focus-visible {
    background: var(--surface-2);
    outline: none;
  }
  .q {
    font-size: 11.5px;
  }
  .link {
    border: 0;
    background: transparent;
    color: var(--accent);
    cursor: pointer;
    padding: 0;
    font: inherit;
    font-size: 12px;
  }
  .primary,
  .ghost {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 4px 10px;
    border-radius: var(--radius-m);
    cursor: pointer;
    font-size: 12px;
  }
  .primary {
    border: 1px solid var(--accent);
    background: var(--accent);
    color: var(--accent-contrast);
    font-weight: 600;
  }
  .primary:disabled {
    opacity: 0.55;
    cursor: default;
  }
  .ghost {
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text);
  }
  .icon-btn {
    display: inline-grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: transparent;
    color: var(--text);
    cursor: pointer;
  }
  @media (max-width: 640px) {
    .hide-sm {
      display: none;
    }
  }
</style>
