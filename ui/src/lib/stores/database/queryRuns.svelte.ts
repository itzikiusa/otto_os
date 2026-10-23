// Database Explorer store — query runs: run (with the guarded-write
// confirmation), paging, managed statements, Explain / query plans, cancel
// (client abort + server-side cancel), re-attaching to a run whose HTTP wait was
// lost, and the per-tab result memory budget. The DatabaseStore façade owns one
// instance as `runs` and delegates its public methods here.

import { api, ApiError, isAbortError, withDeadline, READ_DEADLINE_MS } from '../../api/client';
import { confirmer } from '../../confirm.svelte';
import type {
  Connection,
  DbCancelOutcome,
  DbQueryPlan,
  DbQueryStatus,
  Id,
  QueryResult,
  RunQueryReq,
} from '../../api/types';
import { toasts } from '../../toast.svelte';
import { router } from '../../router.svelte';
import { normalizeDbError } from '../../database/error-normalize';
import { parkedEditCount, releaseGridResult } from '../../database/grid-tab-state';
import { estimateResultBytes, isReleased, releasedStub, resultBudget } from '../db-result-budget';
import { loadErrorText } from '../../loadError';
import { toastError } from '../../toastError';
import { isReadOnlyRefusal, isWriteBlocked, newQueryId, rawResult } from './model';
import type {
  GuardedWriteAnswer,
  ManagedWriteCanceled,
  PendingRun,
  QueryTab,
  RunOutcome,
} from './model';
import { parseExplicitLimit } from './statement';
import type { DatabaseStore } from '../database.svelte';

/** The DatabaseStore members this part reads and calls (its whole contract with the façade). */
export type QueryRunsHost = Pick<
  DatabaseStore,
  | 'accessEpoch'
  | 'activeDb'
  | 'activePane'
  | 'activeTab'
  | 'capabilities'
  | 'conn'
  | 'connBase'
  | 'epochs'
  | 'loadHistory'
  | 'locateTab'
  | 'persist'
  | 'planOpen'
  | 'queryPlan'
  | 'rowLimit'
  | 'selectedConn'
  | 'selectedConnId'
  | 'tab'
  | 'tabByUid'
  | 'tabs'
>;

export class DbQueryRunsStore {
  private readonly db: QueryRunsHost;
  constructor(db: QueryRunsHost) {
    this.db = db;
  }

  /**
   * In-flight runs, keyed by tab id. Non-reactive (plain Map): lets us cancel a
   * running query (`abortQuery`) without storing a non-serializable controller
   * inside the reactive `$state` tab objects. Each entry carries the fetch
   * `AbortController` (to drop the HTTP wait), plus the `queryId` + `connId` the
   * run was issued with — so `abortQuery` can also tell the SERVER to cancel the
   * query engine-side (`POST …/db/cancel`), not just abandon the response.
   */
  runControllers = new Map<
    number,
    { controller: AbortController; queryId: string; connId: Id }
  >();
  /** Tab ids with an active `reattach` poll loop — dedupes concurrent kicks. */
  private reattaching = new Set<number>();

  // ── Result memory budget (lib/stores/db-result-budget.ts) ─────────────────
  /**
   * Land a result on a tab and account for it: every resident result across
   * the live tabs AND every parked connection snapshot shares one budget; past
   * it the least-recently-viewed results are released (rows dropped, columns
   * and stats kept) and the grid offers a Re-run.
   */
  private installResult(t: QueryTab, result: QueryResult): void {
    t.result = rawResult(result);
    resultBudget.note(t.uid, estimateResultBytes(result));
    this.enforceResultBudget();
  }

  /** Release least-recently-viewed results until the total fits the budget. */
  enforceResultBudget(): void {
    const pinned = new Set<string>();
    const shown = this.db.tabs[this.db.activeTab];
    if (shown) pinned.add(shown.uid);
    const consider = (tabs: QueryTab[]) => {
      for (const t of tabs) {
        // Never under a running query or un-applied cell edits.
        if (t.running || t.pending || parkedEditCount(t.uid) > 0) pinned.add(t.uid);
      }
    };
    consider(this.db.tabs);
    for (const snap of this.db.conn.snapshots.values()) consider(snap.tabs);
    for (const uid of resultBudget.pick(pinned)) {
      resultBudget.forget(uid);
      const t = this.db.tabByUid(uid);
      if (!t?.result || isReleased(t.result)) continue;
      t.result = rawResult(releasedStub(t.result));
      releaseGridResult(uid);
    }
  }

  /**
   * Re-run the statement that produced the active tab's RELEASED result (same
   * statement, scope, offset and page size) — the grid's "Re-run" button.
   */
  rerunReleased(): Promise<QueryResult | null> {
    const t = this.db.tab;
    if (!t?.result || !isReleased(t.result) || !t.ran_statement) return Promise.resolve(null);
    return this.runQuery(t.ran_statement, t.ran_node, {
      keepOffset: true,
      transient: true,
      ...(t.result.auto_limited ? { maxRows: t.result.auto_limited } : {}),
    });
  }

  // ── Query ─────────────────────────────────────────────────────────────────

  /** True when `t` (a tab of connection `connId`) is the query tab on screen:
   *  the Database page is open, on that connection's DB workbench, with `t`
   *  as its active tab. */
  private isVisibleTab(connId: Id, t: QueryTab): boolean {
    return (
      (router.module === 'database' || router.module === 'connections') &&
      this.db.selectedConnId === connId &&
      this.db.activePane === null &&
      this.db.tab?.id === t.id
    );
  }

  /** Run the active tab's statement (or a given one) and store the result.
   *  `node` scopes it: omitted (`undefined`) → the active database; `null` →
   *  explicitly NO scope (how a result that ran unscoped is paged/refreshed —
   *  it must not pick up a database selected since). */
  async runQuery(
    statement?: string,
    node?: string | null,
    opts?: {
      transient?: boolean;
      keepOffset?: boolean;
      cursor?: unknown;
      /**
       * Run READ-ONLY (`QueryRequest.read_only`): the server refuses a write/DDL
       * before touching the driver. What an agent's UI-control run always sends —
       * the human's token must never quietly upgrade the agent to writes.
       */
      readOnly?: boolean;
      /**
       * Asked when a `readOnly` run was refused as a write on an UNGUARDED
       * connection (the attributed "‹agent› wants to write" confirm). True
       * re-runs it normally. A guarded (prod / read-only) connection skips this
       * and goes straight to the typed confirm, attributed with `agentLabel`.
       */
      confirmWrite?: () => Promise<boolean>;
      /** Who asked, for the typed guarded-write confirm ("Claude · fix-orders"). */
      agentLabel?: string;
      /** Filled with how the run ended (see {@link RunOutcome}). */
      outcome?: RunOutcome;
      /** Called right before a confirm waits on the person (an agent command
       *  extends its deadline to the human-wait cap). */
      awaitingHuman?: (note: string) => void;
      /** Row cap for THIS run instead of the persisted default (`rowLimit`) —
       *  an agent's `row_limit` must not rewrite the person's preference. */
      maxRows?: number;
    },
  ): Promise<QueryResult | null> {
    const id = this.db.selectedConnId;
    const origin = this.db.selectedConn ? { ...this.db.selectedConn } : null;
    const t = this.db.tab;
    const outcome = opts?.outcome;
    if (!id) {
      toasts.error('No connection selected');
      if (outcome) Object.assign(outcome, { status: 'invalid', error: 'no connection selected' });
      return null;
    }
    const sql = (statement ?? t.statement).trim();
    if (!sql) {
      toasts.error('Statement is empty');
      if (outcome) Object.assign(outcome, { status: 'invalid', error: 'statement is empty' });
      return null;
    }
    // A transient run (the selected / current statement, variable-substituted)
    // must NOT clobber the editor's full multi-statement buffer.
    if (statement !== undefined && !opts?.transient) t.statement = statement;
    // A fresh run starts at the first page; only the pager (runPage) keeps offset.
    if (!opts?.keepOffset) t.offset = 0;
    // Cancel any prior in-flight run for this tab before starting a new one
    // (server-side too, so a previous heavy query stops on the DB).
    this.abortQuery(t.id);
    const controller = new AbortController();
    // Per-run id the server registers the query under; the same id lets the
    // cancel endpoint issue engine-native cancellation (KILL QUERY / etc.) and
    // the query-status endpoint re-attach to a run whose HTTP wait was lost.
    const queryId = newQueryId();
    const accessEpoch=this.db.accessEpoch;
    // The engine this run belongs to — a failure may land after the person has
    // switched connection, and its background toast must read it as THIS engine.
    const runEngine = this.db.capabilities?.engine ?? null;
    this.runControllers.set(t.id, { controller, queryId, connId: id });
    // Scope to the active database (so unqualified tables resolve) unless a
    // node was passed — `null` included, which means "no scope".
    const scopeNode = node === undefined ? this.db.activeDb || null : node;
    t.running = true;
    t.error = null;
    t.err_statement = null;
    t.pending = { queryId, connId: id, sql, node: scopeNode, startedAt: Date.now() };
    this.db.persist.persistTabs();
    // Stop clears ownership immediately; its HTTP wait stays alive until the
    // native cancel responds, so the signal alone no longer identifies a live run.
    const stopped = () => accessEpoch !== this.db.accessEpoch || controller.signal.aborted
      || this.runControllers.get(t.id)?.controller !== controller || t.pending?.queryId !== queryId;
    try {
      // Honor an explicit LIMIT in the SQL; otherwise apply the configured
      // default row cap. The server also injects this LIMIT into the SQL so a
      // huge table isn't fully scanned — this value just sizes that cap.
      const explicit = parseExplicitLimit(sql);
      // Per-tab timeout (opt-in; null / 0 = no limit).
      const tabTimeoutMs = this.db.tab?.timeout_ms ?? null;

      const tabMask = this.db.tab?.mask ?? false;
      // Dropped (below) once the human approves the write the read-only run
      // refused — the re-run is then an ordinary run under their own confirm.
      let readOnly = opts?.readOnly === true;
      const post = (confirmWrite: boolean): Promise<QueryResult> => {
        const body: RunQueryReq = {
          statement: sql,
          max_rows: explicit ?? opts?.maxRows ?? this.db.rowLimit,
          node: scopeNode,
          confirm_write: confirmWrite,
          // Per-run id so the cancel endpoint can issue engine-native
          // cancellation (KILL QUERY / etc.) for this in-flight query.
          query_id: queryId,
          // Footer pager: server appends OFFSET (Mongo: skip) when auto-limiting.
          ...(t.offset > 0 ? { offset: t.offset } : {}),
          // Keyset "Next" (Mongo): the previous page's `next_cursor`, echoed back
          // so the server pages by `_id > cursor` instead of `skip`. Sent next to
          // `offset` — the server ignores `skip` when the cursor applies and falls
          // back to it when the find isn't keyset-eligible.
          ...(opts?.cursor !== undefined ? { cursor: opts.cursor } : {}),
          // Driver-enforced timeout (engine-native, e.g. MySQL MAX_EXECUTION_TIME).
          ...(tabTimeoutMs && tabTimeoutMs > 0 ? { timeout_ms: tabTimeoutMs } : {}),
          // Server-side PII/prod masking: redacts cell values before they leave
          // the server. Only sent when the toggle is explicitly on.
          ...(tabMask ? { mask: true } : {}),
          ...(readOnly ? { read_only: true } : {}),
        };
        return api.post<QueryResult>(`${this.db.connBase(id)}/query`, body, controller.signal);
      };

      let result: QueryResult;
      try {
        result = await post(false);
      } catch (e) {
        if (stopped()) throw e;
        if (readOnly && isReadOnlyRefusal(e)) {
          // An agent's read-only run hit a write/DDL: the human decides. A
          // guarded connection always takes the typed confirm (and re-runs with
          // the explicit confirm flag); an unguarded one takes the caller's
          // attributed confirm, then re-runs as an ordinary run.
          const guarded = origin?.environment === 'prod' || origin?.read_only === true;
          opts?.awaitingHuman?.(guarded ? 'Waiting for the typed write confirm' : 'Waiting for you to allow the write');
          const answer: GuardedWriteAnswer = guarded
            ? await this.confirmGuardedWrite(origin, e, opts?.agentLabel)
            : (await (opts?.confirmWrite?.() ?? Promise.resolve(false)))
              ? 'confirmed'
              : 'declined';
          if (answer !== 'confirmed' || stopped()) {
            // A mistyped name was already explained by the gate.
            if (answer !== 'mismatch') toasts.info('Write canceled');
            if (t.pending?.queryId === queryId) this.clearPending(t);
            if (outcome) Object.assign(outcome, { status: 'cancelled', error: 'the user declined the write' });
            return null;
          }
          readOnly = false;
          result = await post(guarded);
        } else if (isWriteBlocked(e)) {
          // Production / read-only guardrail: the server refused a write/DDL on a
          // guarded connection. Ask for a typed confirmation and, if granted,
          // retry with the explicit confirm flag.
          opts?.awaitingHuman?.('Waiting for the typed write confirm');
          const answer = await this.confirmGuardedWrite(origin, e, opts?.agentLabel);
          if (answer !== 'confirmed' || stopped()) {
            if (answer !== 'mismatch') toasts.info('Write canceled');
            if (t.pending?.queryId === queryId) this.clearPending(t);
            if (outcome) Object.assign(outcome, { status: 'cancelled', error: 'the user declined the write' });
            return null;
          }
          result = await post(true);
        } else {
          throw e;
        }
      }
      if (stopped()) {
        if (outcome) Object.assign(outcome, { status: 'aborted', error: 'the run was stopped or superseded' });
        return null;
      }
      this.installResult(t, result);
      t.ran_statement = sql;
      t.ran_node = scopeNode;
      t.err_statement = null;
      this.clearPending(t);
      void this.db.loadHistory(id);
      if (outcome) outcome.status = 'ok';
      return result;
    } catch (e) {
      // A user-initiated abort isn't an error — leave the prior result intact.
      // What the Stop actually achieved is reported by `abortQuery` from the
      // server's cancel outcome — this only drops our wait, so it must not
      // claim the query stopped.
      if (isAbortError(e) || stopped()) {
        if (outcome) Object.assign(outcome, { status: 'aborted', error: 'the query was stopped' });
        return null;
      }
      // The server answered → the query itself finished with an error.
      if (e instanceof ApiError) {
        if (outcome) Object.assign(outcome, { status: 'failed', error: loadErrorText(e) });
        this.clearPending(t);
        t.error = loadErrorText(e);
        // What failed — ErrorPanel's caret / "Ask AI to fix" describe THIS, not
        // the (possibly much larger, since edited) editor buffer.
        t.err_statement = sql;
        // The tab's inline ErrorPanel already says this when it's on screen —
        // only toast when the failing tab is NOT the one the person is looking at.
        // Headline only (the normalised title) — the full error, cause and
        // hint wait in the tab's ErrorPanel.
        if (!this.isVisibleTab(id, t)) {
          toasts.error('Couldn’t query', normalizeDbError(runEngine, loadErrorText(e), sql).title);
        }
        return null;
      }
      // The HTTP wait was lost (page teardown / network blip) but the server
      // keeps executing the query detached — re-attach by query_id instead of
      // declaring failure.
      if (outcome) Object.assign(outcome, { status: 'detached', error: loadErrorText(e) });
      void this.reattach(t);
      return null;
    } finally {
      // Only clear running/controller if this run is still the current one
      // (a newer run may have replaced it).
      if (this.runControllers.get(t.id)?.controller === controller) {
        this.runControllers.delete(t.id);
        t.running = false;
      }
    }
  }

  /**
   * Page the active tab's auto-limited result by `delta` pages (±1). The page
   * size is the server's applied LIMIT (`auto_limited`); re-runs the same
   * statement with the new row offset (server appends OFFSET / Mongo skip).
   * "Next" on a keyset-eligible Mongo find also echoes the page's `next_cursor`
   * back as `cursor`, so the server pages by `_id > cursor` instead of `skip`;
   * "Prev" stays offset-based. No-op when the current result wasn't auto-paginated.
   */
  runPage(
    delta: number,
    extra?: Pick<
      NonNullable<Parameters<DatabaseStore['runQuery']>[2]>,
      'readOnly' | 'confirmWrite' | 'agentLabel' | 'outcome' | 'awaitingHuman'
    >,
  ): Promise<QueryResult | null> {
    const t = this.db.tab;
    const pageSize = t?.result?.auto_limited ?? 0;
    // No known producing statement (a re-attached run from an older marker):
    // never fall back to re-running the whole editor buffer.
    if (!t || pageSize <= 0 || !t.ran_statement) return Promise.resolve(null);
    const next = Math.max(0, t.offset + delta * pageSize);
    if (next === t.offset) return Promise.resolve(null);
    // The cursor belongs to the page the user is LEAVING — read it before the
    // re-run replaces `t.result`.
    const cursor = delta > 0 ? (t.result?.next_cursor ?? undefined) : undefined;
    const prev = t.offset;
    t.offset = next;
    // How the run ended — the caller's object when it passed one (an agent
    // command reads it back), else a local one.
    const outcome: RunOutcome = extra?.outcome ?? {};
    // Page the statement (and scope node) that PRODUCED the result — the editor
    // buffer / active DB may have been edited since the run. `ran_node` is
    // passed as-is: a `null` (ran unscoped) must stay unscoped, not fall back
    // to a database selected since. `transient` keeps the buffer untouched.
    return this.runQuery(t.ran_statement ?? undefined, t.ran_node, {
      ...extra,
      outcome,
      keepOffset: true,
      transient: true,
      cursor,
      // Keep the page size the pages are counted in: the result being paged
      // may have run with a different cap (an agent's `row_limit`, or a Limit
      // changed since) than the persisted default.
      maxRows: pageSize,
    }).then((r) => {
      // A stopped, declined or failed page change leaves the grid on the page
      // it was on — so must the pager (else the footer says "51–100" over rows
      // 1–50 and the next Next skips a page). A detached run keeps `next`: its
      // re-attach installs THAT page. A newer pager move owns the offset.
      if (!r && outcome.status !== 'detached' && t.offset === next) t.offset = prev;
      return r;
    });
  }

  /**
   * Typed confirmation gate for a write on a guarded (prod / read-only)
   * connection. The user must type the connection name verbatim, so a write to
   * production is a deliberate, explicit act. `'confirmed'` only on an exact,
   * case-insensitive match. A typed name that doesn't match is explained here
   * (the shared prod-gate wording) and answers `'mismatch'`, so callers don't
   * add a second, vaguer "canceled" toast. Public so other write flows (the
   * import dialog keeps an import-worded copy today) can take the same gate.
   */
  async confirmGuardedWrite(
    conn: Pick<Connection, 'name' | 'environment'> | null,
    blocked?: unknown,
    agentLabel?: string,
  ): Promise<GuardedWriteAnswer> {
    if (!conn) return 'declined';
    const prod = conn.environment === 'prod';
    const label = prod ? 'PRODUCTION' : 'read-only';
    // An agent-driven write names who is asking — the typed confirm is the
    // person's decision about SOMEONE ELSE's statement.
    const who = agentLabel ? `${agentLabel} wants to run this. ` : '';
    // Redis `KEYS` is gated too — not a write, but it blocks the whole server
    // while it walks every key; say that instead of "can modify data".
    const blockingKeys = blocked instanceof ApiError && blocked.message.includes('KEYS blocks');
    const body = blockingKeys
      ? `KEYS blocks the Redis server while it scans every key on the ${label} ` +
        `connection "${conn.name}" — prefer SCAN with MATCH/COUNT. Type the ` +
        `connection name to run it anyway.`
      : `You are about to run a WRITE / schema change on the ${label} connection ` +
        `"${conn.name}". This can modify or destroy data. Type the connection ` +
        `name to confirm.`;
    // Titled by what the connection IS — a read-only dev connection is not
    // "production". The danger Modal carries the tone; no glyph.
    const title = blockingKeys
      ? 'Confirm blocking command'
      : prod
        ? 'Confirm production write'
        : 'Confirm write on a read-only connection';
    const typed = await confirmer.promptText(who + body, {
      title,
      confirmLabel: blockingKeys ? 'Run KEYS' : 'Run write',
      placeholder: conn.name,
      danger: true,
    });
    if (typed == null) return 'declined';
    if (typed.trim().toLowerCase() !== conn.name.trim().toLowerCase()) {
      toasts.info('Nothing was changed', 'What you typed didn’t match, so the action was canceled.');
      return 'mismatch';
    }
    return 'confirmed';
  }

  /**
   * Run an arbitrary statement against the selected connection (used by inline
   * cell edits / row deletes), applying the production / read-only write-gate:
   * if the server blocks the write, ask for a typed confirmation and retry with
   * the confirm flag. Returns the result, or `{ canceled }` saying why nothing
   * ran (see {@link ManagedWriteCanceled}).
   *
   * Throws on any non-guardrail failure so the caller keeps its own error UX.
   */
  async runManagedStatement(sql: string, node?: string | null): Promise<QueryResult | ManagedWriteCanceled> {
    const id = this.db.selectedConnId;
    const origin = this.db.selectedConn ? { ...this.db.selectedConn } : null;
    const accessEpoch = this.db.accessEpoch;
    const connEpoch = id ? this.db.epochs.of(id) : 0;
    if (!id) throw new Error('No connection selected');
    // Same scope rule as `runQuery`: only an OMITTED node means "active DB";
    // an explicit `null` (a grid edit of a result that ran unscoped) stays
    // unscoped instead of landing in whatever database is selected now.
    const scopeNode = node === undefined ? this.db.activeDb || null : node;
    const post = (confirmWrite: boolean): Promise<QueryResult> =>
      api.post<QueryResult>(`${this.db.connBase(id)}/query`, {
        statement: sql,
        node: scopeNode,
        confirm_write: confirmWrite,
      });
    try {
      return await post(false);
    } catch (e) {
      if (isWriteBlocked(e)) {
        const answer = await this.confirmGuardedWrite(origin, e);
        if (answer !== 'confirmed') return { canceled: answer };
        if (accessEpoch !== this.db.accessEpoch || connEpoch !== this.db.epochs.of(id)) return { canceled: 'stale' };
        return await post(true);
      }
      throw e;
    }
  }

  /**
   * Run a real query plan for the active tab's statement: SQL engines prepend
   * `EXPLAIN`; Mongo sends the `explain` flag (server `explain` command). The
   * plan replaces the tab's result.
   */
  async runExplain(opts?: { readOnly?: boolean }): Promise<QueryResult | null> {
    const id = this.db.selectedConnId;
    const t = this.db.tab;
    if (!id) {
      toasts.error('No connection selected');
      return null;
    }
    const stmt = t.statement.trim();
    if (!stmt) {
      toasts.error('Statement is empty');
      return null;
    }
    const isSql = this.db.capabilities?.sql === true;
    const accessEpoch=this.db.accessEpoch;
    t.running = true;
    t.error = null;
    t.err_statement = null;
    try {
      const body: Record<string, unknown> = isSql
        ? { statement: `EXPLAIN ${stmt}`, max_rows: this.db.rowLimit, node: this.db.activeDb || null }
        : { statement: stmt, max_rows: this.db.rowLimit, node: this.db.activeDb || null, explain: true };
      // An agent's Explain can't smuggle a write through `EXPLAIN ANALYZE …`.
      if (opts?.readOnly) body.read_only = true;
      const result = await api.post<QueryResult>(`${this.db.connBase(id)}/query`, body);
      if(accessEpoch!==this.db.accessEpoch)return null;
      this.installResult(t, result);
      t.ran_statement = stmt;
      t.ran_node = this.db.activeDb || null;
      return result;
    } catch (e) {
      t.error = loadErrorText(e);
      t.err_statement = stmt;
      toastError('Couldn’t explain', e);
      return null;
    } finally {
      t.running = false;
    }
  }

  /**
   * Fetch a NORMALIZED query plan (`POST …/db/query-plan`) for the active
   * statement and open the plan panel. Falls back to the raw `EXPLAIN` → grid
   * path (`runExplain`) if the endpoint fails or can't normalize the plan, so
   * Explain always shows something. The statement is EXPLAIN-wrapped server-side
   * (never executed raw), so this is read-only even on a guarded connection.
   */
  async explainPlan(opts?: { readOnly?: boolean }): Promise<void> {
    const accessEpoch=this.db.accessEpoch;
    const id = this.db.selectedConnId;
    const stmt = this.db.tab.statement.trim();
    if (!id) {
      toasts.error('No connection selected');
      return;
    }
    if (!stmt) {
      toasts.error('Statement is empty');
      return;
    }
    try {
      const plan = await api.post<DbQueryPlan>(`${this.db.connBase(id)}/query-plan`, {
        statement: stmt,
        node: this.db.activeDb || null,
      });
      if(accessEpoch!==this.db.accessEpoch)return;
      this.db.queryPlan = plan;
      this.db.planOpen = true;
    } catch {
      // Engine can't produce a normalized plan (or the endpoint failed) — fall
      // back to the always-available raw EXPLAIN → grid path.
      if(accessEpoch!==this.db.accessEpoch)return;
      this.closePlan();
      await this.runExplain(opts);
    }
  }

  /** Close the query-plan panel and drop the plan. */
  closePlan(): void {
    this.db.planOpen = false;
    this.db.queryPlan = null;
  }

  /**
   * Stop the in-flight query for a tab (defaults to the active tab). Aborts the
   * fetch (drops our HTTP wait) AND tells the server to cancel the query
   * engine-side (`POST …/db/cancel` with the run's `query_id`) so the database
   * stops the heavy work and frees the cached connection — not just our client.
   * The server cancel never blocks stopping the UI. `report` (the user's Stop
   * button / Esc) toasts what the server says the Stop ACHIEVED — "stopped"
   * only when it really stopped; tab closes and superseded runs stay quiet.
   */
  abortQuery(tabId?: number, opts?: { report?: boolean }): void {
    const report = opts?.report === true;
    const id = tabId ?? this.db.tab?.id;
    if (id == null) return;
    const at = this.db.locateTab(id);
    if (at) {
      this.abortRunForTab(at.tab, report);
      // A captured tab ID can belong to a parked connection. Persist its
      // cleared marker there, so restoring/reloading cannot reattach the run.
      this.db.persist.persistTabsNow(at.connId);
      return;
    }
    // No retained tab (a closed/foreign run) — still drop the controller if held.
    const entry = this.runControllers.get(id);
    if (!entry) return;
    this.runControllers.delete(id);
    void this.sendCancel(entry.connId, entry.queryId, report).finally(() => entry.controller.abort());
  }

  /** Ask the server to cancel a run (`POST …/db/cancel`). With `report`, toast
   *  its outcome: only `cancelled` means the database stopped the work. */
  private sendCancel(connId: Id, queryId: string, report: boolean): Promise<void> {
    // Writes have no default transport deadline. Bound Stop so a lost cancel
    // response still releases the original query wait. Never retry cancellation.
    const deadline = withDeadline(READ_DEADLINE_MS);
    return api
      .post<DbCancelOutcome | undefined>(`${this.db.connBase(connId)}/cancel`, { query_id: queryId }, deadline.signal)
      .then((out) => {
        if (!report) return;
        switch (out?.status) {
          case 'aborted':
            toasts.warn(
              'Query stopped in Otto',
              'No further statement will run, but a statement already sent may still finish on the server.',
            );
            break;
          case 'not_stoppable':
            toasts.warn(
              'Query is still running',
              'This engine cannot cancel it — it runs until it finishes or reaches its timeout.',
            );
            break;
          case 'not_running':
            toasts.info('Query had already finished');
            break;
          default:
            toasts.info('Query stopped');
        }
      })
      .catch((e) => {
        if (report) toastError('Couldn’t stop the query', e);
      })
      .finally(deadline.done);
  }

  /**
   * Cancel one tab's run — server-side engine cancel (fire-and-forget), fetch
   * abort, and run-state reset. Works on snapshot tabs of a BACKGROUND
   * connection too, which is why it never persists: the localStorage key
   * belongs to the ACTIVE connection (callers that own it persist themselves).
   * The run's identity is the live controller entry, or — when the HTTP wait
   * was already lost and the tab is in re-attach mode — its pending marker.
   */
  abortRunForTab(t: QueryTab, report = false): void {
    const entry = this.runControllers.get(t.id);
    const target = entry ?? (t.pending ? { ...t.pending } : null);
    if (!target) return;
    this.runControllers.delete(t.id);
    // Governed queries are owned by their HTTP request. Aborting that request
    // first removes the native cancellation handle before /cancel can use it.
    // Capture this controller: a new run may start before cancellation settles.
    void this.sendCancel(target.connId, target.queryId, report).finally(() => entry?.controller.abort());
    // Clear UI ownership immediately; late results/errors cannot affect this
    // tab, and re-attach must not resurrect a run the user explicitly stopped.
    t.running = false;
    t.pending = null;
  }

  /** Forget a tab's pending run marker (persisted with the tabs). */
  private clearPending(t: QueryTab): void {
    if (!t.pending) return;
    t.pending = null;
    this.db.persist.persistTabs();
  }

  /**
   * Re-attach a tab to a run whose HTTP wait was lost: poll the server's
   * `db/query-status` for the tab's `pending` run until it reports `done`
   * (install the parked result/error) or `unknown` (daemon restarted or the
   * outcome expired — stop quietly). The query itself keeps executing
   * server-side the whole time; this only recovers its outcome.
   */
  async reattach(t: QueryTab): Promise<void> {
    const pending = t.pending;
    if (!pending) return;
    // One poll loop per tab — a second kick (restore + catch both fire) joins
    // the existing loop instead of doubling the polling.
    if (this.reattaching.has(t.id)) return;
    this.reattaching.add(t.id);
    try {
      await this.reattachLoop(t, pending);
    } finally {
      this.reattaching.delete(t.id);
    }
  }

  private async reattachLoop(
    t: QueryTab,
    pending: PendingRun,
  ): Promise<void> {
    t.running = true;
    let failures = 0;
    // Re-entrancy / staleness guard: stop when the tab moved on to a different
    // run (a new runQuery replaces `pending`) or the marker was cleared.
    while (t.pending && t.pending.queryId === pending.queryId) {
      let st: DbQueryStatus;
      try {
        st = await api.post<DbQueryStatus>(`${this.db.connBase(pending.connId)}/query-status`, {
          query_id: pending.queryId,
        });
        failures = 0;
      } catch {
        // Daemon unreachable (sleep/restart in progress): retry a few times,
        // then give up for now — the persisted marker lets a later visit retry.
        if (++failures >= 5) {
          t.running = false;
          return;
        }
        await new Promise((r) => setTimeout(r, 2000));
        continue;
      }
      if (st.status === 'running') {
        // A run this page didn't start (agent, other window, an old marker):
        // backfill its real start so the timer doesn't count from 0.
        if (
          st.elapsed_ms != null &&
          t.pending?.queryId === pending.queryId &&
          !t.pending.startedAt
        ) {
          t.pending = { ...t.pending, startedAt: Date.now() - st.elapsed_ms };
          this.db.persist.persistTabs();
        }
        await new Promise((r) => setTimeout(r, 1500));
        continue;
      }
      if (t.pending?.queryId !== pending.queryId) return; // superseded mid-poll
      if (st.status === 'done') {
        if (st.error != null) {
          t.error = st.error;
          t.err_statement = pending.sql ?? null;
        } else if (st.result) {
          this.installResult(t, st.result);
          // The statement + scope the run was ISSUED with (kept in the marker).
          // Never the editor buffer: it is usually a multi-statement script the
          // run only took one statement of, and the pager re-runs ran_statement.
          t.ran_statement = pending.sql ?? null;
          t.ran_node = pending.node ?? null;
          t.error = null;
          t.err_statement = null;
        }
        void this.db.loadHistory(pending.connId);
      }
      // done or unknown: either way this run is over for the client.
      t.running = false;
      this.clearPending(t);
      return;
    }
  }

  /** Kick off re-attach polling for every tab restored with a pending run. */
  reattachPendingTabs(): void {
    for (const t of this.db.tabs) {
      if (t.pending && !this.runControllers.has(t.id)) void this.reattach(t);
    }
  }
}
