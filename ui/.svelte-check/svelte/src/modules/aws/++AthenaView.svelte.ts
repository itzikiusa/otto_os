///<reference types="svelte" />
;
import { announce } from '../../lib/announce.svelte';
import { rowMenu } from '../../lib/rowMenu';
import { toastError } from '../../lib/toastError';
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
import LoadState from '../../lib/components/LoadState.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import Icon from '../../lib/components/Icon.svelte';
import ResultsGrid from '../database/ResultsGrid.svelte';
import RegionPicker from './RegionPicker.svelte';
import { athenaCostUsd, fmtAgo, fmtBytes, fmtMs, awsErrorText } from './util';
import { awsLoadText } from './loadText';
import Tabs from '../../lib/components/Tabs.svelte';
import { statusPollMs } from '../../lib/pollBackoff';
import { confirmProd, isProdEnv } from '../../lib/confirmProd';
import { athenaIsRead, athenaLeadKeyword } from './athena-sql';
import type { AthenaExecution, AthenaQueryState, AthenaTable, AwsAccount, QueryResult } from '../../lib/api/types';
;

  interface Props {
    account: AwsAccount;
    onsignin: () => void;
  };;

  type Tab = 'results' | 'history';;function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { account, onsignin }: Props = $props();

  $effect(() => { void resourceAccess.load('aws_account', account.id); });
  const canRun = $derived(resourceAccess.can('aws_account', account.id, 'athena_query', 'aws_athena', 'edit'));
  // A-1: workgroups, catalogs and history are per region.
  // svelte-ignore state_referenced_locally
  let region = $state(account.region);
  /** `''` = the account default region, else the override sent as `?region=`. */
  const rq = $derived(region === account.region ? '' : region);
  const catalog = $derived(aws.athena[`${account.id}:${rq}`] ?? null);
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
  // Storage can throw (private window, blocked site data): the editor then
  // just starts empty and nothing is remembered.
  function readKey(key: string): string {
    try {
      return localStorage.getItem(key) ?? '';
    } catch {
      return '';
    }
  }
  function writeKey(key: string, value: string): void {
    try {
      localStorage.setItem(key, value);
    } catch {
      /* not persisted — the session keeps working */
    }
  }
  let sql = $state(readKey(SQL_KEY));
  let workgroup = $state(readKey(WG_KEY));
  let database = $state(readKey(DB_KEY));
  let editorSel = $state<{ text: string; cursor: number }>({ text: '', cursor: 0 });

  // Execution state.
  let qid = $state<string | null>(null);
  /** Region the running query was started in (status/cancel must follow it). */
  let qRegion = '';
  let executionGeneration = 0;
  let qstate = $state<AthenaQueryState | null>(null);
  let scanned = $state(0);
  let execMs = $state(0);
  let result = $state<QueryResult | null>(null);
  let resultError = $state<string | null>(null);
  let ranSql = $state('');
  let submitting = $state(false);
  let pollTimer: ReturnType<typeof setTimeout> | null = null;
  const running = $derived(qstate === 'QUEUED' || qstate === 'RUNNING' || submitting);

  let tab = $state<Tab>('results');
  let history = $state<AthenaExecution[]>([]);
  let historyLoading = $state(false);
  let historyError = $state('');
  let historyRegion = '';
  let historyGeneration = 0;
  let catalogGeneration = 0;

  async function loadCatalog(): Promise<void> {
    const seq = ++catalogGeneration;
    const requestedRegion = rq;
    catLoading = true;
    try {
      const c = await aws.loadAthenaCatalog(account.id, rq);
      if (seq !== catalogGeneration || requestedRegion !== rq) return;
      catError = '';
      // A workgroup / database remembered from another region does not exist here.
      if (workgroup && !c.workgroups.some((w) => w.name === workgroup)) workgroup = '';
      if (database && !c.databases.includes(database)) database = '';
      if (!workgroup && c.workgroups.length) workgroup = c.workgroups.find((w) => w.name === 'primary')?.name ?? c.workgroups[0].name;
      if (!database && c.databases.length) database = c.databases[0];
    } catch (e) {
      if (seq !== catalogGeneration || requestedRegion !== rq) return;
      catError = awsLoadText(e);
    } finally {
      if (seq === catalogGeneration && requestedRegion === rq) catLoading = false;
    }
  }

  async function toggleDb(db: string): Promise<void> {
    const open = !openDbs[db];
    openDbs = { ...openDbs, [db]: open };
    if (open && !catalog?.tables[db] && !tablesLoading[db]) {
      tablesLoading = { ...tablesLoading, [db]: true };
      try {
        await aws.loadAthenaTables(account.id, db, rq);
      } catch (e) {
        toastError(`Couldn’t list tables in ${db}`, e);
      } finally {
        tablesLoading = { ...tablesLoading, [db]: false };
      }
    }
  }

  $effect(() => {
    void rq;
    untrack(() => {
      historyGeneration++;
      history = [];
      historyError = '';
      historyLoading = false;
      if (tab === 'history') void loadHistory();
      if (!catalog) void loadCatalog();
      else {
        if (!workgroup && catalog.workgroups.length) workgroup = catalog.workgroups[0].name;
        if (!database && catalog.databases.length) database = catalog.databases[0];
      }
    });
  });

  // Persist the editor + selectors.
  $effect(() => {
    writeKey(SQL_KEY, sql);
  });
  $effect(() => {
    if (workgroup) writeKey(WG_KEY, workgroup);
    if (database) writeKey(DB_KEY, database);
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
    // DDL / DML (DROP, INSERT, CTAS, ALTER, MSCK…) runs through the same call
    // as a SELECT: confirm where and what first — typed on a prod account —
    // and tell the daemon it was confirmed (it refuses an unconfirmed write
    // on prod).
    const write = !athenaIsRead(text);
    if (write) {
      const prod = isProdEnv(account.environment);
      const ok = await confirmProd({
        env: account.environment,
        verb: `Run ${athenaLeadKeyword(text) || 'statement'}`,
        title: prod ? 'Run a write on production?' : 'Run a write statement?',
        where: `Athena · ${account.name} · ${rq || account.region}${workgroup ? ` · workgroup ${workgroup}` : ''}${database ? ` · ${database}` : ''}`,
        what: text,
        typed: prod ? account.name : undefined,
        danger: true,
      });
      if (!ok || running) return;
    }
    const seq = ++executionGeneration;
    submitting = true;
    resultError = null;
    result = null;
    scanned = 0;
    execMs = 0;
    ranSql = text;
    tab = 'results';
    pollFails = 0;
    statusUnknown = '';
    try {
      const started = rq;
      const r = await awsApi.athenaQuery(account.id, {
        sql: text,
        database: database || undefined,
        workgroup: workgroup || undefined,
        ...(write ? { confirm: true } : {}),
      }, started || undefined);
      if (seq !== executionGeneration) return;
      qRegion = started;
      qid = r.query_execution_id;
      qstate = 'QUEUED';
      pollN = 0;
      schedulePoll(0);
    } catch (e) {
      if (seq !== executionGeneration) return;
      qstate = 'FAILED';
      resultError = awsLoadText(e);
    } finally {
      if (seq === executionGeneration) submitting = false;
    }
  }

  // Status-poll backoff: each poll is an `aws` CLI process (~0.3 s CPU), so a
  // 10-minute scan polled every second burned ~30% of a core. 1,1,2,2,3,5 s…
  // capped at 5 s (15 s while the window is hidden) — lib/pollBackoff, shared
  // with Logs Insights.
  let pollN = 0;
  /** Consecutive failed status polls. A transient error must not mark a
   *  still-running query FAILED (that hid Cancel while it kept scanning):
   *  the state stays as last known, polls retry with backoff, and after
   *  MAX_POLL_FAILS the bar offers "Check again". */
  let pollFails = $state(0);
  const MAX_POLL_FAILS = 5;
  let statusUnknown = $state('');
  function nextPollMs(): number {
    return statusPollMs(pollN++, typeof document !== 'undefined' && document.visibilityState === 'hidden');
  }

  function schedulePoll(ms: number): void {
    stopPoll();
    pollTimer = setTimeout(() => void poll(), ms);
  }

  async function poll(): Promise<void> {
    if (!qid) return;
    const id = qid;
    const seq = executionGeneration;
    const requestedRegion = qRegion;
    try {
      const s = await awsApi.athenaStatus(account.id, id, undefined, undefined, requestedRegion || undefined);
      if (qid !== id || seq !== executionGeneration) return;
      pollFails = 0;
      statusUnknown = '';
      qstate = s.state;
      scanned = s.stats?.data_scanned_bytes ?? 0;
      execMs = s.stats?.execution_ms ?? 0;
      if (s.state === 'QUEUED' || s.state === 'RUNNING') {
        schedulePoll(nextPollMs());
        return;
      }
      if (s.state === 'SUCCEEDED') {
        result = s.result ?? { columns: [], rows: [], stats: { duration_ms: execMs, row_count: 0 }, truncated: false };
      } else {
        resultError = `${s.state}${s.reason ? `: ${s.reason}` : ''}`;
      }
      void loadHistory();
    } catch (e) {
      if (qid !== id || seq !== executionGeneration) return;
      pollFails++;
      statusUnknown = awsLoadText(e);
      if (pollFails < MAX_POLL_FAILS) schedulePoll(Math.min(30_000, 2_000 * 2 ** (pollFails - 1)));
    }
  }

  function checkAgain(): void {
    pollFails = 0;
    statusUnknown = '';
    schedulePoll(0);
  }

  async function cancel(): Promise<void> {
    if (!qid) return;
    try {
      await awsApi.athenaCancel(account.id, qid, qRegion || undefined);
      toasts.info('Cancel requested');
    } catch (e) {
      toastError('Couldn’t cancel', e);
    }
  }

  async function loadHistory(): Promise<void> {
    const seq = ++historyGeneration;
    const requestedRegion = rq;
    const requestedWorkgroup = workgroup;
    historyLoading = true;
    try {
      const rows = (await awsApi.athenaHistory(account.id, requestedWorkgroup || undefined, 50, requestedRegion || undefined)).executions;
      if (seq !== historyGeneration || requestedRegion !== rq || requestedWorkgroup !== workgroup) return;
      history = rows;
      historyRegion = requestedRegion;
      historyError = '';
    } catch (e) {
      if (seq !== historyGeneration || requestedRegion !== rq || requestedWorkgroup !== workgroup) return;
      historyError = awsLoadText(e);
    } finally {
      if (seq === historyGeneration) historyLoading = false;
    }
  }

  /** Open a past execution: put its SQL in the editor and fetch its result. */
  function openExecution(x: AthenaExecution): void {
    if (historyRegion !== rq || !history.includes(x)) return;
    executionGeneration++;
    qRegion = historyRegion;
    submitting = false;
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
    pollN = 0;
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
    } catch {
      toasts.error('Couldn’t copy', 'The clipboard isn’t available here. Select the text and copy it manually.');
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

  onDestroy(() => {
    // Leaving the view cancels a still-running query: nothing would poll it,
    // and Athena bills for every byte it keeps scanning.
    if (qid && (qstate === 'QUEUED' || qstate === 'RUNNING')) {
      void awsApi.athenaCancel(account.id, qid, qRegion || undefined).catch(() => {});
    }
    executionGeneration++;
    historyGeneration++;
    catalogGeneration++;
    stopPoll();
  });
  const loginNeeded = $derived(isLoginRequired(new Error(catError)));
  let treeOpen = $state(!viewport.isMobile);
  $effect(() => { if (statusUnknown) announce(`Query status unknown. ${statusUnknown}`); });
;
async () => {

 { svelteHTML.createElement("div", {  "class":`ath`,});viewport.isMobile;
  if(treeOpen){
     { svelteHTML.createElement("aside", {   "class":`tree`,"aria-label":`Athena catalog`,});
       { svelteHTML.createElement("div", { "class":`tree-head`,});
         { svelteHTML.createElement("label", { "class":`tf`,});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":12,}});}
           { svelteHTML.createElement("input", {          "dir":`ltr`,"type":`search`,"bind:value":treeFilter,"placeholder":`Filter catalog…`,"aria-label":`Filter catalog`,});/*Ωignore_startΩ*/() => treeFilter = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
         { svelteHTML.createElement("button", {         "class":`icon-btn`,"onclick":() => void loadCatalog(),"title":`Refresh catalog`,"aria-label":`Refresh catalog`,"disabled":catLoading,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});} }
        if(viewport.isMobile){
           { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => (treeOpen = false),"aria-label":`Hide catalog`,"title":`Hide catalog`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
        }
       }
      if(catLoading && !catalog){
         { svelteHTML.createElement("div", { "class":`pad`,}); { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":6,"label":`the data catalog`,}});} }
      } else if (catError && !catalog){
         { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL2 = new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {     "what":`the catalog`,"error":awsErrorText(catError),"empty":true,"onretry":() => void loadCatalog(),errorActions:() => { async ()/*Ωignore_positionΩ*/ => {if(loginNeeded){ { svelteHTML.createElement("button", {   "class":`btn primary`,"onclick":onsignin,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`key`,"size":12,}});}   }}};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {errorActions} = $$_etatSdaoL2.$$prop_def;/*Ωignore_endΩ*/ LoadState}
      }else{
         { svelteHTML.createElement("ul", { "class":`dbs`,});
             for(let db of __sveltets_2_ensureArray(treeDbs)){db;
            const open = openDbs[db] === true || treeFilter.trim() !== '';
             { svelteHTML.createElement("li", {});
               { svelteHTML.createElement("button", {      "class":`node`,"onclick":() => void toggleDb(db),"aria-expanded":open,});db === database;
                 { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":open ? 'chevronDown' : 'chevronRight',"size":12,}});}
                 { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`db`,"size":12,}});}
                 { svelteHTML.createElement("span", { "class":`nlabel`,});db; }
               }
              if(open){
                if(tablesLoading[db]){
                   { svelteHTML.createElement("div", { "class":`sub dim`,});  }
                } else if (!catalog?.tables[db]){
                   { svelteHTML.createElement("div", { "class":`sub`,}); { svelteHTML.createElement("button", {   "class":`link`,"onclick":() => void toggleDb(db),});  } }
                }else{
                   { svelteHTML.createElement("ul", { "class":`tables`,});
                       for(let t of __sveltets_2_ensureArray(treeTables(db))){t.name;
                      const tk = `${db}.${t.name}`;
                      const topen = openTables[tk] === true;
                       { svelteHTML.createElement("li", {});
                         {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('button')));{ svelteHTML.createElement("button", __sveltets_2_union($$action_0), {              "class":`node`,"onclick":() => (openTables = { ...openTables, [tk]: !topen }),"ondblclick":() => insertTable(db, t),"oncontextmenu":(e) => tableMenu(e, db, t),"aria-expanded":topen,"title":`${tk} · double-click to SELECT`,});
                           { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":topen ? 'chevronDown' : 'chevronRight',"size":12,}});}
                           { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`grid`,"size":12,}});}
                           { svelteHTML.createElement("span", { "class":`nlabel`,});t.name; }
                           { svelteHTML.createElement("span", { "class":`dim cnt`,});t.columns.length; }
                         }}
                        if(topen){
                           { svelteHTML.createElement("ul", { "class":`cols`,});
                               for(let c of __sveltets_2_ensureArray(t.columns)){c.name;
                               { svelteHTML.createElement("li", { "class":`col`,}); { svelteHTML.createElement("span", { "class":`mono`,});c.name; } { svelteHTML.createElement("span", { "class":`dim`,});c.type; } }
                            }
                           }
                        }
                       }
                    }
                       { svelteHTML.createElement("li", { "class":`sub dim`,});  }
                    
                   }
                }
              }
             }
          }
             { svelteHTML.createElement("li", { "class":`sub dim`,});treeFilter ? 'No matches.' : 'No databases.'; }
          
         }
      }
     }
  }

   { svelteHTML.createElement("section", { "class":`main`,});
     { svelteHTML.createElement("div", { "class":`bar`,});
      if(viewport.isMobile && !treeOpen){
         { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => (treeOpen = true),"aria-label":`Show catalog`,"title":`Catalog`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`sidebar`,"size":13,}});} }
      }
        { const $$_rekciPnoigeR3C = __sveltets_2_ensureComponent(RegionPicker); const $$_rekciPnoigeR3 = new $$_rekciPnoigeR3C({ target: __sveltets_2_any(), props: {   account,"service":`athena`,region,}});/*Ωignore_startΩ*/() => region = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rekciPnoigeR3.$$bindings = 'region';}
       { svelteHTML.createElement("label", { "class":`sel`,}); { svelteHTML.createElement("span", { "class":`lbl`,});  }
         { svelteHTML.createElement("select", {   "bind:value":workgroup,"aria-label":`Workgroup`,});/*Ωignore_startΩ*/() => workgroup = __sveltets_2_any(null);/*Ωignore_endΩ*/
             for(let w of __sveltets_2_ensureArray(catalog?.workgroups ?? [])){w.name; { svelteHTML.createElement("option", {   "value":w.name,"disabled":w.state !== 'ENABLED',});w.name;w.output_location ? '' : ' (no output location)'; }}
         }
       }
       { svelteHTML.createElement("label", { "class":`sel`,}); { svelteHTML.createElement("span", { "class":`lbl`,});  }
         { svelteHTML.createElement("select", {   "bind:value":database,"aria-label":`Database`,});/*Ωignore_startΩ*/() => database = __sveltets_2_any(null);/*Ωignore_endΩ*/
             for(let d of __sveltets_2_ensureArray(catalog?.databases ?? [])){d; { svelteHTML.createElement("option", { "value":d,});d; }}
         }
       }
       { svelteHTML.createElement("span", { "class":`spacer`,}); }
      if(running && qid){
        if(statusUnknown){
           { svelteHTML.createElement("span", {   "class":`dim`,"title":statusUnknown,}); pollFails < MAX_POLL_FAILS ? ' · retrying' : ''; }
          if(pollFails >= MAX_POLL_FAILS){
             { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":checkAgain,});  }
          }
        }
         { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void cancel(),}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}  }
      }
       { svelteHTML.createElement("button", {           "class":`btn primary small`,"onclick":() => void run(),"disabled":!canRun || running || !sql.trim(),"title":canRun ? 'Run (⌘↵) — runs the selection when there is one' : 'Needs Edit on Athena',...__sveltets_2_empty({"data-testid":`athena-run`}),});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":12,}});} running ? 'Running…' : editorSel.text.trim() ? 'Run selection' : 'Run';
       }
     }

     { svelteHTML.createElement("div", { "class":`editor`,});
       { const $$_rotidEedoC3C = __sveltets_2_ensureComponent(CodeEditor); new $$_rotidEedoC3C({ target: __sveltets_2_any(), props: {                    "path":`athena-${account.id}.sql`,"content":sql,"root":"","language":`sql`,"readOnly":false,"minimal":true,"completionSource":completionSource,"onchange":(v) => (sql = v),"onsubmit":() => void run(),"onselect":(s) => (editorSel = s),}});}
     }

    
     { svelteHTML.createElement("span", {   "class":`sr-only`,"role":`status`,});qstate ? `Query ${qstate.toLowerCase()}` : ''; }
     { svelteHTML.createElement("div", { "class":`status`,});
      if(qstate){
         { svelteHTML.createElement("span", { "class":`st ${qstate.toLowerCase()}`,});qstate; }
         { svelteHTML.createElement("span", { "class":`dim`,});  { svelteHTML.createElement("strong", {});fmtBytes(scanned); } }
         { svelteHTML.createElement("span", { "class":`dim`,});  { svelteHTML.createElement("strong", {});athenaCostUsd(scanned); }  { svelteHTML.createElement("span", { "title":`$5 per TB scanned, 10 MB minimum`,});  } }
        if(execMs){ { svelteHTML.createElement("span", { "class":`dim`,});fmtMs(execMs); }}
        if(qid){ { svelteHTML.createElement("button", {     "class":`link mono`,"onclick":() => qid && void copy(qid, 'execution id'),"title":`Copy execution id`,});qid.slice(0, 8);  }}
      }else{
         { svelteHTML.createElement("span", { "class":`dim`,});canRun ? 'Write a query and press ⌘↵.' : 'You can browse the catalog and history; running queries needs Edit on Athena.'; }
      }
       { svelteHTML.createElement("span", { "class":`spacer`,}); }
       { svelteHTML.createElement("div", { "class":`out-tabs`,});
         { const $$_sbaT4C = __sveltets_2_ensureComponent(Tabs); new $$_sbaT4C({ target: __sveltets_2_any(), props: {         "label":`Query output`,"tabs":[{ id: 'results', label: 'Results' }, { id: 'history', label: 'History' }],"value":tab,"onchange":(id) => {
            tab = id;
            if (id === 'history') void loadHistory();
          },}});}
       }
     }

     { svelteHTML.createElement("div", { "class":`results`,});
      if(tab === 'results'){
        if(!qstate && !result){
           { const $$_etatSytpmE3C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE3C({ target: __sveltets_2_any(), props: {      "icon":`db`,"title":`No results yet`,"body":`Run a query to see rows here.`,}});}
        }else{
           { const $$_dirGstluseR3C = __sveltets_2_ensureComponent(ResultsGrid); new $$_dirGstluseR3C({ target: __sveltets_2_any(), props: {           result,"error":resultError,"statement":ranSql,"connectionId":null,"running":running,"oncancel":() => void cancel(),}});}
        }
      } else if (historyLoading && history.length === 0){
         { svelteHTML.createElement("div", { "class":`pad`,}); { const $$_notelekS4C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS4C({ target: __sveltets_2_any(), props: {    "rows":6,"label":`query history`,}});} }
      } else if (historyError){
         { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {      "what":`query history`,"error":awsErrorText(historyError),"empty":true,"onretry":() => void loadHistory(),}});}
      } else if (history.length === 0){
         { const $$_etatSytpmE3C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE3C({ target: __sveltets_2_any(), props: {      "icon":`clock`,"title":`No recent executions`,"body":`Nothing has run in workgroup ${workgroup || '—'} lately.`,}});}
      }else{
         { svelteHTML.createElement("table", { "class":`hist`,});
           { svelteHTML.createElement("thead", {}); { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", { "class":`hide-sm`,});  } { svelteHTML.createElement("th", { "class":`num hide-sm`,});  } { svelteHTML.createElement("th", { "class":`num hide-sm`,});  } } }
           { svelteHTML.createElement("tbody", {});
               for(let x of __sveltets_2_ensureArray(history)){x.id;
               {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('tr')));{ svelteHTML.createElement("tr", __sveltets_2_union($$action_0), {          "class":`trow`,"tabindex":0,"onclick":() => openExecution(x),"onkeydown":(e) => { if (e.target === e.currentTarget && (e.key === 'Enter' || e.key === ' ')) { e.preventDefault(); openExecution(x); } },"oncontextmenu":(e) => ctxMenu.show(e, [
                { label: 'Load into editor', icon: 'edit', action: () => { sql = x.query; } },
                { label: 'Open result', icon: 'play', action: () => openExecution(x) },
                { label: 'Copy SQL', icon: 'copy', action: () => void copy(x.query, 'SQL') },
                { label: 'Copy execution id', icon: 'copy', action: () => void copy(x.id, 'id') },
              ]),});
                 { svelteHTML.createElement("td", {}); { svelteHTML.createElement("span", { "class":`st ${x.state.toLowerCase()}`,});x.state; } }
                 { svelteHTML.createElement("td", {   "class":`q mono`,"title":x.query,});x.query.replace(/\s+/g, ' ').slice(0, 160); }
                 { svelteHTML.createElement("td", { "class":`dim hide-sm`,});fmtAgo(x.submitted_at); }
                 { svelteHTML.createElement("td", { "class":`num mono hide-sm`,});fmtBytes(x.data_scanned_bytes); }
                 { svelteHTML.createElement("td", { "class":`num mono hide-sm`,});fmtMs(x.execution_ms); }
               }}
            }
           }
         }
      }
     }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AthenaView__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AthenaView__SvelteComponent_ = ReturnType<typeof AthenaView__SvelteComponent_>;
/*Ωignore_endΩ*/export default AthenaView__SvelteComponent_;