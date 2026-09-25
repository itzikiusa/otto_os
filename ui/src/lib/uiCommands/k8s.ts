// Agent UI control — Kubernetes console handlers (`otto.ui_k8s_*`). Each one
// drives the SAME route→store path the user's clicks take (`#/kubernetes/<id>/
// <kind>/<ns>/<name>`, KubernetesPage's route effect → k8s store), waits for
// the store to settle, then answers from it — so what the agent reads is what
// the table/drawer beside its session shows.
//
// Risk: listing/describing/logs are `read`, moving the view is `navigate`,
// actions are `local_write`: an attributed confirm (rememberable per cluster)
// for routine ones, and ALWAYS the typed-name confirm for a destructive action
// (delete pod, rollout undo, scale to 0, prune sync) or any action on a prod
// cluster — the daemon still demands `confirm_name` for the destructive ones.

import { registerUiCommands, registerUiState, UiCommandError, type UiCommandCtx } from '../uiCommands';
import { router } from '../router.svelte';
import { k8s } from '../stores/k8s.svelte';
import { k8sApi, followLogs } from '../api/k8s';
import { confirmer } from '../confirm.svelte';
import { toasts } from '../toast.svelte';
import type { K8sAction, K8sCluster, K8sResourceKind, K8sRow } from '../api/types';
import type { K8sDrawerTab } from '../stores/k8s.svelte';
import { ACTIONS, typedConfirm } from '../../modules/kubernetes/actions';
import { clusterLabel, isKind, kindDef } from '../../modules/kubernetes/k8s-util';
import {
  agentLabel,
  asUiError,
  capList,
  dismissOnAbort,
  highlightWhenReady,
  resolveByIdOrName,
  tailText,
  waitFor,
} from './pagePort';

const DRAWER_TABS: K8sDrawerTab[] = ['overview', 'manifest', 'describe', 'events', 'logs', 'metrics', 'pods'];

async function clusterFor(key: string | undefined, signal: AbortSignal): Promise<K8sCluster> {
  if (!k8s.clustersLoaded) await k8s.loadClusters();
  if (signal.aborted) throw new UiCommandError('cancelled_by_user', 'Cancelled');
  if (!key) {
    if (k8s.cluster) return k8s.cluster;
    throw new UiCommandError('invalid_args', 'Pass `cluster` (id or name — otto.ui_k8s_list_clusters).');
  }
  return resolveByIdOrName(k8s.clusters, key, (c) => c.id, (c) => c.name || c.context_name, 'Kubernetes cluster');
}

function kindArg(kind: unknown, fallback: K8sResourceKind = 'pods'): K8sResourceKind {
  if (kind === undefined || kind === null || kind === '') return fallback;
  if (typeof kind === 'string' && isKind(kind)) return kind;
  throw new UiCommandError('invalid_args', `Unknown kind “${String(kind)}”.`);
}

const base = (c: K8sCluster): string => `kubernetes/${encodeURIComponent(c.id)}`;

/** Route the workspace to (cluster, kind[, ns]) and wait for the table to load
 *  for exactly that selection. Returns the loaded rows (unfiltered). */
async function showTable(
  c: K8sCluster,
  kind: K8sResourceKind,
  ns: string | undefined,
  ctx: UiCommandCtx,
): Promise<K8sRow[]> {
  router.go(`${base(c)}/${kind}`);
  await waitFor(() => k8s.clusterId === c.id && k8s.kind === kind, ctx.signal, 10_000, 'the cluster workspace');
  if (ns !== undefined && ns.trim().toLowerCase() !== k8s.namespace) k8s.setNamespace(ns);
  const key = k8s.currentKey;
  // The workspace's own effect kicks the load on a key change; for the SAME
  // key (cached rows) refresh in place so the agent never reads stale rows.
  if (!k8s.rowsLoading && k8s.rowsKey === key) await k8s.loadResources(true);
  await waitFor(
    () => k8s.currentKey === key && k8s.rowsKey === key && !k8s.rowsLoading,
    ctx.signal,
    30_000,
    'the resource list',
  );
  if (k8s.rowsError) throw new UiCommandError('failed', k8s.rowsError);
  return k8s.rows;
}

function rowSummary(r: K8sRow): Record<string, unknown> {
  return {
    name: r.name,
    namespace: r.namespace || undefined,
    status: r.status,
    ready: r.ready ?? undefined,
    restarts: r.restarts ?? undefined,
    age_seconds: r.age_seconds,
    node: r.node ?? undefined,
    health: r.health ?? undefined,
    extra: Object.keys(r.extra ?? {}).length ? r.extra : undefined,
  };
}

/** Select a row (drawer opens on `tab`) — the route the user's click takes. */
async function selectRow(
  c: K8sCluster,
  kind: K8sResourceKind,
  ns: string,
  name: string,
  tab: K8sDrawerTab,
  ctx: UiCommandCtx,
): Promise<void> {
  k8s.drawerTab = tab;
  router.go(`${base(c)}/${kind}/${encodeURIComponent(ns || '-')}/${encodeURIComponent(name)}`);
  await waitFor(
    () => k8s.clusterId === c.id && k8s.kind === kind && k8s.selected?.name === name && (k8s.selected?.ns ?? '') === ns,
    ctx.signal,
    10_000,
    'the resource drawer',
  );
  void highlightWhenReady(ctx, '[data-testid="k8s-drawer"]');
}

const where = (c: K8sCluster, ns: string): string =>
  `${ns ? `${ns} on ` : ''}${clusterLabel(c)}${c.environment === 'prod' ? ' (PRODUCTION)' : ''}`;

registerUiCommands('kubernetes', {
  async k8s_list_clusters(_args, ctx) {
    router.go('kubernetes');
    await k8s.loadClusters();
    if (k8s.clustersError) throw new UiCommandError('failed', k8s.clustersError);
    void highlightWhenReady(ctx, '[data-testid="k8s-page"]');
    return {
      clusters: k8s.clusters.map((c) => ({
        id: c.id,
        name: c.name,
        context: c.context_name,
        environment: c.environment,
        default_namespace: c.default_namespace ?? null,
      })),
    };
  },

  async k8s_open(args: { cluster: string; kind?: string; namespace?: string }, ctx) {
    const c = await clusterFor(args.cluster, ctx.signal);
    const kind = kindArg(args.kind, k8s.clusterId === c.id ? k8s.kind : 'pods');
    const rows = await showTable(c, kind, args.namespace, ctx);
    void highlightWhenReady(ctx, '[data-testid="k8s-resource-table"]');
    return {
      cluster: { id: c.id, name: c.name, environment: c.environment },
      kind,
      namespace: k8s.namespace || null,
      namespaces: k8s.namespaces.map((n) => n.name),
      row_count: rows.length,
    };
  },

  async k8s_list_resources(
    args: { cluster?: string; kind?: string; namespace?: string; filter?: string; limit?: number },
    ctx,
  ) {
    const c = await clusterFor(args.cluster, ctx.signal);
    const kind = kindArg(args.kind, k8s.clusterId === c.id ? k8s.kind : 'pods');
    await showTable(c, kind, args.namespace, ctx);
    // The filter lands in the workspace's own filter box, so the user sees
    // the same narrowed table the agent reads.
    if (args.filter !== undefined) k8s.filter = args.filter;
    const rows = k8s.filteredRows;
    const { items, total, truncated } = capList(rows, Math.min(Math.max(args.limit ?? 200, 1), 200));
    void highlightWhenReady(ctx, '[data-testid="k8s-resource-table"]');
    return {
      cluster: c.name,
      kind,
      namespace: k8s.namespace || null,
      filter: k8s.filter || null,
      has_metrics: k8s.hasMetrics,
      total,
      truncated,
      rows: items.map(rowSummary),
    };
  },

  async k8s_select(
    args: { cluster?: string; kind: string; namespace?: string; name: string; tab?: string },
    ctx,
  ) {
    const c = await clusterFor(args.cluster, ctx.signal);
    const kind = kindArg(args.kind);
    const tab = (args.tab ?? 'overview') as K8sDrawerTab;
    if (!DRAWER_TABS.includes(tab)) throw new UiCommandError('invalid_args', `Unknown drawer tab “${args.tab}”.`);
    await selectRow(c, kind, (args.namespace ?? '').trim().toLowerCase(), args.name, tab, ctx);
    const row = k8s.selectedRow;
    return { cluster: c.name, kind, selected: row ? rowSummary(row) : { name: args.name, namespace: args.namespace ?? '' }, tab };
  },

  async k8s_describe(args: { cluster?: string; kind: string; namespace?: string; name: string }, ctx) {
    const c = await clusterFor(args.cluster, ctx.signal);
    const kind = kindArg(args.kind);
    const ns = (args.namespace ?? '').trim().toLowerCase();
    await selectRow(c, kind, ns, args.name, 'describe', ctx);
    try {
      const d = await k8sApi.resource(c.id, kind, ns, args.name, ctx.signal);
      const t = tailText(d.describe, 60_000);
      return {
        cluster: c.name,
        kind,
        namespace: ns || null,
        name: args.name,
        describe: t.text,
        truncated: t.truncated,
        events: (d.events ?? []).slice(-50),
      };
    } catch (e) {
      throw asUiError(e);
    }
  },

  async k8s_logs(
    args: { cluster?: string; namespace: string; pod: string; container?: string; tail_lines?: number; previous?: boolean },
    ctx,
  ) {
    const c = await clusterFor(args.cluster, ctx.signal);
    const ns = args.namespace.trim().toLowerCase();
    await selectRow(c, 'pods', ns, args.pod, 'logs', ctx);
    const tail = Math.min(Math.max(args.tail_lines ?? 200, 1), 5000);
    let text = '';
    try {
      await followLogs(
        c.id,
        ns,
        args.pod,
        { container: args.container || undefined, tail, previous: args.previous || undefined },
        (chunk) => {
          text += chunk;
        },
        ctx.signal,
      );
    } catch (e) {
      throw asUiError(e);
    }
    const t = tailText(text, 60_000);
    return { cluster: c.name, namespace: ns, pod: args.pod, container: args.container ?? null, logs: t.text, truncated: t.truncated };
  },

  async k8s_action(
    args: {
      cluster?: string;
      kind: string;
      namespace?: string;
      name: string;
      action: K8sAction;
      params?: Record<string, unknown>;
    },
    ctx,
  ) {
    const c = await clusterFor(args.cluster, ctx.signal);
    const kind = kindArg(args.kind);
    const ns = (args.namespace ?? '').trim().toLowerCase();
    const defs = (ACTIONS[kind] ?? []).filter((a) => a.id === args.action);
    if (defs.length === 0) {
      const offered = [...new Set((ACTIONS[kind] ?? []).map((a) => a.id))].join(', ') || 'none';
      throw new UiCommandError('invalid_args', `“${args.action}” isn't an action for ${kind} (offered: ${offered}).`);
    }
    const params: Record<string, unknown> = { ...(args.params ?? {}) };
    // Prefer the menu entry whose fixed params match (Promote vs Promote (full)).
    const def =
      defs.find((d) => d.params && Object.entries(d.params).every(([k, v]) => params[k] === v)) ??
      defs.find((d) => !d.params) ??
      defs[0];
    const merged = { ...(def.params ?? {}), ...params };
    delete merged.confirm_name; // only a human's typed confirm sets it
    if (def.id === 'scale' && !Number.isInteger(Number(merged.replicas))) {
      throw new UiCommandError('invalid_args', '`scale` needs `params.replicas` (an integer ≥ 0).');
    }

    // Show the target first: the drawer opens on it, next to the agent.
    await selectRow(c, kind, ns, args.name, 'overview', ctx);

    const singular = kindDef(kind).singular;
    const verb = def.label.replace(/…$/, '');
    const what =
      def.id === 'scale'
        ? `Scale ${singular} “${args.name}” to ${Number(merged.replicas)} replicas`
        : `${verb} ${singular} “${args.name}”`;
    const destructive =
      def.confirm ||
      (def.id === 'scale' && Number(merged.replicas) === 0) ||
      (def.id === 'argocd_sync' && merged.prune === true);
    const who = agentLabel(ctx.agent);

    let ok: boolean;
    if (destructive || c.environment === 'prod') {
      ctx.progress(`Waiting for you to confirm: ${what}`, true);
      // Never remembered: the typed name is the whole point.
      ok = await dismissOnAbort(
        ctx.signal,
        typedConfirm(`${who} wants to: ${what} in ${where(c, ns)}.`, args.name, {
          title: `${verb} (requested by ${who})`,
          confirmLabel: verb,
        }),
      );
      if (ok && destructive) merged.confirm_name = args.name;
    } else if (def.danger) {
      ctx.progress(`Waiting for you to confirm: ${what}`, true);
      ok = await dismissOnAbort(
        ctx.signal,
        confirmer.ask(`${who} wants to: ${what} in ${where(c, ns)}.`, {
          title: `${verb}?`,
          confirmLabel: verb,
          danger: true,
        }),
      );
    } else {
      ok = await ctx.confirmWrite({ what, where: where(c, ns), connId: `k8s:${c.id}`, verb });
    }
    if (!ok) throw new UiCommandError('cancelled_by_user', 'The user declined the action.');
    ctx.progress(`${verb} · ${args.name}`);

    try {
      const resp = await k8sApi.action(c.id, {
        action: def.id,
        kind,
        ns,
        name: args.name,
        params: Object.keys(merged).length ? merged : undefined,
      });
      if (resp.ok) toasts.success(`${verb} · ${args.name}`, `${who}${resp.message ? ` — ${resp.message}` : ''}`);
      else toasts.error(`${verb} failed · ${args.name}`, resp.message || undefined);
      void k8s.loadResources(true);
      return { ok: resp.ok, message: resp.message ?? null, output: resp.output ?? null };
    } catch (e) {
      throw asUiError(e);
    }
  },
});

// `otto.ui_state` view: which cluster / kind / namespace / row is showing.
registerUiState('kubernetes', () => ({
  cluster: k8s.cluster ? { id: k8s.cluster.id, name: k8s.cluster.name, environment: k8s.cluster.environment } : null,
  kind: k8s.clusterId ? k8s.kind : null,
  namespace: k8s.clusterId ? k8s.namespace || null : null,
  filter: k8s.filter || null,
  selected: k8s.selected,
  drawer_tab: k8s.selected ? k8s.drawerTab : null,
  rows: k8s.rowsKey === k8s.currentKey ? k8s.rows.length : null,
}));
