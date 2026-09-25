// Agent UI control — Database Explorer handlers (`otto.ui_db_*`, catalog:
// docs/contracts/ui-commands.json). Each handler drives the SAME store methods
// the Explorer's buttons call (lib/stores/database.svelte.ts), in the document
// the daemon routed the command to — normally the side pane next to the agent's
// session — so every step happens on screen: the connection opens, a tab with
// the agent's chip appears, the statement lands in the editor, the grid fills.
//
// Safety, on top of the daemon's grant + governance:
// - every agent run is READ-ONLY (`QueryRequest.read_only`). A write/DDL comes
//   back refused and the PERSON decides: the attributed confirm (with "Allow
//   writes on ‹conn› for this session") on an unguarded connection, the typed
//   confirm on a prod / read-only one. Declining → `cancelled_by_user`.
// - Explain runs read-only too (no `EXPLAIN ANALYZE <write>`).
// - Export never picks a file: the prefilled dialog waits for the person.
// - Rows handed back are capped (≤200 per call, long cells clipped); the daemon
//   redacts and caps again before anything reaches the agent's transcript.

import { registerUiCommands, registerUiState, UiCommandError, whenMounted, type UiCommandCtx } from '../uiCommands';
import { agentName } from '../stores/uiControl.svelte';
import {
  database,
  effectiveViewMode,
  type DbMainTab,
  type QueryTab,
  type RunOutcome,
  type ViewMode,
} from '../stores/database.svelte';
import { ws } from '../stores/workspace.svelte';
import { ui } from '../stores/ui.svelte';
import { router } from '../router.svelte';
import type { Connection, DbExportFormat, QueryResult, SchemaNode } from '../api/types';

/** Rows handed back per call (the catalog's documented cap). */
const ROWS_MAX = 200;
/** A single cell's text is clipped past this (a blob must not eat the payload). */
const CELL_MAX = 2000;

type Args = Record<string, unknown>;

// ─── Small helpers ──────────────────────────────────────────────────────────

const cancelled = (): UiCommandError => new UiCommandError('cancelled_by_user', 'Cancelled');

function str(args: Args, key: string): string | undefined {
  const v = args[key];
  if (v === undefined || v === null) return undefined;
  if (typeof v !== 'string') throw new UiCommandError('invalid_args', `\`${key}\` must be a string`);
  return v;
}

function int(args: Args, key: string): number | undefined {
  const v = args[key];
  if (v === undefined || v === null) return undefined;
  if (typeof v !== 'number' || !Number.isInteger(v)) {
    throw new UiCommandError('invalid_args', `\`${key}\` must be an integer`);
  }
  return v;
}

/** `tab_id` is a string on the wire (the Explorer's numeric tab id). */
function tabIdOf(args: Args, required = true): number | undefined {
  const raw = args.tab_id;
  if (raw === undefined || raw === null || raw === '') {
    if (required) throw new UiCommandError('invalid_args', 'Pass `tab_id` (from otto.ui_db_new_tab).');
    return undefined;
  }
  const n = typeof raw === 'number' ? raw : Number(String(raw).trim());
  if (!Number.isInteger(n) || n <= 0) throw new UiCommandError('invalid_args', `Unknown tab_id "${String(raw)}"`);
  return n;
}

/** Poll until `pred` yields a value (store state the page fills in async). */
function waitFor<T>(
  pred: () => T | null | undefined | false,
  signal: AbortSignal,
  timeoutMs: number,
  what: string,
): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const started = Date.now();
    const tick = (): void => {
      if (signal.aborted) return reject(cancelled());
      let v: T | null | undefined | false;
      try {
        v = pred();
      } catch (e) {
        return reject(e);
      }
      if (v !== null && v !== undefined && v !== false) return resolve(v);
      if (Date.now() - started > timeoutMs) {
        return reject(new UiCommandError('failed', `Timed out waiting for ${what}`));
      }
      setTimeout(tick, 50);
    };
    tick();
  });
}

function agentOf(ctx: UiCommandCtx): { session_id: string; label: string } {
  return { session_id: ctx.agent.session_id, label: agentName(ctx.agent) };
}

/** Clip a cell for the agent: long strings / big JSON become a marked prefix. */
function clipCell(v: unknown): unknown {
  if (typeof v === 'string') return v.length > CELL_MAX ? `${v.slice(0, CELL_MAX)}… [${v.length} chars]` : v;
  if (v && typeof v === 'object') {
    let text: string;
    try {
      text = JSON.stringify(v);
    } catch {
      return String(v);
    }
    return text.length > CELL_MAX ? `${text.slice(0, CELL_MAX)}… [${text.length} chars]` : v;
  }
  return v;
}

// ─── Getting the Explorer on screen ─────────────────────────────────────────

/**
 * Make sure this document shows the Explorer (the daemon routed us here
 * because it shows the Connections module — but `#/brokers` shares that key)
 * and that the connection library has loaded.
 */
async function ensureExplorer(ctx: UiCommandCtx): Promise<void> {
  if (router.module !== 'database' && router.module !== 'connections') {
    router.go('database');
  }
  await whenMounted('.db-root', ctx.signal, 15_000);
  if (!ws.currentId) throw new UiCommandError('failed', 'No workspace is open in this Otto window');
  if (!database.connections.length && !database.connectionsLoading) await database.loadConnections();
  await waitFor(() => !database.connectionsLoading, ctx.signal, 15_000, 'the connection list');
  if (database.connectionsError) {
    throw new UiCommandError('failed', `Could not load connections: ${database.connectionsError}`);
  }
}

/** A connection by id, else by (case-insensitive) name — the daemon already
 *  canonicalises ids, the name path covers a stale or hand-typed argument. */
function resolveConn(key: string | undefined): Connection {
  if (!key || !key.trim()) {
    throw new UiCommandError('invalid_args', 'Pass `connection_id` (id or name — otto.ui_db_list_connections).');
  }
  const k = key.trim();
  const byId = database.connections.find((c) => c.id === k);
  if (byId) return byId;
  const lower = k.toLowerCase();
  const byName = database.connections.filter((c) => c.name.toLowerCase() === lower);
  if (byName.length === 1) return byName[0]!;
  if (byName.length > 1) {
    throw new UiCommandError('invalid_args', `More than one connection is named "${k}" — pass its id.`);
  }
  if (database.otherConnections.some((c) => c.id === k || c.name.toLowerCase() === lower)) {
    throw new UiCommandError('invalid_args', `"${k}" isn't a database connection (SSH / custom profiles open as terminals).`);
  }
  throw new UiCommandError('not_found', `No database connection "${k}" in this workspace`);
}

/** Open (or focus) a connection and wait until it's connected. */
async function openConn(c: Connection, ctx: UiCommandCtx, dbName?: string): Promise<void> {
  ctx.progress(`Opening ${c.name}`);
  await database.openConnection(c.id);
  const st = await waitFor(
    () => {
      const s = database.connStatus.get(c.id);
      return s && s.phase !== 'connecting' ? s : null;
    },
    ctx.signal,
    20_000,
    `${c.name} to connect`,
  );
  if (st.phase === 'error') {
    throw new UiCommandError('failed', `Could not connect to ${c.name}: ${st.error ?? 'unknown error'}`);
  }
  if (dbName !== undefined && dbName.trim()) database.setActiveDb(dbName.trim());
}

/** Focus a tab by id (its connection, the tab, the Query view) or fail. */
async function focusTab(tabId: number, ctx: UiCommandCtx): Promise<QueryTab> {
  const t = await database.focusTab(tabId);
  if (!t) {
    throw new UiCommandError(
      'not_found',
      `No open query tab ${tabId} (it was closed, or the window reloaded) — open one with otto.ui_db_new_tab.`,
    );
  }
  if (ctx.signal.aborted) throw cancelled();
  await whenMounted(`[data-db-tab-id="${tabId}"]`, ctx.signal).catch(() => null);
  return t;
}

// ─── Result summaries ───────────────────────────────────────────────────────

/** A page of a tab's result, the shape `db_run_query` / `db_get_result` /
 *  `db_page` return (see the catalog descriptions). */
function summarize(
  t: QueryTab,
  connId: string,
  opts: { offset?: number; limit?: number; columns?: string[] } = {},
): Record<string, unknown> {
  const conn = database.connections.find((c) => c.id === connId);
  const base: Record<string, unknown> = {
    tab_id: String(t.id),
    connection_id: connId,
    connection: conn?.name ?? null,
    statement: t.ran_statement ?? t.statement,
    running: t.running,
  };
  if (t.error) base.error = t.error;
  const r: QueryResult | null = t.result;
  if (!r) return { ...base, columns: [], rows: [], loaded_rows: 0, has_result: false };

  const names = r.columns.map((c) => c.name);
  let pick = names.map((_, i) => i);
  if (opts.columns?.length) {
    const want = new Set(opts.columns.map((c) => c.toLowerCase()));
    pick = pick.filter((i) => want.has(names[i]!.toLowerCase()));
    if (!pick.length) {
      throw new UiCommandError('invalid_args', `None of those columns are in the result (${names.join(', ')})`);
    }
  }
  const from = Math.max(0, opts.offset ?? 0);
  const limit = Math.min(ROWS_MAX, Math.max(1, opts.limit ?? ROWS_MAX));
  const slice = r.rows.slice(from, from + limit);
  const pageSize = r.auto_limited ?? null;
  const out: Record<string, unknown> = {
    ...base,
    has_result: true,
    columns: pick.map((i) => ({ name: names[i], type: r.columns[i]?.type_hint ?? null })),
    rows: slice.map((row) => pick.map((i) => clipCell(row[i]))),
    // Rows the grid holds for this page / how many came back with this call.
    loaded_rows: r.rows.length,
    returned_rows: slice.length,
    row_offset: from,
    truncated: from + slice.length < r.rows.length || r.truncated === true,
    // The grid's own pager (a single auto-limited SELECT / find).
    auto_limited: pageSize,
    page_offset: t.offset,
    page: pageSize ? Math.floor(t.offset / pageSize) + 1 : null,
    has_next: !!pageSize && r.rows.length >= pageSize,
    has_prev: !!pageSize && t.offset > 0,
    elapsed_ms: r.stats?.duration_ms ?? null,
    affected: r.rows_affected ?? null,
    masked: r.masked === true,
    // The view the grid renders (the same inputs QueryEditor resolves).
    view: effectiveViewMode({
      tabPick: t.viewMode ?? null,
      connPick: database.connView,
      columnCount: r.columns.length,
      autoVerticalCols: ui.dbAutoVerticalFor(database.capabilities?.engine),
      engine: database.capabilities?.engine ?? null,
    }),
  };
  if (r.message) out.message = r.message;
  if (r.more_results?.length) {
    out.more_results = r.more_results.map((m) => ({
      statement: m.statement ?? null,
      columns: m.columns.length,
      rows: m.rows.length,
      affected: m.rows_affected ?? null,
      errored: m.errored === true,
      message: m.message ?? null,
    }));
  }
  return out;
}

/** Map a run's outcome to the agent's result (or a coded refusal). */
function afterRun(t: QueryTab, connId: string, outcome: RunOutcome, ctx: UiCommandCtx): Record<string, unknown> {
  switch (outcome.status) {
    case 'ok':
      ctx.highlight('.qe-results');
      return summarize(t, connId);
    case 'cancelled':
      throw new UiCommandError('cancelled_by_user', 'The user declined the write — nothing was changed.');
    case 'aborted':
      throw new UiCommandError('cancelled_by_user', 'The query was stopped before it finished.');
    case 'invalid':
      throw new UiCommandError('invalid_args', outcome.error ?? 'The statement is empty');
    case 'failed':
      // The engine answered with an error: that's a result the agent needs to
      // see (the tab shows the same error inline).
      ctx.highlight('.qe-results');
      throw new UiCommandError('failed', outcome.error ?? 'The query failed');
    case 'detached':
      throw new UiCommandError(
        'failed',
        'Lost the wait for this query; it may still finish in the tab — read it with otto.ui_db_get_result.',
      );
    default:
      throw new UiCommandError('failed', 'The query did not run');
  }
}

/** Run the ACTIVE tab (already focused) read-only, the person deciding on writes. */
async function runActive(
  ctx: UiCommandCtx,
  conn: Connection,
  run: (opts: Parameters<typeof database.runQuery>[2]) => Promise<unknown>,
): Promise<{ t: QueryTab; outcome: RunOutcome }> {
  const t = database.tab;
  const outcome: RunOutcome = {};
  // Stop / deadline → stop the query too, server-side (not just our wait).
  const onAbort = (): void => database.abortQuery(t.id);
  ctx.signal.addEventListener('abort', onAbort, { once: true });
  try {
    ctx.progress(`Running on ${conn.name}`);
    await run({
      readOnly: true,
      agentLabel: agentName(ctx.agent),
      outcome,
      awaitingHuman: (note) => ctx.progress(note, true),
      confirmWrite: () =>
        ctx.confirmWrite({
          what: (t.statement || '').slice(0, 1200),
          where: conn.name,
          connId: conn.id,
          verb: 'Run',
        }),
    });
  } finally {
    ctx.signal.removeEventListener('abort', onAbort);
  }
  if (ctx.signal.aborted) throw cancelled();
  return { t, outcome };
}

/** Walk a schema path of labels (`["shop", "orders"]`) to its tree node,
 *  expanding as it goes (visibly — the tree opens like a person's clicks).
 *  Folder nodes ("Tables", "Views") in between are looked through. */
async function resolvePath(path: string[], ctx: UiCommandCtx): Promise<SchemaNode> {
  await waitFor(() => !database.schemaLoading && database.schemaRoot.length > 0, ctx.signal, 20_000, 'the schema tree');
  let level: SchemaNode[] = database.schemaRoot;
  let found: SchemaNode | null = null;
  const childrenOf = async (n: SchemaNode): Promise<SchemaNode[]> => {
    if (!n.has_children) return [];
    if (!database.isExpanded(n.id)) await database.expand(n);
    return waitFor(() => database.childrenOf(n.id), ctx.signal, 20_000, `the children of ${n.label}`);
  };
  for (const [depth, raw] of path.entries()) {
    const want = raw.trim().toLowerCase();
    const match = (nodes: SchemaNode[]): SchemaNode | undefined =>
      nodes.find((n) => n.label.toLowerCase() === want) ?? nodes.find((n) => n.id.toLowerCase() === want);
    let hit = match(level);
    if (!hit) {
      for (const folder of level.filter((n) => n.kind === 'folder')) {
        hit = match(await childrenOf(folder));
        if (hit) break;
      }
    }
    if (!hit) {
      const names = level.map((n) => n.label).slice(0, 40).join(', ');
      throw new UiCommandError('not_found', `No "${raw}" at ${path.slice(0, depth).join(' / ') || 'the top'} (have: ${names})`);
    }
    found = hit;
    if (depth < path.length - 1) level = await childrenOf(hit);
  }
  return found!;
}

// ─── Handlers ───────────────────────────────────────────────────────────────

registerUiCommands('connections', {
  async db_list_connections(args: Args, ctx) {
    await ensureExplorer(ctx);
    const q = str(args, 'query')?.trim().toLowerCase();
    const list = database.connections
      .filter((c) => !q || c.name.toLowerCase().includes(q) || c.kind.toLowerCase().includes(q))
      .map((c) => ({
        id: c.id,
        name: c.name,
        kind: c.kind,
        environment: c.environment ?? null,
        read_only: c.read_only === true,
        guarded: c.environment === 'prod' || c.read_only === true,
        open: database.openConnIds.includes(c.id),
        selected: database.selectedConnId === c.id,
      }));
    return { connections: list, workspace_id: ws.currentId };
  },

  async db_open_connection(args: Args, ctx) {
    await ensureExplorer(ctx);
    const c = resolveConn(str(args, 'connection_id'));
    await openConn(c, ctx, str(args, 'database'));
    ctx.highlight('.query-editor, .db-root');
    return {
      connection_id: c.id,
      name: c.name,
      kind: c.kind,
      environment: c.environment ?? null,
      guarded: database.isGuarded,
      databases: database.isRedis ? database.keyspaces.map((k) => k.label) : database.databaseNames,
      active_database: database.activeDb,
    };
  },

  async db_new_tab(args: Args, ctx) {
    await ensureExplorer(ctx);
    const c = resolveConn(str(args, 'connection_id'));
    await openConn(c, ctx, str(args, 'database'));
    const t = database.newAgentTab(str(args, 'statement') ?? '', agentOf(ctx));
    const el = await whenMounted(`[data-db-tab-id="${t.id}"]`, ctx.signal).catch(() => null);
    ctx.highlight(el);
    return { tab_id: String(t.id), connection_id: c.id, statement: t.statement, active_database: database.activeDb };
  },

  async db_set_statement(args: Args, ctx) {
    await ensureExplorer(ctx);
    const tabId = tabIdOf(args)!;
    const statement = str(args, 'statement');
    if (statement === undefined) throw new UiCommandError('invalid_args', 'Pass `statement`.');
    await focusTab(tabId, ctx);
    database.setStatement(statement);
    ctx.highlight(await whenMounted('.qe-edit', ctx.signal).catch(() => null));
    return { tab_id: String(tabId), statement };
  },

  async db_run_query(args: Args, ctx) {
    await ensureExplorer(ctx);
    const tabId = tabIdOf(args, false);
    const statement = str(args, 'statement');
    const dbName = str(args, 'database');
    const rowLimit = int(args, 'row_limit');
    const timeoutMs = int(args, 'timeout_ms');
    let connId: string;
    if (tabId !== undefined) {
      await focusTab(tabId, ctx);
      connId = database.selectedConnId!;
      if (dbName) database.setActiveDb(dbName);
      if (statement !== undefined) database.setStatement(statement);
    } else {
      if (statement === undefined || !statement.trim()) {
        throw new UiCommandError('invalid_args', 'Pass `tab_id`, or `connection_id` + `statement` to run in a new tab.');
      }
      const c = resolveConn(str(args, 'connection_id'));
      await openConn(c, ctx, dbName);
      database.newAgentTab(statement, agentOf(ctx));
      connId = c.id;
    }
    const conn = database.connections.find((c) => c.id === connId)!;
    const tab = database.tab;
    if (!tab.statement.trim()) throw new UiCommandError('invalid_args', `Tab ${tab.id} has no statement to run.`);
    // The per-tab statement timeout is a visible tab setting — the agent's
    // value shows there like the person's would.
    if (timeoutMs !== undefined) tab.timeout_ms = timeoutMs;
    ctx.highlight(await whenMounted('.qe-edit', ctx.signal).catch(() => null));
    const { t, outcome } = await runActive(ctx, conn, (opts) =>
      database.runQuery(undefined, undefined, { ...opts, ...(rowLimit ? { maxRows: rowLimit } : {}) }),
    );
    return afterRun(t, connId, outcome, ctx);
  },

  async db_get_result(args: Args, ctx) {
    await ensureExplorer(ctx);
    const tabId = tabIdOf(args)!;
    const t = await focusTab(tabId, ctx);
    const columns = Array.isArray(args.columns) ? args.columns.filter((c): c is string => typeof c === 'string') : undefined;
    const out = summarize(t, database.selectedConnId!, {
      offset: int(args, 'offset'),
      limit: int(args, 'limit'),
      columns,
    });
    if (t.result) ctx.highlight('.qe-results');
    return out;
  },

  async db_page(args: Args, ctx) {
    await ensureExplorer(ctx);
    const tabId = tabIdOf(args)!;
    const delta = int(args, 'delta');
    if (delta !== 1 && delta !== -1) throw new UiCommandError('invalid_args', '`delta` must be 1 or -1');
    const t = await focusTab(tabId, ctx);
    const pageSize = t.result?.auto_limited ?? 0;
    if (!t.result) throw new UiCommandError('invalid_args', `Tab ${tabId} has no result to page — run it first.`);
    if (pageSize <= 0) {
      throw new UiCommandError('invalid_args', 'This result is not paged (it has its own LIMIT, or is not a single SELECT/find).');
    }
    if (delta === 1 && t.result.rows.length < pageSize) throw new UiCommandError('invalid_args', 'Already on the last page.');
    if (delta === -1 && t.offset <= 0) throw new UiCommandError('invalid_args', 'Already on the first page.');
    const conn = database.connections.find((c) => c.id === database.selectedConnId)!;
    const { outcome } = await runActive(ctx, conn, (opts) => database.runPage(delta, opts));
    return afterRun(t, conn.id, outcome, ctx);
  },

  async db_set_view(args: Args, ctx) {
    await ensureExplorer(ctx);
    const tabId = tabIdOf(args, false);
    const view = str(args, 'view') as ViewMode | undefined;
    const main = str(args, 'main_tab') as DbMainTab | undefined;
    if (!view && !main) throw new UiCommandError('invalid_args', 'Pass `view` and/or `main_tab`.');
    if (view && !['grid', 'vertical', 'json'].includes(view)) throw new UiCommandError('invalid_args', `Unknown view "${view}"`);
    if (main && !['query', 'structure', 'diagram', 'builder'].includes(main)) {
      throw new UiCommandError('invalid_args', `Unknown main_tab "${main}"`);
    }
    if (tabId !== undefined) await focusTab(tabId, ctx);
    else if (!database.selectedConnId) {
      throw new UiCommandError('invalid_args', 'No connection is open — pass `tab_id` (or open one first).');
    }
    if (view) {
      database.setViewMode(view);
      if (database.mainTab !== 'query') database.setMainTab('query');
    }
    if (main) {
      if (main === 'builder' && !database.supportsBuilder) {
        throw new UiCommandError('invalid_args', 'The visual builder is not available for this engine.');
      }
      database.setMainTab(main);
    }
    ctx.highlight(view ? '.qe-results' : '.db-root');
    return { tab_id: String(database.tab.id), view: database.tab.viewMode ?? null, main_tab: database.mainTab };
  },

  async db_open_object(args: Args, ctx) {
    await ensureExplorer(ctx);
    const c = resolveConn(str(args, 'connection_id'));
    const path = Array.isArray(args.path) ? args.path.filter((p): p is string => typeof p === 'string' && !!p.trim()) : [];
    if (!path.length) throw new UiCommandError('invalid_args', 'Pass `path`, e.g. ["shop", "orders"].');
    await openConn(c, ctx);
    const node = await resolvePath(path, ctx);
    await database.openObject(node);
    await waitFor(() => !database.objectLoading, ctx.signal, 20_000, `${node.label} to load`);
    if (database.objectError) throw new UiCommandError('failed', database.objectError);
    const d = database.objectDetail;
    ctx.highlight('.db-root');
    return {
      connection_id: c.id,
      path: node.id,
      name: d?.name ?? node.label,
      kind: d?.kind ?? node.kind,
      columns: (d?.columns ?? []).slice(0, ROWS_MAX).map((col) => ({
        name: col.name,
        type: col.data_type,
        nullable: col.nullable,
        key: col.key ?? null,
      })),
      primary_key: d?.primary_key ?? [],
      indexes: (d?.indexes ?? []).map((i) => ({ name: i.name, columns: i.columns, unique: i.unique })),
      foreign_keys: (d?.foreign_keys ?? []).map((f) => ({
        name: f.name,
        columns: f.columns,
        ref_table: f.ref_table,
        ref_columns: f.ref_columns,
      })),
      row_count: d?.row_count ?? null,
    };
  },

  async db_explain(args: Args, ctx) {
    await ensureExplorer(ctx);
    const tabId = tabIdOf(args)!;
    const t = await focusTab(tabId, ctx);
    const statement = str(args, 'statement');
    if (statement !== undefined) database.setStatement(statement);
    if (!t.statement.trim()) throw new UiCommandError('invalid_args', `Tab ${tabId} has no statement to explain.`);
    ctx.progress('Explaining');
    const onAbort = (): void => database.abortQuery(t.id);
    ctx.signal.addEventListener('abort', onAbort, { once: true });
    try {
      await database.explainPlan({ readOnly: true });
    } finally {
      ctx.signal.removeEventListener('abort', onAbort);
    }
    if (ctx.signal.aborted) throw cancelled();
    if (database.planOpen && database.queryPlan) {
      ctx.highlight('.db-root');
      return { tab_id: String(t.id), statement: t.statement, plan: database.queryPlan.root, engine: database.queryPlan.engine };
    }
    // The raw EXPLAIN fell back into the grid.
    if (t.error) throw new UiCommandError('failed', t.error);
    ctx.highlight('.qe-results');
    return { tab_id: String(t.id), statement: t.statement, raw: summarize(t, database.selectedConnId!) };
  },

  async db_stop(args: Args, ctx) {
    await ensureExplorer(ctx);
    const tabId = tabIdOf(args)!;
    const at = database.locateTab(tabId);
    if (!at) throw new UiCommandError('not_found', `No open query tab ${tabId}`);
    if (!at.tab.running && !at.tab.pending) return { tab_id: String(tabId), stopped: false, running: false };
    await focusTab(tabId, ctx);
    database.abortQuery(tabId, { report: true });
    ctx.highlight('.qe-results');
    return { tab_id: String(tabId), stopped: true };
  },

  async db_export(args: Args, ctx) {
    await ensureExplorer(ctx);
    const tabId = tabIdOf(args)!;
    const t = await focusTab(tabId, ctx);
    const statement = t.ran_statement;
    if (!statement || !t.result) throw new UiCommandError('invalid_args', `Tab ${tabId} has no result to export — run it first.`);
    const format = str(args, 'format') as DbExportFormat | undefined;
    const maxRows = int(args, 'max_rows');
    if (database.exportRequest) throw new UiCommandError('failed', 'An export dialog is already open.');
    ctx.progress('Waiting for you to choose where to save the export', true);
    const done = new Promise<{ exported: boolean; path?: string; rows?: number; bytes?: number }>((resolve) => {
      database.exportRequest = {
        connId: database.selectedConnId!,
        statement,
        node: t.ran_node,
        format,
        maxRows,
        agentLabel: agentName(ctx.agent),
        done: resolve,
      };
    });
    const req = database.exportRequest;
    const onAbort = (): void => {
      if (database.exportRequest === req) database.exportRequest = null;
    };
    ctx.signal.addEventListener('abort', onAbort, { once: true });
    try {
      const r = await done;
      if (!r.exported) throw new UiCommandError('cancelled_by_user', 'The user closed the export dialog without exporting.');
      return { exported: true, path: r.path, rows: r.rows, bytes: r.bytes };
    } finally {
      ctx.signal.removeEventListener('abort', onAbort);
      if (database.exportRequest === req) database.exportRequest = null;
    }
  },
});

// What `otto.ui_state` reports for a document showing the Explorer: which
// connection / tab is up and what its grid holds — never row data.
registerUiState('connections', () => {
  const t = database.selectedConnId ? database.tab : null;
  return {
    selected_connection: database.selectedConn ? { id: database.selectedConn.id, name: database.selectedConn.name } : null,
    open_connections: database.openConnIds,
    active_database: database.activeDb,
    main_tab: database.mainTab,
    tabs: database.selectedConnId
      ? database.tabs.map((x) => ({
          tab_id: String(x.id),
          statement: x.statement.slice(0, 200),
          running: x.running,
          has_result: !!x.result,
          by_agent: x.agent?.label ?? null,
        }))
      : [],
    active_tab: t ? String(t.id) : null,
    result: t?.result ? { rows: t.result.rows.length, columns: t.result.columns.length } : null,
  };
});
