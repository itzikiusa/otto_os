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

  // ── agent sessions (scripted stand-ins + rich transcripts for Threads) ──
  const trDir = join(dataDir, 'tour-transcripts');
  mkdirSync(trDir, { recursive: true });
  const trRate = join(trDir, '00000000-0000-4000-8000-000000000101.jsonl');
  const trFlaky = join(trDir, 'rollout-2026-10-31T10-12-00-00000000-0000-4000-8000-000000000102.jsonl');
  const trReview = join(trDir, '00000000-0000-4000-8000-000000000103.jsonl');
  const trNotes = join(trDir, '00000000-0000-4000-8000-000000000104.jsonl');

  writeFileSync(trRate, [
    { type: 'user', uuid: 'rl-u1', timestamp: '2026-10-31T09:40:00Z', sessionId: '00000000-0000-4000-8000-000000000101', cwd: repoDir, message: { role: 'user', content: 'Add token-bucket rate limiting (60 req/min, burst 60) to `/checkout` in `src/middleware/rateLimit.ts`, verify traffic volume in `shopdb.orders`, and notify the cart worker when ready.' } },
    { type: 'assistant', uuid: 'rl-a1', requestId: 'rl-r1', timestamp: '2026-10-31T09:40:06Z', sessionId: '00000000-0000-4000-8000-000000000101', cwd: repoDir, message: { id: 'm-rl-1', model: 'claude-opus-4-8', content: [
      { type: 'thinking', thinking: 'Check peak per-region order rate in shopdb first so 60 req/min has generous headroom, then wire tokenBucket with idle eviction.' },
      { type: 'tool_use', id: 'toolu_rl_db', name: 'mcp__otto__otto_db_query', input: { connection: 'shop · mariadb', sql: 'SELECT region, COUNT(*) AS orders FROM shopdb.orders GROUP BY region ORDER BY orders DESC' } },
      { type: 'tool_use', id: 'toolu_rl_edit', name: 'Edit', input: { file_path: `${repoDir}/src/middleware/rateLimit.ts`, old_string: 'const buckets = new Map();', new_string: 'const buckets = new Map<string, { tokens: number; at: number }>();' } },
    ], usage: { input_tokens: 14200, output_tokens: 890, cache_read_input_tokens: 28400 } } },
    { type: 'user', uuid: 'rl-u2', timestamp: '2026-10-31T09:40:10Z', sessionId: '00000000-0000-4000-8000-000000000101', cwd: repoDir, message: { role: 'user', content: [
      { type: 'tool_result', tool_use_id: 'toolu_rl_db', content: JSON.stringify({ columns: ['region', 'orders'], rows: [['us-east', 42], ['eu-west', 31], ['eu-central', 19]], row_count: 3, duration_ms: 8 }) },
      { type: 'tool_result', tool_use_id: 'toolu_rl_edit', content: 'Updated src/middleware/rateLimit.ts (+14 -1)' },
    ] } },
    { type: 'assistant', uuid: 'rl-a2', requestId: 'rl-r2', timestamp: '2026-10-31T09:40:24Z', sessionId: '00000000-0000-4000-8000-000000000101', cwd: repoDir, message: { id: 'm-rl-2', model: 'claude-opus-4-8', content: [
      { type: 'text', text: 'Implemented `tokenBucket({ capacity: 60, refillPerSec: 1 })` in `src/middleware/rateLimit.ts:1` with 60s idle bucket eviction and `429 Retry-After: 1` on `/checkout`.\n\n| Check | Result |\n| :--- | :--- |\n| Burst + refill suite | **32 passed** |\n| Idle key eviction | Bounded (`60s` TTL) |\n| Peak regional traffic | `42` orders (`us-east`) |' },
    ], usage: { input_tokens: 6400, output_tokens: 520 } } },
  ].map((l) => JSON.stringify(l)).join('\n') + '\n');

  writeFileSync(trFlaky, [
    { timestamp: '2026-10-31T09:41:00Z', type: 'session_meta', payload: { session_id: '00000000-0000-4000-8000-000000000102', id: '00000000-0000-4000-8000-000000000102', cwd: repoDir, cli_version: '0.150.0', model: 'gpt-5.5-codex' } },
    { timestamp: '2026-10-31T09:41:00Z', type: 'turn_context', payload: { cwd: repoDir, model: 'gpt-5.5-codex' } },
    { timestamp: '2026-10-31T09:41:01Z', type: 'event_msg', payload: { type: 'task_started', turn_id: 'T1' } },
    { timestamp: '2026-10-31T09:41:01Z', type: 'response_item', payload: { type: 'message', role: 'user', content: [{ type: 'input_text', text: 'Fix the flaky cart totals rounding failure in `src/cart/totals.ts` and re-run 50 iterations.' }] } },
    { timestamp: '2026-10-31T09:41:14Z', type: 'event_msg', payload: { type: 'item_completed', turn_id: 'T1', item: { type: 'CommandExecution', id: 'e1', command: ['/bin/zsh', '-lc', 'npm test -- cart --runInBand'], cwd: `file://${repoDir}`, status: 'completed', aggregated_output: 'PASS test/cart.test.ts (50/50 runs)\n', exit_code: 0 } } },
    { timestamp: '2026-10-31T09:41:20Z', type: 'event_msg', payload: { type: 'item_completed', turn_id: 'T1', item: { type: 'AgentMessage', id: 'm1', content: [{ type: 'Text', text: 'Fixed integer-cent rounding in `src/cart/totals.ts:1` and pinned the test clock:\n\n```ts\nexport function total(items: { price: number; qty: number }[], taxBps = 2000) {\n  const netCents = items.reduce((sum, i) => sum + i.price * i.qty, 0);\n  return Math.round((netCents * (10_000 + taxBps)) / 10_000);\n}\n```\n\nAll **50 consecutive runs** passed with zero drift.' }] } } },
    { timestamp: '2026-10-31T09:41:21Z', type: 'event_msg', payload: { type: 'task_complete', turn_id: 'T1', last_agent_message: 'All 50 consecutive runs passed.' } },
  ].map((l) => JSON.stringify(l)).join('\n') + '\n');

  writeFileSync(trReview, [
    { type: 'user', uuid: 'rv-u1', timestamp: '2026-10-31T09:42:00Z', sessionId: '00000000-0000-4000-8000-000000000103', cwd: repoDir, message: { role: 'user', content: 'Review PR #214 (refunds) for security and correctness.' } },
    { type: 'assistant', uuid: 'rv-a1', requestId: 'rv-r1', timestamp: '2026-10-31T09:42:18Z', sessionId: '00000000-0000-4000-8000-000000000103', cwd: repoDir, message: { id: 'm-rv-1', model: 'claude-sonnet-4-8', content: [{ type: 'text', text: 'Reviewed 7 changed files (`+312 −48`). Two actionable findings:\n\n- **High** (`src/refunds/handler.ts:88`): post-refund redirect URL is not validated against allowlist.\n- **Medium** (`src/refunds/api.ts:41`): `POST /refund` lacks CSRF token verification.' }], usage: { input_tokens: 9200, output_tokens: 410 } } },
  ].map((l) => JSON.stringify(l)).join('\n') + '\n');

  writeFileSync(trNotes, [
    { type: 'user', uuid: 'rn-u1', timestamp: '2026-10-31T09:43:00Z', sessionId: '00000000-0000-4000-8000-000000000104', cwd: repoDir, message: { role: 'user', content: 'Draft release notes for 1.5.0' } },
    { type: 'assistant', uuid: 'rn-a1', requestId: 'rn-r1', timestamp: '2026-10-31T09:43:15Z', sessionId: '00000000-0000-4000-8000-000000000104', cwd: repoDir, message: { id: 'm-rn-1', model: 'claude-opus-4-8', content: [{ type: 'text', text: '## Acme Checkout 1.5.0\n\n- **Payments:** declined cards retry with exponential backoff; hard declines fail fast.\n- **Cart:** regional tax rules are memoized per region.\n- **API:** token-bucket rate limiter (`60 req/min`) with `Retry-After` headers.' }], usage: { input_tokens: 5800, output_tokens: 360 } } },
  ].map((l) => JSON.stringify(l)).join('\n') + '\n');

  const sessions = [
    ['work', 'claude', 'Add rate limiting to checkout', 'claude-opus-4-8', trRate, false],
    ['flaky', 'codex', 'Fix the flaky cart totals test', 'gpt-5.5-codex', trFlaky, true],
    ['review', 'claude', 'Review PR #214 — refunds', 'claude-sonnet-4-8', trReview, false],
    ['notes', 'claude', 'Draft release notes for 1.5.0', 'claude-opus-4-8', trNotes, false],
  ];
  ids.sessions = [];
  for (const [mode, provider, title, model, trPath, delegated] of sessions) {
    agentMode(fakeBin, mode);
    const meta = {
      origin: 'manual',
      e2e_transcript_path: trPath,
      ...(delegated && ids.sessions[0] ? { delegated_by: ids.sessions[0] } : {}),
    };
    const s = await api.post(`${W}/sessions`, { kind: 'agent', provider, title, cwd: repoDir, model, meta });
    ids.sessions.push(s.id);
    await sleep(900);
  }
  if (ids.sessions[0] && ids.sessions[1]) {
    await api.try('POST', `/sessions/${ids.sessions[1]}/message`, {
      text: 'Rate-limit middleware landed on feat/rate-limit — safe to rebase cart totals.',
      submit: false,
      from_session_id: ids.sessions[0],
    });
  }
  const shell = await api.post(`${W}/sessions`, { kind: 'agent', provider: 'shell', title: 'checkout · dev server', cwd: repoDir, meta: { origin: 'manual' } });
  ids.shell = shell.id;
  await sleep(600);
  await api.try('POST', `/sessions/${shell.id}/input`, {
    text: `clear; for i in $(seq 1 3000); do printf '\\033[2m%s\\033[0m  \\033[32mGET\\033[0m /api/checkout/%d  \\033[36m200\\033[0m  %dms\\n' "$(date +%H:%M:%S)" $((RANDOM%900+100)) $((RANDOM%40+8)); sleep 0.7; done`,
    submit: true,
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
  const kube = join(home, 'kubeconfig-demo.yaml');
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
    notify_on_change: true, destination: { type: 'none' }, enabled: true,
  });
  await api.try('POST', `${W}/scheduled-tasks`, {
    name: 'Weekly dependency audit', prompt: 'Audit outdated and vulnerable dependencies.', provider: 'codex',
    schedule: { cadence: 'weekly', at: '08:00', weekday: 0 }, timezone: 'Europe/London', sandbox: 'worktree', max_retries: 1,
    notify_on_change: true, destination: { type: 'none' }, enabled: true,
  });
  const st3 = await api.try('POST', `${W}/scheduled-tasks`, {
    name: 'Flaky test hunt', prompt: 'Find tests that failed then passed on retry this week.', provider: 'claude',
    schedule: { cadence: 'cron', expr: '0 18 * * 5' }, timezone: 'Europe/London', sandbox: 'worktree', max_retries: 2,
    notify_on_change: false, destination: { type: 'none' }, enabled: true,
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
