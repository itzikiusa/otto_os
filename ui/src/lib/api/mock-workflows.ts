// Mock-mode Workflows + Kubernetes fleet monitor (ui/src/lib/api/mock.ts routes
// it). Just what the Workflows page reads when it opens — the node catalog,
// templates, the workspace's workflows, one workflow's runs / triggers /
// versions, the Running list + queue and run history — and the fleet-wide
// `GET /k8s/monitor/overview` the Home "Kubernetes" box and the k8s Monitor
// tab read. Writes are not mocked: the page reports them like any refusal.
import type {
  ActiveWorkflowRun,
  K8sMonitorOverviewRow,
  NodeTypeSpec,
  RunListItem,
  Workflow,
  WorkflowGraph,
  WorkflowRun,
  WorkflowTemplate,
} from './types';

type Out = { status?: number; json?: unknown } | undefined;
type MockRoute = { method: string; re: RegExp; handle: (m: RegExpMatchArray, body: unknown, q: URLSearchParams) => Out };

const ago = (mins: number): string => new Date(Date.now() - mins * 60_000).toISOString();
const notFound = (what: string): Out => ({ status: 404, json: { code: 'not_found', message: what } });

const spec = (kind: string, label: string, category: string, description: string, color: string, icon: string, inputs = 1): NodeTypeSpec =>
  ({ kind, label, category, description, inputs, outputs: 1, color, icon });

// A subset of otto-workflows' node_catalog (crates/otto-workflows/src/catalog.rs).
const NODE_TYPES: NodeTypeSpec[] = [
  spec('manual_trigger', 'Manual Trigger', 'Triggers', 'Starts the workflow and emits its input payload.', '#6b7bff', 'play', 0),
  spec('agent_prompt', 'Agent', 'AI', 'Run an agent turn with a prompt; outputs its reply.', '#d97cff', 'command'),
  spec('review_run', 'Review Run', 'AI', 'Multi-agent code review; outputs findings + a 0–100 score + passed.', '#c080ff', 'search'),
  spec('git_pr', 'Git PR', 'Network', 'Draft a PR; with open=true opens it on the remote.', '#46c0a0', 'git-pull-request'),
  spec('http_request', 'HTTP Request', 'Network', 'Call an HTTP endpoint and capture the response.', '#46c0a0', 'globe'),
  spec('condition', 'Condition', 'Flow', 'Evaluate an expression on the input; outputs { result, value }.', '#f0c040', 'git-branch'),
  spec('human_approval', 'Human Approval', 'Flow', 'Pause the run until an operator approves.', '#f0c040', 'user-check'),
  spec('channel_notify', 'Channel Notify', 'Integrations', 'Send a message to a configured Slack/Telegram integration.', '#46c56a', 'message-square'),
];

const FIX_GRAPH: WorkflowGraph = {
  nodes: [
    { id: 'n1', kind: 'manual_trigger', name: 'Start', x: 80, y: 160, params: {} },
    { id: 'n2', kind: 'agent_prompt', name: 'Fix the issue', x: 320, y: 160, params: { provider: 'claude', prompt: 'Fix {{input.msg}}' } },
    { id: 'n3', kind: 'review_run', name: 'Review', x: 560, y: 160, params: { providers: ['claude'], threshold: 80 } },
    { id: 'n4', kind: 'git_pr', name: 'Open PR', x: 800, y: 160, params: { open: false } },
  ],
  edges: [
    { id: 'e1', source: 'n1', target: 'n2' },
    { id: 'e2', source: 'n2', target: 'n3' },
    { id: 'e3', source: 'n3', target: 'n4', condition: 'input.passed' },
  ],
};
const DIGEST_GRAPH: WorkflowGraph = {
  nodes: [
    { id: 'n1', kind: 'manual_trigger', name: 'Start', x: 80, y: 160, params: {} },
    { id: 'n2', kind: 'http_request', name: 'Fetch status', x: 320, y: 160, params: { method: 'GET', url: 'https://status.example.com/api' } },
    { id: 'n3', kind: 'channel_notify', name: 'Post to Slack', x: 560, y: 160, params: { text: 'Daily digest' } },
  ],
  edges: [{ id: 'e1', source: 'n1', target: 'n2' }, { id: 'e2', source: 'n2', target: 'n3' }],
};

const TEMPLATES: WorkflowTemplate[] = [
  { id: 'fix-review-pr', name: 'Fix → review → PR', description: 'An agent fixes the ask, a review gates it, then a PR is drafted.', instructions: '', icon: 'git-pull-request', graph: FIX_GRAPH },
];

const wfBase = (wid: string) => [
  { id: 'wf_fix', name: 'Fix → review → PR', description: 'Fix a reported bug, review it, draft the PR.', graph: FIX_GRAPH, created_at: ago(60 * 24 * 6), updated_at: ago(60 * 5), version: 4 },
  { id: 'wf_digest', name: 'Daily status digest', description: 'Fetch the status page and post a summary to Slack.', graph: DIGEST_GRAPH, created_at: ago(60 * 24 * 14), updated_at: ago(60 * 24 * 2), version: 2 },
].map((w): Workflow => ({ ...w, workspace_id: wid, instructions: '', on_restart: 'resume', created_by: 'usr_root' }));

/** Rows are made per workspace on first read, so any mock workspace id works. */
const byWorkspace = new Map<string, Workflow[]>();
function workflowsOf(wid: string): Workflow[] {
  let rows = byWorkspace.get(wid);
  if (!rows) byWorkspace.set(wid, (rows = wfBase(wid)));
  return rows;
}
// A deep link can ask for a workflow before any list was read: fall back to
// the default mock workspace's rows.
const findWorkflow = (id: string): Workflow | undefined =>
  [...byWorkspace.values()].flat().find((w) => w.id === id) ?? workflowsOf('wsp_otto').find((w) => w.id === id);

function runsOf(wf: Workflow): WorkflowRun[] {
  const node = (id: string, status: WorkflowRun['nodes'][number]['status'], mins: number) =>
    ({ node_id: id, status, logs: [], started_at: ago(mins), duration_ms: status === 'running' ? null : 42_000 });
  const ids = wf.graph.nodes.map((n) => n.id);
  return [
    { id: `${wf.id}_run3`, status: 'running', started_at: ago(3), nodes: ids.map((n, i) => node(n, i < 2 ? 'success' : i === 2 ? 'running' : 'pending', 3)) },
    { id: `${wf.id}_run2`, status: 'success', started_at: ago(60 * 20), finished_at: ago(60 * 20 - 6), nodes: ids.map((n) => node(n, 'success', 60 * 20)) },
    { id: `${wf.id}_run1`, status: 'error', started_at: ago(60 * 46), finished_at: ago(60 * 46 - 2), error: 'Review score 62 is below the threshold 80', nodes: ids.map((n, i) => node(n, i < 2 ? 'success' : i === 2 ? 'error' : 'skipped', 60 * 46)) },
  ].map((r): WorkflowRun => ({ ...r, workflow_id: wf.id, workspace_id: wf.workspace_id, input: { msg: 'checkout times out on slow networks' }, rev: 3 }) as WorkflowRun);
}
const findRun = (id: string): WorkflowRun | undefined => {
  const wf = findWorkflow(id.replace(/_run\d+$/, ''));
  return wf ? runsOf(wf).find((r) => r.id === id) : undefined;
};

export const workflowsMockRoutes: MockRoute[] = [
  { method: 'GET', re: /^\/workflows\/node-types$/, handle: () => ({ json: NODE_TYPES }) },
  { method: 'GET', re: /^\/workflows\/templates$/, handle: () => ({ json: TEMPLATES }) },
  { method: 'GET', re: /^\/workspaces\/([^/]+)\/workflows$/, handle: (m) => ({ json: workflowsOf(m[1]) }) },
  { method: 'GET', re: /^\/workflows\/([^/]+)$/, handle: (m) => { const w = findWorkflow(m[1]); return w ? { json: w } : notFound('workflow'); } },
  {
    method: 'GET', re: /^\/workflows\/([^/]+)\/runs$/,
    handle: (m) => { const w = findWorkflow(m[1]); return w ? { json: runsOf(w).map(({ id, workflow_id, status, started_at, rev }) => ({ id, workflow_id, status, started_at, rev })) } : notFound('workflow'); },
  },
  { method: 'GET', re: /^\/workflows\/([^/]+)\/triggers$/, handle: () => ({ json: [] }) },
  {
    method: 'GET', re: /^\/workflows\/([^/]+)\/versions$/,
    handle: (m) => { const w = findWorkflow(m[1]); return { json: Array.from({ length: w?.version ?? 1 }, (_, i) => ({ id: `${m[1]}_v${i + 1}`, workflow_id: m[1], version: (w?.version ?? 1) - i, note: i === 0 ? 'Raised the review threshold' : '', created_by: 'usr_root', created_at: ago(60 * 24 * (i + 1)) })) }; },
  },
  { method: 'GET', re: /^\/workflow-runs\/([^/]+)\/progress$/, handle: (m) => { const r = findRun(m[1]); return r ? { json: { changed: true, rev: r.rev ?? 0, run: r } } : notFound('run'); } },
  {
    method: 'GET', re: /^\/workspaces\/([^/]+)\/workflow-runs\/active$/,
    handle: (m) => ({ json: workflowsOf(m[1]).slice(0, 1).map((w): ActiveWorkflowRun => ({ run_id: `${w.id}_run3`, workflow_id: w.id, workspace_id: m[1], workflow_name: w.name, status: 'running', started_at: ago(3), nodes_total: w.graph.nodes.length, nodes_done: 2, current_node_id: 'n3', current_step: 'Review' })) }),
  },
  { method: 'GET', re: /^\/workspaces\/([^/]+)\/workflow-runs\/queue$/, handle: () => ({ json: { max_parallel_runs: 4, queued: [] } }) },
  {
    method: 'GET', re: /^\/workspaces\/([^/]+)\/workflow-runs$/,
    handle: (m) => ({
      json: {
        items: workflowsOf(m[1]).flatMap((w) => runsOf(w).map((r): RunListItem => ({
          id: r.id, workflow_id: w.id, workflow_name: w.name, status: r.status, started_at: r.started_at, finished_at: r.finished_at ?? null,
          duration_ms: r.finished_at ? Date.parse(r.finished_at) - Date.parse(r.started_at) : null, trigger: 'manual',
          message: 'checkout times out on slow networks', current_step: r.status === 'running' ? 'Review' : null, error: r.error ?? null,
        }))).sort((a, b) => b.started_at.localeCompare(a.started_at)),
        next_cursor: null,
      },
    }),
  },
  { method: 'GET', re: /^\/k8s\/monitor\/overview$/, handle: (_m, _b, q) => ({ json: k8sMonitorOverview(q.get('window') ?? '24h') }) },
  // Home's "Today" strip reads these three on every load; unmocked they made
  // it say "Some items couldn't load". Nothing pending / no designs / no tasks.
  { method: 'GET', re: /^\/mcp\/approvals$/, handle: () => ({ json: [] }) },
  { method: 'GET', re: /^\/design\/artifacts$/, handle: () => ({ json: [] }) },
  { method: 'GET', re: /^\/workspaces\/([^/]+)\/scheduled-tasks$/, handle: () => ({ json: [] }) },
  // Home's Usage box: the embedded ClickHouse engine does not run in mock mode.
  {
    method: 'GET', re: /^\/usage\/status$/,
    handle: () => ({ json: { available: false, enabled: false, binary: null, version: null, data_dir: '~/Library/Application Support/Otto/usage', retention_days: 90, metrics_interval_secs: 60, usage_rows: 0, metric_rows: 0, disk_bytes: 0 } }),
  },
];

/** Mirrors the two mock clusters in mock.ts (`k8s_dev`, `k8s_prod`). */
export function k8sMonitorOverview(window: string): K8sMonitorOverviewRow[] {
  const status = (cluster_id: string, pods: number) => ({ cluster_id, last_cycle_at: ago(1), last_ok_at: ago(1), last_error: '', transport_used: 'port-forward', metrics_server: 'ok', pods_seen: pods, pods_scraped: pods - 1, pods_failed: 1, cycle_ms: 4200 });
  return [
    {
      cluster: { id: 'k8s_dev', name: 'dev-eu-1', environment: 'dev', color: '#4f8cff' }, enabled: true, interval_secs: 60, status: status('k8s_dev', 24),
      health: 'degraded', window, pods: { running: 22, pending: 1, failed: 0, crashloop: 1, total: 24 }, restarts: { oom: 2, crash: 14, probe: 1, unknown: 0 },
      churn: 6, mem: { used: 9.4 * 1024 ** 3, limit: 16 * 1024 ** 3, pct: 58.8 }, rps: 182.4, err_pct: 0.7, drift: [{ workload: 'api', versions: ['1.4.0', '1.3.9'] }], workloads: 9,
    },
    {
      cluster: { id: 'k8s_prod', name: 'prod-eu-1', environment: 'prod', color: '#e5484d' }, enabled: true, interval_secs: 60, status: status('k8s_prod', 48),
      health: 'healthy', window, pods: { running: 48, pending: 0, failed: 0, crashloop: 0, total: 48 }, restarts: { oom: 0, crash: 0, probe: 0, unknown: 0 },
      churn: 3, mem: { used: 41 * 1024 ** 3, limit: 96 * 1024 ** 3, pct: 42.7 }, rps: 1240, err_pct: 0.1, drift: [], workloads: 14,
    },
  ];
}
