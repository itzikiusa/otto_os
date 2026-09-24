// Seed the throwaway daemon with a believable, entirely FICTIONAL company
// ("Acme Storefront") through its public HTTP API. Nothing here touches the
// user's real daemon, data or accounts. Returns the ids the capture needs.
import { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { buildDemoRepo } from './demo-repo.mjs';
import { writeVault } from './vault.mjs';
import { writeInsights } from './insights.mjs';

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** Tell the fake agent CLI which script to play for the NEXT session. */
function agentMode(fakeBin, mode) {
  writeFileSync(join(fakeBin, 'mode'), mode);
}

export async function seedAll(api, { dataDir, home, fakeBin, repoRoot, docker }) {
  const ids = {};
  const code = join(home, 'Code');
  mkdirSync(code, { recursive: true });

  // ── workspace + repos ────────────────────────────────────────────────────
  const repoDir = buildDemoRepo(join(code, 'acme-checkout'), home);
  const webDir = buildDemoRepo(join(code, 'acme-web'), home);
  const ws = await api.post('/workspaces', { name: 'Acme Storefront', root_path: code });
  ids.ws = ws.id;
  const W = `/workspaces/${ws.id}`;
  ids.repo = (await api.post(`${W}/repos`, { path: repoDir, name: 'acme-checkout' })).id;
  ids.repoWeb = (await api.post(`${W}/repos`, { path: webDir, name: 'acme-web' })).id;
  ids.repoDir = repoDir;
  // Clear the credential notices the fake HOME triggers (Claude keychain probe).
  await api.try('DELETE', '/notifications');

  // ── agent sessions (scripted stand-ins, never the real CLIs) ─────────────
  const sessions = [
    ['work', 'claude', 'Add rate limiting to checkout', 'claude-opus-4-8'],
    ['flaky', 'codex', 'Fix the flaky cart totals test', 'gpt-5.5-codex'],
    ['review', 'claude', 'Review PR #214 — refunds', 'claude-sonnet-4-8'],
    ['notes', 'claude', 'Draft release notes for 1.5.0', 'claude-opus-4-8'],
  ];
  ids.sessions = [];
  for (const [mode, provider, title, model] of sessions) {
    agentMode(fakeBin, mode);
    const s = await api.post(`${W}/sessions`, { kind: 'agent', provider, title, cwd: repoDir, model, meta: { origin: 'manual' } });
    ids.sessions.push(s.id);
    await sleep(900);
  }
  const shell = await api.post(`${W}/sessions`, { kind: 'agent', provider: 'shell', title: 'checkout · dev server', cwd: repoDir, meta: { origin: 'manual' } });
  ids.shell = shell.id;
  await sleep(600);
  await api.try('POST', `/sessions/${shell.id}/input`, {
    text: `clear; for i in $(seq 1 3000); do printf '\\033[2m%s\\033[0m  \\033[32mGET\\033[0m /api/checkout/%d  \\033[36m200\\033[0m  %dms\\n' "$(date +%H:%M:%S)" $((RANDOM%900+100)) $((RANDOM%40+8)); sleep 0.7; done`,
    submit: true,
  });

  // ── assistant: threads, memory, tasks, needs-you ─────────────────────────
  const t1 = await api.try('POST', '/assistant/threads', { title: 'Plan the 1.5 release', space_slot: 1, provider: 'claude', model: null, incognito: false });
  const t2 = await api.try('POST', '/assistant/threads', { title: 'Weekly status for the team', space_slot: 2, provider: 'claude', model: null, incognito: false });
  await api.try('POST', '/assistant/threads', { title: 'Compare error trackers', space_slot: 3, provider: 'codex', model: null, incognito: false });
  await api.try('POST', '/assistant/threads', { title: 'Lisbon offsite logistics', space_slot: null, provider: 'claude', model: null, incognito: false });
  ids.thread = t1?.id;
  for (const [text, kind, tags] of [
    ['Release branches are cut on Tuesdays; freeze starts Monday 18:00.', 'fact', ['release']],
    ['Prefers short PR descriptions with a test plan checklist.', 'preference', ['prs']],
    ['Checkout is owned by the Payments squad (Noah, Iris).', 'fact', ['ownership']],
    ['Never deploy on Fridays after 15:00.', 'preference', ['deploy']],
    ['Staging DB is read-only for agents.', 'fact', ['db', 'safety']],
  ]) {
    await api.try('POST', '/assistant/memory', { text, kind, tags });
  }
  const inHours = (h) => new Date(Date.now() + h * 3600000).toISOString();
  await api.try('POST', '/assistant/tasks', { kind: 'reminder', title: 'Release freeze starts', run_at: inHours(3), timezone: 'UTC', thread_id: t1?.id });
  await api.try('POST', '/assistant/tasks', { kind: 'reminder', title: 'Send the weekly status', run_at: inHours(6), timezone: 'UTC', thread_id: t2?.id });
  const task = await api.try('POST', '/assistant/tasks', { kind: 'task', title: 'Collect merged PRs since 1.4', detail: 'Group by area for the release notes.', thread_id: t1?.id });
  const task2 = await api.try('POST', '/assistant/tasks', { kind: 'task', title: 'Pick the release date', detail: 'Two candidates from the team calendar.', thread_id: t1?.id });
  if (task2?.id) {
    await api.try('POST', '/assistant/agent/task_update', { task_id: task2.id, state: 'needs_you', question: 'Ship 1.5 on Tuesday or Thursday?', options: ['Tuesday', 'Thursday'] });
  }
  if (task?.id) await api.try('POST', '/assistant/agent/task_update', { task_id: task.id, state: 'running' });
  await api.try('POST', '/assistant/agent/approval', {
    where: 'Slack · #payments', what: 'Release plan summary for 1.5', who_sees: 'The #payments channel (14 people)',
    reason: 'You asked me to share the plan when it was ready', category: 'post', tool: 'slack_post', destination: '#payments',
  });

  // ── DB connections, saved queries, dashboard ─────────────────────────────
  if (docker.mysql) {
    const c = await api.post(`${W}/connections`, {
      name: 'shop · mariadb', kind: 'mysql', params: { host: '127.0.0.1', port: docker.mysql, user: 'otto', db: 'shopdb' },
      secret: 'ottopw', environment: 'dev', read_only: false,
    });
    ids.mysql = c.id;
    await api.try('POST', `/connections/${c.id}/test`, {});
    for (const [name, statement] of [
      ['Revenue by region', 'SELECT region, COUNT(*) AS orders, SUM(total_cents)/100 AS revenue\nFROM shopdb.orders\nWHERE status <> \'refunded\'\nGROUP BY region\nORDER BY revenue DESC;'],
      ['Top customers', 'SELECT c.name, c.plan, SUM(o.total_cents)/100 AS spent\nFROM shopdb.orders o JOIN shopdb.customers c ON c.id = o.customer_id\nGROUP BY c.id HAVING spent > 500\nORDER BY spent DESC LIMIT 10;'],
    ]) await api.try('POST', `${W}/db/saved-queries`, { connection_id: c.id, name, statement });
    const dash = await api.try('POST', `${W}/db/dashboards`, { name: 'Storefront' });
    if (dash?.id) {
      ids.dashboard = dash.id;
      await api.try('POST', `${W}/db/widgets`, { connection_id: c.id, dashboard_id: dash.id, title: 'Revenue by region', statement: "SELECT region, ROUND(SUM(total_cents)/100) AS revenue FROM shopdb.orders WHERE status <> 'refunded' GROUP BY region ORDER BY revenue DESC", viz: 'bar', mapping: { x: 'region', y: ['revenue'] }, options: {} });
      await api.try('POST', `${W}/db/widgets`, { connection_id: c.id, dashboard_id: dash.id, title: 'Orders by status', statement: 'SELECT status, COUNT(*) AS n FROM shopdb.orders GROUP BY status', viz: 'pie', mapping: { category: 'status', value: 'n' }, options: {} });
      await api.try('POST', `${W}/db/widgets`, { connection_id: c.id, dashboard_id: dash.id, title: 'Daily revenue', statement: 'SELECT day, ROUND(SUM(revenue_cents)/100) AS revenue FROM shopdb.daily_sales GROUP BY day ORDER BY day', viz: 'area', mapping: { x: 'day', y: ['revenue'] }, options: {} });
      await api.try('POST', `${W}/db/widgets`, { connection_id: c.id, dashboard_id: dash.id, title: 'Paid orders', statement: "SELECT COUNT(*) AS paid FROM shopdb.orders WHERE status = 'paid'", viz: 'number', mapping: { value: 'paid' }, options: {} });
    }
  }
  if (docker.mongo) {
    const c = await api.post(`${W}/connections`, {
      name: 'catalog · mongodb', kind: 'mongodb', params: { host: '127.0.0.1', port: docker.mongo, user: 'otto', db: 'shopdb', auth_source: 'admin' },
      secret: 'ottopw', environment: 'dev', read_only: false,
    });
    ids.mongo = c.id;
    await api.try('POST', `/connections/${c.id}/test`, {});
  }

  // ── SSH hosts in sections (rows render without a reachable host) ─────────
  const prod = await api.try('POST', `${W}/connection-sections`, { name: 'Production', parent_id: null });
  const stg = await api.try('POST', `${W}/connection-sections`, { name: 'Staging', parent_id: null });
  for (const [name, host, env, sec] of [
    ['bastion-eu', 'bastion.eu.acme.example', 'prod', prod?.id],
    ['checkout-api-01', 'checkout-01.internal.acme.example', 'prod', prod?.id],
    ['checkout-api-02', 'checkout-02.internal.acme.example', 'prod', prod?.id],
    ['stg-worker', 'worker.stg.acme.example', 'staging', stg?.id],
  ]) {
    await api.try('POST', `${W}/connections`, { name, kind: 'ssh', params: { host, user: 'deploy', port: 22 }, secret: null, environment: env, read_only: false, section_id: sec ?? null });
  }

  // ── Kafka (Redpanda) ─────────────────────────────────────────────────────
  if (docker.kafka) {
    const k = await api.try('POST', `${W}/brokers/clusters`, {
      name: 'orders · redpanda', bootstrap_servers: `127.0.0.1:${docker.kafka}`, security_protocol: 'plaintext',
      schema_registry_url: null, metrics_url: null, environment: 'dev', read_only: false,
    });
    if (k?.id) {
      ids.kafka = k.id;
      for (let i = 0; i < 20; i++) {
        const r = await api.try('POST', `/brokers/clusters/${k.id}/test`, {});
        if (r?.ok) break;
        await sleep(1500);
      }
      for (const [topic, parts] of [['orders.events', 3], ['payments.settled', 2], ['inventory.changes', 2]]) {
        await api.try('POST', `/brokers/clusters/${k.id}/topics`, { name: topic, partitions: parts, replication_factor: 1 });
      }
      const statuses = ['created', 'paid', 'paid', 'shipped', 'refunded'];
      for (let i = 1; i <= 36; i++) {
        await api.try('POST', `/brokers/clusters/${k.id}/topics/orders.events/produce`, {
          key: `order-${1000 + i}`,
          value: JSON.stringify({ order_id: 1000 + i, status: statuses[i % 5], region: ['us-east', 'eu-west', 'eu-central'][i % 3], total_cents: 1500 + ((i * 7919) % 60000), at: new Date(Date.now() - (40 - i) * 60000).toISOString() }),
        });
      }
      for (let i = 1; i <= 8; i++) {
        await api.try('POST', `/brokers/clusters/${k.id}/topics/payments.settled/produce`, { key: `pay-${i}`, value: JSON.stringify({ payment_id: `pay_${i}`, amount_cents: 1200 * i, currency: 'USD' }) });
      }
    }
  }

  // ── AWS + Kubernetes registrations (cards only; no live cloud calls) ─────
  await api.try('POST', '/aws/accounts', { name: 'acme-prod', auth_mode: 'access_keys', region: 'eu-west-1', access_key_id: 'AKIADEMOPLACEHOLDER0', secret_access_key: 'demo-placeholder-not-a-key', environment: 'prod', color: '#ef4444' });
  await api.try('POST', '/aws/accounts', { name: 'acme-staging', auth_mode: 'access_keys', region: 'us-east-1', access_key_id: 'AKIADEMOPLACEHOLDER1', secret_access_key: 'demo-placeholder-not-a-key', environment: 'staging', color: '#f59e0b' });
  const kube = join(dataDir, 'kubeconfig-demo.yaml');
  writeFileSync(kube, `apiVersion: v1\nkind: Config\nclusters:\n- name: demo\n  cluster:\n    server: https://127.0.0.1:1\n    insecure-skip-tls-verify: true\ncontexts:\n- name: prod-eu\n  context: { cluster: demo, user: demo }\n- name: stg-us\n  context: { cluster: demo, user: demo }\ncurrent-context: prod-eu\nusers:\n- name: demo\n  user: { token: demo }\n`);
  await api.try('POST', '/k8s/clusters', { name: 'prod-eu', source: 'kubeconfig', kubeconfig_path: kube, context_name: 'prod-eu', default_namespace: 'checkout', environment: 'prod' });
  await api.try('POST', '/k8s/clusters', { name: 'stg-us', source: 'kubeconfig', kubeconfig_path: kube, context_name: 'stg-us', default_namespace: 'default', environment: 'staging' });

  // ── API client: collection, env, requests against the demo daemon ────────
  await api.try('PATCH', W, { settings: { api_client: { allow_local: true } } });
  const col = await api.try('POST', `${W}/api-client/collections`, { name: 'Checkout API', parent_id: null });
  const env = await api.try('POST', `${W}/api-client/environments`, { name: 'Local', variables: { base_url: `http://127.0.0.1:${process.env.OTTO_E2E_PORT ?? '7811'}` } });
  if (env?.id) await api.try('POST', `${W}/api-client/environments/${env.id}/activate`, {});
  if (col?.id) {
    const reqs = [
      ['Health check', 'GET', '{{base_url}}/api/v1/health'],
      ['List orders', 'GET', '{{base_url}}/api/v1/health?orders=recent'],
      ['Create order', 'POST', '{{base_url}}/api/v1/health'],
      ['Refund order', 'POST', '{{base_url}}/api/v1/health?refund=1'],
    ];
    ids.apiRequests = [];
    for (const [name, method, url] of reqs) {
      const r = await api.try('POST', `${W}/api-client/requests`, {
        collection_id: col.id, name, method, url, headers: [{ key: 'Accept', value: 'application/json', enabled: true }],
        query: [], body_mode: method === 'POST' ? 'json' : 'none', body: method === 'POST' ? '{\n  "order_id": 1042,\n  "reason": "damaged"\n}' : '', auth: { type: 'none' },
      });
      if (r?.id) ids.apiRequests.push(r.id);
    }
    ids.apiEnv = env?.id;
  }

  // ── MCP control plane (local mock stdio server from the repo fixtures) ───
  // A copy under the fake HOME, launched by bare `node`, so no host path shows.
  const mockSrc = join(repoRoot, 'ui/e2e/fixtures/mock-mcp-server.mjs');
  const mock = join(home, 'mcp', 'inventory-server.mjs');
  if (existsSync(mockSrc)) {
    mkdirSync(join(home, 'mcp'), { recursive: true });
    copyFileSync(mockSrc, mock);
    const srv = await api.try('POST', `${W}/mcp/servers`, { name: 'inventory', transport: 'stdio', command: 'node', args: [mock], enabled: true, default_tool_access: 'allow', description: 'Warehouse inventory tools' });
    if (srv?.id) {
      ids.mcp = srv.id;
      await api.try('POST', `/mcp/servers/${srv.id}/discover`, {});
      await api.try('POST', `/mcp/servers/${srv.id}/health`, {});
      await api.try('POST', `/mcp/servers/${srv.id}/tools/list_items/invoke`, { arguments: {} });
      await api.try('POST', `/mcp/servers/${srv.id}/tools/list_items/invoke`, { arguments: {} });
      await api.try('POST', `/mcp/servers/${srv.id}/tools/delete_thing/invoke`, { arguments: { id: 7 }, dry_run: true });
      await api.try('POST', `/mcp/servers/${srv.id}/tools/delete_thing/invoke`, { arguments: { id: 7 } });
    }
    await api.try('POST', '/mcp/policies', { workspace_id: ws.id, name: 'Gate destructive tools', match: { tool_glob: 'delete_*' }, effect: 'require_approval', reason: 'Deletes need a human' });
    await api.try('POST', '/mcp/tokens', { label: 'ci-readonly', scope: { allow_writes: false, tools: null, workspace_id: null } });
  }

  // ── Swarm ────────────────────────────────────────────────────────────────
  const swarm = await api.try('POST', `${W}/swarm/swarms`, { name: 'Checkout Squad', preset_slug: 'engineering-squad' });
  if (swarm?.id) {
    ids.swarm = swarm.id;
    const full = await api.try('GET', `/swarm/swarms/${swarm.id}`);
    let project = full?.projects?.[0];
    if (!project) project = await api.try('POST', `/swarm/swarms/${swarm.id}/projects`, { name: 'Checkout v2', goal_md: 'Ship one-page checkout with saved cards and rate limiting.', repo_path: repoDir });
    const agents = full?.agents ?? [];
    const pick = (i) => agents[i % Math.max(1, agents.length)]?.id ?? null;
    if (project?.id) {
      const tasks = [
        ['Design the one-page checkout flow', 'done', 'high'],
        ['Token-bucket rate limiter', 'in_review', 'urgent'],
        ['Saved cards API', 'in_progress', 'high'],
        ['Retry declined payments', 'in_progress', 'medium'],
        ['Load test at 2k rps', 'todo', 'medium'],
        ['Update the checkout runbook', 'todo', 'low'],
        ['Accessibility pass on the payment form', 'backlog', 'medium'],
        ['Fraud score threshold review', 'blocked', 'high'],
      ];
      for (const [i, [title, status, priority]] of tasks.entries()) {
        const t = await api.try('POST', `/swarm/projects/${project.id}/tasks`, { title, priority, assignee_agent_id: pick(i + 1), description: '', labels: ['checkout'] });
        if (t?.id && status !== 'backlog') await api.try('PATCH', `/swarm/tasks/${t.id}`, { status });
      }
      for (const [kind, body] of [
        ['decision', 'We ship rate limiting behind a flag first, then enable per region.'],
        ['idea', 'Reuse the retry helper for webhook deliveries too.'],
        ['concern', 'Saved cards need a PCI review before launch.'],
        ['message', 'Rate limiter PR is ready for review — 42 tests passing.'],
      ]) await api.try('POST', `/swarm/swarms/${swarm.id}/board`, { body, kind, project_id: project.id });
    }
  }

  // ── Goal loop (draft) ────────────────────────────────────────────────────
  const loop = await api.try('POST', `${W}/goal-loops`, {
    name: 'Checkout p95 under 300 ms', repo_path: repoDir, autostart: false,
    definition: {
      title: 'Checkout p95 under 300 ms', summary: 'Bring checkout latency down without changing behaviour.',
      objectives: ['Remove the N+1 query on order items', 'Cache regional tax rules', 'Keep every existing test green'],
      acceptance_criteria: [
        { id: 'c1', text: 'All tests pass', verify: 'npm test', verify_kind: 'command', verify_cmd: 'npm test' },
        { id: 'c2', text: 'p95 under 300 ms in the load test', verify: 'k6 run load/checkout.js', verify_kind: 'command', verify_cmd: 'k6 run load/checkout.js' },
        { id: 'c3', text: 'Runbook explains the new cache', verify: 'docs mention the tax cache', verify_kind: 'agent' },
      ],
      constraints: ['No new runtime dependencies'], out_of_scope: ['Frontend changes'], success_signal: 'Load test p95 < 300 ms',
    },
    limits: { max_iterations: 6, max_runtime_secs: 5400, per_phase_timeout_secs: 900, max_attempts_per_executor: 2 },
    config: {
      mode: 'build', allow_commits: false, require_review: true, skills: [], source_links: [],
      executors: [{ name: 'Implementer', provider: 'claude', model: '', prompt_extra: '' }, { name: 'Perf tester', provider: 'codex', model: '', prompt_extra: '' }],
      planner: { provider: 'claude', model: '', prompt: 'Plan the next step.' },
      evaluator: { provider: 'claude', model: '', prompt: 'Evaluate against the criteria.' },
      digester: { provider: 'claude', model: '', prompt: 'Digest.' },
      definer: { provider: 'claude', model: '', prompt: 'Define.' },
    },
  });
  ids.loop = loop?.id;

  // ── Workflows ────────────────────────────────────────────────────────────
  const node = (id, kind, name, x, y, params = null) => ({ id, kind, name, x, y, params });
  const edge = (source, target) => ({ id: `${source}-${target}`, source, target, condition: null });
  const wf = await api.try('POST', `${W}/workflows`, {
    name: 'PR review pipeline', description: 'Summarise, review in parallel, ask a human, then post.',
    instructions: '# Rules\n- Never push to main\n- Keep comments short',
    graph: {
      nodes: [
        node('t', 'manual_trigger', 'New pull request', 0, 120),
        node('ctx', 'prepare_context', 'Gather diff + tickets', 260, 120),
        node('sum', 'agent_prompt', 'Summarise the change', 520, 20, { prompt: 'Summarise the diff for reviewers.', provider: 'claude' }),
        node('sec', 'agent_prompt', 'Security review', 520, 220, { prompt: 'Review the diff for security issues.', provider: 'codex' }),
        node('ok', 'human_approval', 'Approve comments', 780, 120, { prompt: 'Post these review comments?' }),
        node('log', 'log', 'Post to the PR', 1040, 120, { message: 'posted' }),
      ],
      edges: [edge('t', 'ctx'), edge('ctx', 'sum'), edge('ctx', 'sec'), edge('sum', 'ok'), edge('sec', 'ok'), edge('ok', 'log')],
    },
  });
  if (wf?.id) {
    ids.workflow = wf.id;
    await api.try('POST', `/workflows/${wf.id}/run`, { input: { prompt: 'PR #214 — refunds' } });
    await sleep(1500);
    await api.try('POST', `/workflows/${wf.id}/run`, { input: { prompt: 'PR #218 — saved cards' } });
  }
  await api.try('POST', `${W}/workflows`, {
    name: 'Nightly dependency audit', description: 'Scan, summarise, open a ticket.', instructions: '',
    graph: { nodes: [node('t', 'manual_trigger', 'Every night', 0, 80), node('a', 'agent_prompt', 'Audit dependencies', 260, 80, { prompt: 'Audit dependencies.', provider: 'claude' }), node('l', 'log', 'File the report', 520, 80, { message: 'ok' })], edges: [edge('t', 'a'), edge('a', 'l')] },
  });

  // ── Scheduled tasks ──────────────────────────────────────────────────────
  const st1 = await api.try('POST', `${W}/scheduled-tasks`, {
    name: 'Morning ticket review', prompt: 'Review new tickets and produce a report.', provider: 'claude',
    schedule: { cadence: 'daily', at: '09:00' }, timezone: 'Europe/London', sandbox: 'worktree', max_retries: 2,
    notify_on_change: true, attach_proof: true, destination: { type: 'none' }, enabled: true,
  });
  await api.try('POST', `${W}/scheduled-tasks`, {
    name: 'Weekly dependency audit', prompt: 'Audit outdated and vulnerable dependencies.', provider: 'codex',
    schedule: { cadence: 'weekly', at: '08:00', weekday: 0 }, timezone: 'Europe/London', sandbox: 'worktree', max_retries: 1,
    notify_on_change: true, attach_proof: false, destination: { type: 'none' }, enabled: true,
  });
  const st3 = await api.try('POST', `${W}/scheduled-tasks`, {
    name: 'Flaky test hunt', prompt: 'Find tests that failed then passed on retry this week.', provider: 'claude',
    schedule: { cadence: 'cron', expr: '0 18 * * 5' }, timezone: 'Europe/London', sandbox: 'worktree', max_retries: 2,
    notify_on_change: false, attach_proof: true, destination: { type: 'none' }, enabled: true,
  });
  if (st3?.id) await api.try('PATCH', `/scheduled-tasks/${st3.id}`, { enabled: false });
  if (st1?.id) {
    ids.scheduled = st1.id;
    await api.try('POST', `/scheduled-tasks/${st1.id}/run`, {});
  }

  // ── Personal agents ──────────────────────────────────────────────────────
  for (const [name, avatar, soul, at, directive] of [
    ['Daily Recap', '🗞️', 'You are a crisp chief of staff. Summaries fit on one screen.', '08:30', 'Summarise yesterday’s merged PRs and open incidents.'],
    ['Inbox Triage', '📬', 'You sort requests by urgency and draft short replies.', '09:15', 'Triage new support escalations.'],
    ['Release Scout', '🚀', 'You watch release branches and flag risky changes early.', '17:00', 'List risky changes on the release branch.'],
  ]) {
    const pa = await api.try('POST', `${W}/personal-agents`, { name, avatar, soul_md: soul, provider: 'claude', model: null, cwd: code, browser: false, delivery: { type: 'none' }, enabled: true });
    if (pa?.id) {
      ids.personalAgent ??= pa.id;
      await api.try('POST', `/personal-agents/${pa.id}/schedules`, { schedule: { cadence: 'daily', at }, timezone: 'Europe/London', directive, enabled: true });
    }
  }

  // ── Run with Otto (offline stub drives it to the approval gate) ──────────
  const run1 = await api.try('POST', `${W}/runs`, { source_kind: 'channel', source_ref: 'slack-payments-1', seed_text: 'Add a short note describing the checkout rate limits to the docs.', mode: 'single_agent', repo_id: ids.repo, title: 'Document checkout rate limits' });
  const run2 = await api.try('POST', `${W}/runs`, { source_kind: 'channel', source_ref: 'slack-payments-2', seed_text: 'Add a CHANGELOG entry for the retry helper.', mode: 'single_agent', repo_id: ids.repo, title: 'Changelog: payment retries' });
  ids.runs = [run1?.id, run2?.id].filter(Boolean);

  // ── Proof pack ───────────────────────────────────────────────────────────
  const pp = await api.try('POST', `${W}/proof-packs`, { work_item_kind: 'manual', work_item_id: 'checkout-rate-limit', title: 'Checkout rate limiting' });
  if (pp?.id) {
    ids.proof = pp.id;
    await api.try('POST', `/proof-packs/${pp.id}/artifacts`, { kind: 'command', title: 'npm test', content: 'Test Suites: 12 passed, 12 total\nTests:       42 passed, 42 total\nTime:        3.1 s', status: 'passed' });
    await api.try('POST', `/proof-packs/${pp.id}/artifacts`, { kind: 'ci', title: 'CI · build + lint', content: 'All 6 checks passed', status: 'passed' });
    await api.try('POST', `/proof-packs/${pp.id}/artifacts`, { kind: 'diff', title: 'src/middleware/rateLimit.ts', content: '+ export function tokenBucket(…)\n+ router.use("/checkout", rateLimit)', status: 'info' });
    await api.try('POST', `/proof-packs/${pp.id}/evidence/api`, { title: 'GET /checkout/limits', method: 'GET', url: 'https://staging.acme.example/checkout/limits', status: 200 });
    await api.try('POST', `/proof-packs/${pp.id}/evidence/db`, { title: 'No duplicate orders', engine: 'mysql', query: 'SELECT COUNT(*) FROM orders GROUP BY idempotency_key HAVING COUNT(*) > 1', row_count: 0, sample: '0 rows' });
    await api.try('POST', `/proof-packs/${pp.id}/evidence/kafka`, { title: 'orders.events emitted', topic: 'orders.events', message_count: 3, sample: '{"order_id":1042,"status":"paid"}' });
    await api.try('POST', `/proof-packs/${pp.id}/pr-check`, { title: 'Rate limiting for checkout', description: 'All tests pass. Load test at 2k rps shows 429s after 60 req/min per client.' });
  }

  // ── Product story ────────────────────────────────────────────────────────
  const story = await api.try('POST', `${W}/product/drafts`, { title: 'Instant refunds for damaged orders' });
  const storyId = story?.id ?? story?.story?.id;
  if (storyId) {
    ids.story = storyId;
    await api.try('PATCH', `/product/stories/${storyId}/draft`, {
      title: 'Instant refunds for damaged orders',
      body_md: '## Problem\nCustomers wait 5–7 days for a refund when an order arrives damaged.\n\n## Proposal\nOffer an instant refund to store credit when a photo confirms the damage.\n\n## Acceptance criteria\n- Refund lands in under a minute\n- Fraud score above 0.8 routes to a human\n- The customer gets an email and a push notification\n',
    });
    await api.try('PATCH', `/product/stories/${storyId}`, { tags: 'refunds,payments' });
    for (const [text, rationale, category] of [
      ['Does store credit expire?', 'Affects liability on the balance sheet.', 'business'],
      ['Which carriers share damage photos via API?', 'Decides whether we can auto-verify.', 'technical'],
    ]) await api.try('POST', `/product/stories/${storyId}/questions`, { text, rationale, category });
    await api.try('POST', `/product/stories/${storyId}/notes`, { section: 'analysis', body: 'Refund volume is ~2% of orders; 60% of those are damage claims.' });
  }

  // ── Vault (OKF markdown on disk) ─────────────────────────────────────────
  const vaultDir = writeVault(join(home, 'Docs', 'platform'));
  const v = await api.try('POST', `${W}/vault/vaults`, { name: 'Platform Docs', root_path: vaultDir, okf: true });
  if (v?.id) {
    ids.vault = v.id;
    for (let i = 0; i < 60; i++) {
      const st = await api.try('GET', `${W}/vault/vaults/${v.id}/status`);
      if (st?.scan_state === 'idle' && st?.notes > 5) break;
      await sleep(300);
    }
  }

  // ── Design Hall ──────────────────────────────────────────────────────────
  const dp = await api.try('POST', '/design/projects', { workspace_id: ws.id, name: 'Checkout redesign', description: 'One-page checkout, saved cards and the refunds flow.', brand_kit_id: null });
  const art = async (format, studio, title, content, tags = []) => {
    const r = await api.try('POST', '/design/artifacts', { workspace_id: ws.id, project_id: dp?.id ?? null, format, studio, title, content, tags });
    return r?.artifact?.id ?? r?.id;
  };
  const kitJson = JSON.parse(readFileSync(join(import.meta.dirname, '../demo-design/brand-kit.json'), 'utf8'));
  ids.brand = await art('otto-brand', 'brand', 'Acme Brand Kit', JSON.stringify(kitJson, null, 2), ['brand']);
  ids.frame = await art('html', 'frames', 'One-page checkout', readFileSync(join(import.meta.dirname, '../demo-design/checkout.html'), 'utf8'), ['checkout']);
  ids.frame2 = await art('html', 'frames', 'Refund confirmation', readFileSync(join(import.meta.dirname, '../demo-design/refund.html'), 'utf8'), ['refunds']);
  ids.scene3d = await art('scene3d', '3d', 'Gift card hero', readFileSync(join(import.meta.dirname, '../demo-design/scene3d.json'), 'utf8'), ['launch']);
  ids.diagram = await art('mermaid', 'whiteboard', 'Checkout sequence', 'sequenceDiagram\n  participant C as Customer\n  participant W as Web\n  participant A as Checkout API\n  participant P as Payments\n  C->>W: Pay\n  W->>A: POST /checkout\n  A->>P: charge(card)\n  P-->>A: ok\n  A-->>W: order confirmed\n  W-->>C: Thank you!\n', ['checkout']);
  ids.site = await art('otto-site', 'site', 'Refunds launch site', JSON.stringify({ ...JSON.parse(readFileSync(join(import.meta.dirname, '../demo-design/site.json'), 'utf8')), ...(ids.brand ? { brand: `otto://design/${ids.brand}` } : {}) }, null, 2), ['launch']);
  if (ids.frame) {
    await api.try('POST', `/design/artifacts/${ids.frame}/versions`, { message: 'v2 — saved cards first', content: readFileSync(join(import.meta.dirname, '../demo-design/checkout.html'), 'utf8').replace('Pay now', 'Pay $148.00') });
    if (ids.brand) await api.try('POST', `/design/artifacts/${ids.frame}/links`, { rel: 'uses_tokens', dst_kind: 'artifact', dst_id: ids.brand });
    await api.try('PATCH', `/design/artifacts/${ids.frame}`, { status: 'review' });
  }
  for (const a of [ids.frame, ids.frame2, ids.site].filter(Boolean)) {
    for (const dir of ['rounded', 'rounded', 'bold']) await api.try('POST', '/design/signals', { artifact_id: a, kind: 'variant_accepted', payload: { direction: dir } });
  }
  await api.try('POST', '/design/learned/extract', { workspace_id: ws.id });
  const scene = join(repoRoot, 'ui/e2e/_agent_scene.json');
  if (existsSync(scene)) {
    await api.try('POST', `${W}/canvas/scenes`, { title: 'Place Order — services', section: 'Payments', doc: { type: 'otto-canvas', version: 1, format: 'excalidraw', source: readFileSync(scene, 'utf8') } });
  }

  // ── Skills Lab: a static review of a bundled skill ───────────────────────
  const lib = await api.try('GET', '/library/skills');
  const skillName = Array.isArray(lib) ? (lib.find((s) => /review/.test(s.name))?.name ?? lib[0]?.name) : null;
  if (skillName) {
    ids.skill = skillName;
    await api.try('POST', `${W}/skill-reviews`, { skill_name: skillName, skill_source: 'library', providers: [], agent_mode: 'static' });
  }

  // ── Channels (disabled, placeholder tokens — nothing connects) ───────────
  await api.try('PUT', `${W}/integrations/slack`, { enabled: false, bot_token: 'xoxb-demo-placeholder', app_token: 'xapp-demo-placeholder', allowed_users: 'maya,noah', agent_reply: true, reply_instructions: 'Be brief. Link the PR.', channel_id: 'C0PAYMENTS', preferred_cli: '' });
  await api.try('PUT', `${W}/integrations/telegram`, { enabled: false, bot_token: '000000:demo-placeholder', allowed_users: 'maya', agent_reply: true, reply_instructions: 'Short answers.', channel_id: '', preferred_cli: '' });

  // ── Insights (report files the insights skill would write) ───────────────
  writeInsights(join(dataDir, 'insights'));

  // Let Run with Otto reach its approval gate, then approve the first run.
  await sleep(8000);
  if (run1?.id) {
    for (let i = 0; i < 30; i++) {
      const r = await api.try('GET', `/runs/${run1.id}`);
      if (r?.status === 'awaiting_approval' || r?.run?.status === 'awaiting_approval') {
        await api.try('POST', `/runs/${run1.id}/approve`, { decision: 'approve' });
        break;
      }
      await sleep(1500);
    }
  }
  // ── Git: local AI review + findings (offline seed endpoints) ─────────────
  const rev = await api.try('POST', `${W}/__e2e/review`, {
    repo_id: ids.repo, pr_number: 0,
    agents: [
      { name: 'Correctness', provider: 'claude', model: '', status: 'done', note: '2 findings', comment_count: 2, session_id: null, findings: [
        { path: 'src/middleware/rateLimit.ts', line: 4, severity: 'bug', body: 'Buckets are never evicted — memory grows with every client key.' },
        { path: 'src/router.ts', line: 6, severity: 'warn', body: 'req.ip is the proxy address behind the load balancer; use X-Forwarded-For.' },
      ] },
      { name: 'Security', provider: 'codex', model: '', status: 'done', note: '1 finding', comment_count: 1, session_id: null, findings: [
        { path: 'src/router.ts', line: 6, severity: 'warn', body: 'Rate-limit key can be spoofed via a forged header.' },
      ] },
      { name: 'Performance', provider: 'claude', model: '', status: 'running', note: 'Profiling the hot path…', comment_count: 0, session_id: null, findings: [] },
      { name: 'Tests', provider: 'codex', model: '', status: 'pending', note: '', comment_count: 0, session_id: null, findings: [] },
    ],
  });
  ids.review = rev?.id ?? rev?.review?.id;
  if (ids.review) {
    for (const f of [
      { path: 'src/middleware/rateLimit.ts', line: 4, line_end: 12, severity: 'high', category: 'correctness', title: 'Unbounded bucket map', body: 'Buckets are never evicted, so memory grows with every distinct client key.', evidence: 'const buckets = new Map<string, …>()', reasoning: 'Keys come from client IPs; a crawler creates millions.', suggested_fix: 'Evict buckets idle for 60 s on an interval.', status: 'accepted', requires_human_approval: false },
      { path: 'src/router.ts', line: 6, line_end: 6, severity: 'medium', category: 'security', title: 'Spoofable rate-limit key', body: 'The key trusts a client-supplied header.', evidence: "allow(req.ip)", reasoning: 'Behind the proxy req.ip is constant.', suggested_fix: 'Use the proxy-verified client address.', status: 'open', requires_human_approval: true },
      { path: 'test/rateLimit.test.ts', line: 3, line_end: 6, severity: 'low', category: 'tests', title: 'No refill test', body: 'Only the burst case is covered.', evidence: 'refillPerSec: 0', reasoning: 'Refill math is the tricky part.', suggested_fix: 'Add a fake-timer refill test.', status: 'open', requires_human_approval: false },
    ]) await api.try('POST', `${W}/__e2e/findings`, { review_id: ids.review, repo_id: ids.repo, ...f });
  }

  // Mission Control projects everything above into the work graph.
  await api.try('POST', `${W}/workgraph/backfill`, {});
  await clearCredentialNotices(api);
  return ids;
}

/** Drop only the "re-login needed" notices the fake HOME provokes. */
export async function clearCredentialNotices(api) {
  const list = await api.try('GET', '/notifications');
  for (const n of Array.isArray(list) ? list : []) {
    if (n.kind === 'credential' || String(n.source_key ?? '').startsWith('agent_auth')) await api.try('DELETE', `/notifications/${n.id}`);
  }
}

/** Copy a file only if it exists (helper for optional fixtures). */
export function copyIfExists(src, dst) {
  if (existsSync(src)) copyFileSync(src, dst);
}
