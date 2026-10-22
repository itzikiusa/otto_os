// Database Explorer store — Superset-style dashboards and their widgets, one
// workspace at a time. The DatabaseStore façade (../database.svelte.ts) owns an
// instance and delegates its `dashboards` / `widgets` fields and methods here;
// the host supplies the workspace base, the access epoch and the selected
// connection (the guards every load and create run against).

import { api } from '../../api/client';
import type { DbDashboard, DbViz, DbWidget, DbWidgetMapping, Id, QueryResult } from '../../api/types';
import { toasts } from '../../toast.svelte';
import { toastError } from '../../toastError';
import { loadErrorText } from '../../loadError';
import { obviousWriteVerb } from '../../database/sql-dialect';

/** What the dashboards store reads from its DatabaseStore host. */
export interface DashboardsHost {
  /** `/workspaces/{id}/db`, or null with no workspace. */
  wsBase(): string | null;
  /** Bumped on every access change: a load started before one must not land. */
  accessEpoch(): number;
  selectedConnId(): Id | null;
}

/** Dashboard widgets re-run unattended on every refresh, so they must be
 *  read-only queries. An obvious write is refused at save time with the
 *  reason; the daemon runs every widget read-only regardless (the gate). */
function widgetStatementOk(statement: string): boolean {
  const verb = obviousWriteVerb(statement);
  if (!verb) return true;
  toasts.error('Widgets must be read-only queries', `${verb} changes data — a widget re-runs it on every refresh.`);
  return false;
}

export class DbDashboardsStore {
  private readonly host: DashboardsHost;
  constructor(host: DashboardsHost) {
    this.host = host;
  }

  dashboards: DbDashboard[] = $state([]);
  widgets: DbWidget[] = $state([]);
  /** First/any dashboards load in flight, and the cause of the last failure
   *  (inline in Dashboards.svelte with Retry — the previous list is kept). */
  dashboardsLoading = $state(false);
  dashboardsError: string | null = $state(null);
  selectedDashboardId: Id | null = $state(null);

  selectedDashboard: DbDashboard | null = $derived(
    this.dashboards.find((d) => d.id === this.selectedDashboardId) ?? null,
  );

  // ── Dashboards ────────────────────────────────────────────────────────────

  /** In-flight dashboards(+widgets) load per workspace + access epoch, shared
   *  like connections (an epoch bump must start a fresh load, not join one
   *  whose result will be dropped). */
  private dashboardsInFlight: { key: string; p: Promise<void> } | null = null;
  /** The workspace the dashboards/widgets lists belong to, and the latest
   *  load tickets — same guard as `loadSavedQueries`: a late response from a
   *  workspace (or access epoch) the user has left must not land, since rename
   *  / delete / add-widget act on these ids by id alone. */
  private dashboardsScope: string | null = null;
  private dashboardsRequest = 0;
  private widgetsRequest = 0;
  loadDashboards(): Promise<void> {
    const key = `${this.host.wsBase() ?? ''}:${this.host.accessEpoch()}`;
    if (this.dashboardsInFlight?.key === key) return this.dashboardsInFlight.p;
    const p = this.fetchDashboards().finally(() => {
      if (this.dashboardsInFlight?.p === p) this.dashboardsInFlight = null;
    });
    this.dashboardsInFlight = { key, p };
    return p;
  }

  /** Entering another workspace drops the previous one's dashboards, widgets,
   *  selection and error — never show (or act on) A's dashboards under B. */
  private enterDashboardsScope(base: string | null): void {
    if (this.dashboardsScope === base) return;
    this.dashboards = [];
    this.widgets = [];
    // The very first load keeps a selection made before it (a Live-context
    // "open dashboard" link); fetchDashboards validates it against the list.
    if (this.dashboardsScope !== null) this.selectedDashboardId = null;
    this.dashboardsError = null;
    this.dashboardsScope = base;
  }

  private async fetchDashboards(): Promise<void> {
    const accessEpoch = this.host.accessEpoch();
    const base = this.host.wsBase();
    const request = ++this.dashboardsRequest;
    this.enterDashboardsScope(base);
    if (!base) { this.dashboardsLoading = false; return; }
    const current = () => request === this.dashboardsRequest && base === this.host.wsBase() && accessEpoch === this.host.accessEpoch();
    this.dashboardsLoading = true;
    try {
      const dashboards = await api.get<DbDashboard[]>(`${base}/dashboards`);
      if (!current()) return;
      this.dashboards = dashboards;
      if (dashboards.length > 0 && !dashboards.some((d) => d.id === this.selectedDashboardId)) {
        this.selectedDashboardId = dashboards[0].id;
      }
      this.dashboardsError = null;
      await this.loadWidgets();
    } catch (e) {
      if (current()) this.dashboardsError = loadErrorText(e);
    } finally {
      // A superseded load must not clear the spinner of the load that replaced it.
      if (current()) this.dashboardsLoading = false;
    }
  }

  async loadWidgets(): Promise<void> {
    const accessEpoch = this.host.accessEpoch();
    const base = this.host.wsBase();
    const request = ++this.widgetsRequest;
    this.enterDashboardsScope(base);
    if (!base) return;
    const current = () => request === this.widgetsRequest && base === this.host.wsBase() && accessEpoch === this.host.accessEpoch();
    try {
      const widgets = await api.get<DbWidget[]>(`${base}/widgets`);
      if (current()) this.widgets = widgets;
    } catch (e) {
      // Widgets belong to the dashboards view: surface it there, keep the old widgets.
      if (current()) this.dashboardsError = loadErrorText(e);
    }
  }

  async createDashboard(name: string): Promise<DbDashboard | null> {
    const base = this.host.wsBase();
    if (!base) return null;
    try {
      const d = await api.post<DbDashboard>(`${base}/dashboards`, { name });
      if (base !== this.host.wsBase()) return d; // switched workspace meanwhile
      this.dashboards = [...this.dashboards, d];
      this.selectedDashboardId = d.id;
      return d;
    } catch (e) {
      toastError('Couldn’t create dashboard', e);
      return null;
    }
  }

  async renameDashboard(id: Id, name: string): Promise<void> {
    try {
      const d = await api.patch<DbDashboard>(`/db/dashboards/${id}`, { name });
      this.dashboards = this.dashboards.map((x) => (x.id === id ? d : x));
    } catch (e) {
      toastError('Couldn’t rename dashboard', e);
    }
  }

  async setDashboardRefresh(id: Id, refresh_secs: number | null): Promise<void> {
    try {
      const d = await api.patch<DbDashboard>(`/db/dashboards/${id}`, { refresh_secs });
      this.dashboards = this.dashboards.map((x) => (x.id === id ? d : x));
    } catch (e) {
      toastError('Couldn’t update the dashboard', e);
    }
  }

  async deleteDashboard(id: Id): Promise<void> {
    try {
      await api.del(`/db/dashboards/${id}`);
      this.dashboards = this.dashboards.filter((d) => d.id !== id);
      this.widgets = this.widgets.filter((w) => w.dashboard_id !== id);
      if (this.selectedDashboardId === id) {
        this.selectedDashboardId = this.dashboards[0]?.id ?? null;
      }
    } catch (e) {
      toastError('Couldn’t delete dashboard', e);
    }
  }

  // ── Widgets ────────────────────────────────────────────────────────────────

  async createWidget(input: {
    title: string;
    statement: string;
    viz: DbViz;
    mapping?: DbWidgetMapping;
    options?: Record<string, unknown>;
    dashboard_id?: Id | null;
    connection_id?: Id | null;
  }): Promise<DbWidget | null> {
    const base = this.host.wsBase();
    const connId = input.connection_id ?? this.host.selectedConnId();
    if (!base || !connId) {
      toasts.error('No connection selected');
      return null;
    }
    if (!widgetStatementOk(input.statement)) return null;
    try {
      const w = await api.post<DbWidget>(`${base}/widgets`, {
        connection_id: connId,
        title: input.title,
        statement: input.statement,
        viz: input.viz,
        dashboard_id: input.dashboard_id ?? this.selectedDashboardId,
        mapping: input.mapping ?? {},
        options: input.options ?? {},
      });
      if (base === this.host.wsBase()) this.widgets = [...this.widgets, w];
      toasts.success('Widget added', w.title);
      return w;
    } catch (e) {
      toastError('Couldn’t create widget', e);
      return null;
    }
  }

  /** The updated widget, or null when the statement was rejected (write verb)
   *  or the save failed — both already toasted; the caller keeps its form open. */
  async updateWidget(id: Id, patch: Partial<Pick<DbWidget, 'title' | 'statement' | 'viz' | 'mapping' | 'options' | 'dashboard_id'>>): Promise<DbWidget | null> {
    if (patch.statement !== undefined && !widgetStatementOk(patch.statement)) return null;
    try {
      const w = await api.patch<DbWidget>(`/db/widgets/${id}`, patch);
      this.widgets = this.widgets.map((x) => (x.id === id ? w : x));
      return w;
    } catch (e) {
      toastError('Couldn’t update the widget', e);
      return null;
    }
  }

  async deleteWidget(id: Id): Promise<void> {
    try {
      await api.del(`/db/widgets/${id}`);
      this.widgets = this.widgets.filter((w) => w.id !== id);
    } catch (e) {
      toastError('Couldn’t delete widget', e);
    }
  }

  async runWidget(id: Id): Promise<QueryResult | null> {
    try {
      return await api.post<QueryResult>(`/db/widgets/${id}/run`, {});
    } catch (e) {
      toastError('Couldn’t run the widget query', e);
      return null;
    }
  }

  /** Widgets belonging to the selected dashboard. */
  widgetsForSelectedDashboard(): DbWidget[] {
    const did = this.selectedDashboardId;
    if (!did) return [];
    return this.widgets.filter((w) => w.dashboard_id === did);
  }
}
