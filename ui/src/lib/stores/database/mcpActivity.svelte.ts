// Database Explorer store — live MCP database activity (`db_mcp_activity` WS
// events). Keeps a bounded reactive ring (`activities`, newest first, max 100)
// and projects agent MCP queries (`otto_db_query` / `otto_query_db`) directly
// onto an attributed `QueryTab` on `DatabasePage` so the person sees the live
// SQL, running spinner, result preview, or error in real time.

import type { Id, OttoEvent, QueryResult } from '../../api/types';
import { estimateResultBytes, resultBudget } from '../db-result-budget';
import { blankTab, rawResult } from './model';
import type { ConnSnapshot, QueryTab, QueryTabAgent } from './model';
import type { DatabaseStore } from '../database.svelte';

export type DbMcpActivityEvent = Extract<OttoEvent, { type: 'db_mcp_activity' }>;

export interface DbMcpActivityItem {
  activity_id: string;
  workspace_id: Id;
  connection_id: Id;
  session_id: Id | null;
  session_title: string | null;
  tool: string;
  sql: string | null;
  node: string | null;
  status: 'started' | 'finished' | 'error';
  row_count: number | null;
  duration_ms: number | null;
  columns: string[];
  rows_preview: unknown[][];
  error: string | null;
  ts: string;
}

const MAX_ACTIVITIES = 100;
const MAX_TRACKED_RUNS = 200;

export type McpActivityHost = Pick<
  DatabaseStore,
  | 'activeTab'
  | 'conn'
  | 'connections'
  | 'enforceResultBudget'
  | 'locateTab'
  | 'mainTab'
  | 'openConnIds'
  | 'openConnection'
  | 'persist'
  | 'savedQueries'
  | 'selectedConnId'
  | 'tabs'
  | 'warm'
>;

function toActivityItem(ev: DbMcpActivityEvent): DbMcpActivityItem {
  return {
    activity_id: ev.activity_id,
    workspace_id: ev.workspace_id,
    connection_id: ev.connection_id,
    session_id: ev.session_id,
    session_title: ev.session_title,
    tool: ev.tool,
    sql: ev.sql,
    node: ev.node,
    status: ev.status,
    row_count: ev.row_count,
    duration_ms: ev.duration_ms,
    columns: ev.columns,
    rows_preview: ev.rows_preview,
    error: ev.error,
    ts: ev.ts,
  };
}

export class DbMcpActivityStore {
  private readonly db: McpActivityHost;
  /** Newest-first bounded feed of MCP database calls (max 100). */
  activities: DbMcpActivityItem[] = $state([]);
  /** Maps an in-flight `activity_id` to the `QueryTab.id` it started on. */
  private readonly tabByActivity = new Map<string, number>();

  constructor(db: McpActivityHost) {
    this.db = db;
  }

  clear(): void {
    this.activities = [];
    this.tabByActivity.clear();
  }

  handleMcpActivity(ev: DbMcpActivityEvent): void {
    this.upsertActivity(ev);

    const sql = typeof ev.sql === 'string' ? ev.sql.trim() : '';
    if (!sql) return;
    if (!this.db.connections.some((c) => c.id === ev.connection_id)) return;

    const connId = ev.connection_id;
    const agent: QueryTabAgent = {
      session_id: ev.session_id ?? 'mcp',
      label: ev.session_title ? `MCP · ${ev.session_title}` : 'Agent MCP',
    };

    const tab = this.resolveOrCreateTab(connId, ev.activity_id, sql, agent);
    if (!tab) return;

    if (ev.node !== null && ev.node !== undefined) {
      tab.ran_node = ev.node;
    }

    if (ev.status === 'started') {
      tab.statement = ev.sql!;
      tab.running = true;
      tab.error = null;
      tab.err_statement = null;
    } else if (ev.status === 'finished') {
      if (!tab.statement.trim()) tab.statement = ev.sql!;
      tab.running = false;
      tab.error = null;
      tab.ran_statement = ev.sql!;
      if (ev.columns.length > 0) {
        const result: QueryResult = {
          columns: ev.columns.map((name) => ({ name, type_hint: null })),
          rows: ev.rows_preview,
          stats: {
            row_count: ev.row_count ?? ev.rows_preview.length,
            duration_ms: ev.duration_ms ?? 0,
          },
          truncated: (ev.row_count ?? 0) > ev.rows_preview.length,
        };
        tab.result = rawResult(result);
        resultBudget.note(tab.uid, estimateResultBytes(result));
        this.db.enforceResultBudget();
      }
      this.tabByActivity.delete(ev.activity_id);
    } else if (ev.status === 'error') {
      if (!tab.statement.trim()) tab.statement = ev.sql!;
      tab.running = false;
      tab.error = ev.error ?? 'MCP query failed';
      tab.err_statement = ev.sql!;
      this.tabByActivity.delete(ev.activity_id);
    }

    if (this.db.selectedConnId === connId) {
      this.db.persist.persistTabs();
    } else {
      this.db.persist.persistTabsNow(connId);
    }
  }

  private upsertActivity(ev: DbMcpActivityEvent): void {
    const item = toActivityItem(ev);
    const idx = this.activities.findIndex((a) => a.activity_id === ev.activity_id);
    if (idx === 0) {
      const next = [...this.activities];
      next[0] = item;
      this.activities = next;
    } else if (idx > 0) {
      const rest = this.activities.filter((_, i) => i !== idx);
      this.activities = [item, ...rest].slice(0, MAX_ACTIVITIES);
    } else {
      this.activities = [item, ...this.activities].slice(0, MAX_ACTIVITIES);
    }
  }

  private ensureConnectionOpen(connId: Id): 'active' | 'parked' {
    if (this.db.selectedConnId === null) {
      // `openConnection` synchronously sets `selectedConnId`, appends `connId`
      // to `openConnIds`, and initializes `this.db.tabs` before awaiting schema.
      void this.db.openConnection(connId);
      return 'active';
    }
    if (this.db.selectedConnId === connId) {
      return 'active';
    }
    if (!this.db.openConnIds.includes(connId)) {
      this.db.openConnIds = [...this.db.openConnIds, connId];
      this.db.persist.persistWorkbench();
    }
    if (!this.db.conn.snapshots.has(connId)) {
      this.db.conn.snapshots.set(connId, this.emptyParkedSnapshot(connId));
      void this.db.warm(connId);
    }
    return 'parked';
  }

  private emptyParkedSnapshot(connId: Id): ConnSnapshot {
    const restored = this.db.persist.restoreTabs(connId);
    const view = this.db.persist.restoreView(connId);
    return {
      capabilities: null,
      testResult: null,
      activeDb: restored?.activeDb ?? null,
      schemaRoot: [],
      childrenCache: new Map(),
      expanded: new Set(),
      loadingNodes: new Set(),
      schemaLoading: false,
      selectedObjectPath: null,
      objectDetail: null,
      objectError: null,
      objectSearchQuery: '',
      objectSearchHits: null,
      objectSearchTruncated: false,
      objectSearchScanned: 0,
      builderTablesCache: new Map(),
      tabs: restored?.tabs ?? [blankTab()],
      activeTab: restored?.activeTab ?? 0,
      savedQueries: this.db.savedQueries,
      history: [],
      mainTab: view?.main ?? 'query',
      sideTab: view?.side ?? 'schema',
      connView: view?.view ?? null,
    };
  }

  private resolveOrCreateTab(
    connId: Id,
    activityId: string,
    sql: string,
    agent: QueryTabAgent,
  ): QueryTab | null {
    const mode = this.ensureConnectionOpen(connId);

    const trackedId = this.tabByActivity.get(activityId);
    if (trackedId !== undefined) {
      const hit = this.db.locateTab(trackedId);
      if (hit && hit.connId === connId) {
        hit.tab.agent = { ...agent };
        return hit.tab;
      }
    }

    const tabs = mode === 'active' ? this.db.tabs : this.db.conn.snapshots.get(connId)?.tabs;
    if (!tabs) return null;

    // 1. Reuse an idle tab already attributed to this same agent session.
    const reusable = tabs.find((t) => !t.running && t.agent?.session_id === agent.session_id);
    if (reusable) {
      reusable.agent = { ...agent };
      this.rememberTab(activityId, reusable.id);
      return reusable;
    }

    // 2. Claim the single pristine untouched blank tab when nothing else is open.
    if (
      tabs.length === 1 &&
      !tabs[0].statement.trim() &&
      tabs[0].result === null &&
      !tabs[0].running &&
      !tabs[0].error &&
      !tabs[0].agent
    ) {
      const first = tabs[0];
      first.statement = sql;
      first.agent = { ...agent };
      this.rememberTab(activityId, first.id);
      return first;
    }

    // 3. Otherwise append a dedicated attributed tab.
    const created = blankTab(sql);
    created.agent = { ...agent };
    if (mode === 'active') {
      const cur = this.db.tabs[this.db.activeTab];
      const curIsBlankIdle = !!cur && !cur.statement.trim() && cur.result === null && !cur.running;
      this.db.tabs = [...this.db.tabs, created];
      if (curIsBlankIdle) {
        this.db.activeTab = this.db.tabs.length - 1;
        this.db.mainTab = 'query';
      }
      const liveTab = this.db.tabs[this.db.tabs.length - 1] ?? created;
      this.rememberTab(activityId, liveTab.id);
      return liveTab;
    }

    const snap = this.db.conn.snapshots.get(connId);
    if (!snap) return null;
    snap.tabs = [...snap.tabs, created];
    this.rememberTab(activityId, created.id);
    return created;
  }

  private rememberTab(activityId: string, tabId: number): void {
    if (this.tabByActivity.size >= MAX_TRACKED_RUNS) {
      const oldest = this.tabByActivity.keys().next().value;
      if (oldest !== undefined) this.tabByActivity.delete(oldest);
    }
    this.tabByActivity.set(activityId, tabId);
  }
}
