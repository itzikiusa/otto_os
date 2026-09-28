import { expect, test, type Page, type Route } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { domCount, longTasks, requestLog, watchLongTasks } from './perf';

// ─────────────────────────────────────────────────────────────────────────────
// Infra viewers — perf regression gates (I7, GAPS_TO_9_5 §6). Budgets are DOM /
// request COUNTS first, timings second, so they don't flake.
//
// No live Kafka / cluster / bastion is needed: the rows are real (seeded over
// the API) and the heavy payloads are served by `page.route` mocks, so every
// gate measures the UI's own cost at scale:
//   • Kafka: a 5,000-message peek mounts ≤ 150 table rows (SC-08); 1k groups,
//     a 12k-offset group and 3k schema subjects mount ≤ 150 rows each (SC-22,
//     SC-03); the overview never has > 1 `/metrics` in flight (SC-01).
//   • SFTP: a capped (20k + `truncated`) listing mounts ≤ 200 rows and paints
//     < 300 ms after the response lands (SC-04).
//   • S3: 100k objects mount ≤ 200 rows; an auto-refresh tick re-reads ONE
//     page and keeps the loaded pages + scroll (I9).
//   • K8s: typing into the filter over 5k pods makes no long task, ≤ 150 rows
//     are mounted; 10 rapid `j` presses cost ≤ 3 `/resource` loads (SC-05/12).
// Not covered here: k8s logs follow at 20k lines (needs a streaming mock).
// Unit-level halves: ui/unit/infraViewers.test.ts (TableWindow at 5k/12k/50k,
// manifest scalar clip) and the crates (batched watermarks on a mocked client,
// pooled group consumer, proxy pass-through, SFTP cap, monitor write-on-change).
// ─────────────────────────────────────────────────────────────────────────────

test.use({ serviceWorkers: 'block' });
test.describe.configure({ mode: 'serial', timeout: 120_000 });

const V1 = '/api/v1';
const CLUSTER = `perf-kafka-${Date.now().toString(36)}`;
const TOPIC = 'perf-topic';
let workspaceId = '';
let sshConnId = '';
const SSH_NAME = `perf-ssh-${Date.now().toString(36)}`;
let awsAccountId = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  try {
    workspaceId = await seedWorkspace(ctx, base);
    const r = await ctx.post(`${base}${V1}/workspaces/${workspaceId}/brokers/clusters`, {
      data: {
        name: CLUSTER,
        // Never dialled: every data route below is mocked.
        bootstrap_servers: 'perf-broker.invalid:9092',
        security_protocol: 'plaintext',
        environment: 'dev',
        read_only: false,
      },
    });
    expect(r.ok(), `seed cluster → ${r.status()} ${await r.text()}`).toBeTruthy();
    const c = await ctx.post(`${base}${V1}/workspaces/${workspaceId}/connections`, {
      data: { name: SSH_NAME, kind: 'ssh', params: { host: 'fixture.invalid' } },
    });
    expect(c.ok(), `seed ssh connection → ${c.status()} ${await c.text()}`).toBeTruthy();
    sshConnId = ((await c.json()) as { id: string }).id;
    const a = await ctx.post(`${base}${V1}/aws/accounts`, {
      data: {
        name: `perf-aws-${Date.now().toString(36)}`,
        auth_mode: 'access_keys',
        region: 'eu-west-1',
        access_key_id: 'AKIAE2EFAKEKEY000000',
        secret_access_key: 'e2e-fake-secret-not-real-0000000000000000',
        environment: 'dev',
        color: '#3b82f6',
      },
    });
    expect(a.ok(), `seed aws account → ${a.status()} ${await a.text()}`).toBeTruthy();
    awsAccountId = ((await a.json()) as { id: string }).id;
  } finally {
    await ctx.dispose();
  }
});

test.beforeEach(async ({ page, isMobile }, testInfo) => {
  test.skip(isMobile || testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript((ws) => {
    localStorage.setItem('otto_workspace', ws);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

// ── Kafka fixtures ────────────────────────────────────────────────────────────

const PARTS = 12;
const json = (route: Route, body: unknown, status = 200): Promise<void> =>
  route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });

function messages(n: number) {
  return Array.from({ length: n }, (_, i) => ({
    partition: i % PARTS,
    offset: 10_000 + Math.floor(i / PARTS),
    timestamp_ms: 1_700_000_000_000 + i * 1000,
    key: { format: 'string', text: `key-${i}` },
    value: { format: 'json', text: `{"i":${i},"payload":"${'x'.repeat(64)}"}` },
    headers: [],
    size_bytes: 120 + (i % 50),
  }));
}

function groupDetail(id: string, offsets: number) {
  return {
    group_id: id,
    state: 'Stable',
    protocol_type: 'consumer',
    protocol: 'range',
    members: [{ member_id: 'm-1', client_id: 'svc', host: '/10.0.0.1', assignments: [] }],
    offsets: Array.from({ length: offsets }, (_, i) => ({
      topic: `orders-${Math.floor(i / 600)}`,
      partition: i % 600,
      current_offset: 1000 + i,
      high_watermark: 1000 + i + (i % 97),
      lag: i % 97,
    })),
    total_lag: 0,
  };
}

interface KafkaMockOpts {
  /** Delay (ms) before each `/metrics` answer — a slow cluster-wide sweep. */
  metricsDelayMs?: number;
}

/** Serve every `/brokers/clusters/{id}/…` data route from fixtures. */
async function mockKafka(page: Page, opts: KafkaMockOpts = {}): Promise<void> {
  await page.route(/\/api\/v1\/brokers\/clusters\/[^/]+\/(.+)$/, async (route) => {
    const req = route.request();
    const path = new URL(req.url()).pathname.replace(/^.*\/brokers\/clusters\/[^/]+\//, '');
    const method = req.method();
    if (path === 'test') return json(route, { ok: true, latency_ms: 1, message: 'ok', broker_count: 1 });
    if (path === 'overview') return json(route, { message: 'mocked' }, 502);
    if (path === 'metrics') {
      if (opts.metricsDelayMs) await new Promise((r) => setTimeout(r, opts.metricsDelayMs));
      return json(route, {
        throughput: [],
        messages_per_sec: 0,
        total_messages: 0,
        brokers: [],
        prometheus_available: false,
        sampled_at: new Date().toISOString(),
      });
    }
    if (path === 'topics' && method === 'GET') {
      return json(route, [
        { name: TOPIC, partitions: PARTS, replication_factor: 1, message_count: 5_000_000, cleanup_policy: 'delete', internal: false },
      ]);
    }
    if (path === 'topics/stats') return json(route, {});
    if (path === `topics/${TOPIC}` && method === 'GET') {
      return json(route, {
        name: TOPIC,
        internal: false,
        partitions: Array.from({ length: PARTS }, (_, id) => ({
          id, leader: 1, replicas: [1], isr: [1], low: 0, high: 1_000_000, message_count: 1_000_000,
        })),
        configs: [],
        message_count: 12_000_000,
      });
    }
    if (path === `topics/${TOPIC}/consume`) {
      return json(route, {
        messages: messages(5000),
        partitions: Array.from({ length: PARTS }, (_, partition) => ({ partition, low: 0, high: 20_000 })),
        truncated: false,
      });
    }
    if (path === 'groups') {
      return json(route, Array.from({ length: 1000 }, (_, i) => ({
        group_id: `group-${String(i).padStart(4, '0')}`, state: 'Stable', protocol_type: 'consumer', members: 1 + (i % 3),
      })));
    }
    const g = /^groups\/([^/]+)$/.exec(path);
    if (g) return json(route, groupDetail(decodeURIComponent(g[1]), 12_000));
    if (path === 'schema-registry/subjects') {
      return json(route, Array.from({ length: 3000 }, (_, i) => ({
        subject: `com.example.events.Subject${i}-value`, version: 1 + (i % 4), id: i + 1, schema_type: 'AVRO',
        schema: JSON.stringify({ type: 'record', name: `S${i}`, fields: [{ name: 'id', type: 'long' }] }),
      })));
    }
    return json(route, { error: { code: 'not_found', message: `unmocked ${path}` } }, 404);
  });
}

async function openCluster(page: Page, tab?: string): Promise<void> {
  await page.goto('/#/brokers');
  await expect(page.locator('.brokers-page')).toBeVisible({ timeout: 30_000 });
  await page.locator('.cluster .cn', { hasText: CLUSTER }).first().click();
  await expect(page.locator('.cluster-head .name')).toBeVisible({ timeout: 15_000 });
  if (tab) await page.locator('.tabs button', { hasText: tab }).first().click();
}

/** Scroll `sel` to its end and wait two frames (the window follows on rAF). */
async function scrollToEnd(page: Page, sel: string): Promise<void> {
  await page.locator(sel).evaluate((el) => (el.scrollTop = el.scrollHeight));
  await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(() => r(null)))));
}

// ── Kafka gates ───────────────────────────────────────────────────────────────

test('Kafka: a 5,000-message peek mounts ≤ 150 table rows and still reaches the last message', async ({ page }) => {
  await mockKafka(page);
  await openCluster(page, 'Topics');
  await page.locator('tr', { hasText: TOPIC }).first().click();
  await page.getByRole('button', { name: 'Peek', exact: true }).click();
  const rows = page.locator('.msg-list tbody tr[data-message-key]');
  await expect(rows.first()).toBeVisible({ timeout: 15_000 });
  const mounted = await domCount(page, '.msg-list tbody tr[data-message-key]');
  expect(mounted, 'windowed message table').toBeLessThanOrEqual(150);
  expect(mounted).toBeGreaterThan(5);
  // The spacers keep the full scroll height; the last message is reachable.
  await scrollToEnd(page, '.msg-list');
  const last = messages(5000).at(-1)!;
  await expect(page.locator(`.msg-list tr[data-message-key="${last.partition}-${last.offset}"]`)).toBeVisible();
  expect(await domCount(page, '.msg-list tbody tr[data-message-key]')).toBeLessThanOrEqual(150);
});

test('Kafka: 1,000 groups and a 12,000-offset group mount ≤ 150 rows each', async ({ page }) => {
  await mockKafka(page);
  await openCluster(page, 'Consumer Groups');
  await expect(page.locator('.grow-row').first()).toBeVisible({ timeout: 15_000 });
  expect(await domCount(page, '.grow-row'), 'windowed group list').toBeLessThanOrEqual(150);
  // The list opens on the first group (list/detail rule) — its offsets table is windowed too.
  const offsetRows = '.offsets-wrap tbody tr:not(.tw-spacer)';
  await expect(page.locator(offsetRows).first()).toBeVisible({ timeout: 15_000 });
  expect(await domCount(page, offsetRows), 'windowed offsets table').toBeLessThanOrEqual(150);
  await scrollToEnd(page, '.offsets-wrap');
  expect(await domCount(page, offsetRows)).toBeLessThanOrEqual(150);
  // Scrolling the group list reaches the last group.
  await scrollToEnd(page, '.groups .list');
  await expect(page.locator('.grow-row', { hasText: 'group-0999' })).toBeVisible();
  expect(await domCount(page, '.grow-row')).toBeLessThanOrEqual(150);
});

test('Kafka: 3,000 schema subjects mount ≤ 150 rows', async ({ page }) => {
  await mockKafka(page);
  await openCluster(page, 'Schema Registry');
  await expect(page.locator('.srow').first()).toBeVisible({ timeout: 15_000 });
  expect(await domCount(page, '.srow'), 'windowed subject list').toBeLessThanOrEqual(150);
  await scrollToEnd(page, '.schema .list');
  await expect(page.locator('.srow', { hasText: 'Subject2999-value' })).toBeVisible();
});

test('Kafka: the overview never has more than one /metrics in flight (slow sweep)', async ({ page }) => {
  await mockKafka(page, { metricsDelayMs: 5000 });
  const log = requestLog(page, /\/brokers\/clusters\/[^/]+\/metrics/);
  await openCluster(page);
  await page.waitForTimeout(20_000);
  log.stop();
  const s = log.stats();
  expect(s.count, s.paths.join('\n')).toBeGreaterThanOrEqual(1);
  expect(s.maxInFlight, 'chained poll: a slow sweep never stacks').toBeLessThanOrEqual(1);
  // 5 s answer + 4 s cadence ⇒ at most ~3 sweeps in 20 s (an interval would be 5+).
  expect(s.count).toBeLessThanOrEqual(3);
});

// ── SFTP gate ─────────────────────────────────────────────────────────────────

test('SFTP: a capped 20k listing shows the truncation note, mounts ≤ 200 rows and paints fast', async ({ page }) => {
  const entries = Array.from({ length: 20_000 }, (_, i) => ({
    name: `file-${String(i).padStart(5, '0')}.log`,
    kind: 'file',
    size: 1000 + i,
    mtime: 'Jun 20 12:00',
    perms: '-rw-r--r--',
    symlink_target: null,
  }));
  // What the daemon now answers for a 50k-entry directory (MAX_LIST_ENTRIES).
  await page.route(`**/api/v1/connections/${sshConnId}/sftp/list**`, (route) =>
    json(route, { path: '/var/spool/big', entries, truncated: true }),
  );
  await page.route(`**/api/v1/connections/${sshConnId}/sftp/transfers**`, (route) => json(route, []));
  await page.goto('/#/connections');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 15_000 });
  await page.locator('.conn-row', { hasText: SSH_NAME }).first().click({ button: 'right' });
  const listed = page.waitForResponse((r) => r.url().includes(`/connections/${sshConnId}/sftp/list`));
  await page.getByRole('menuitem', { name: 'Browse files (SFTP)', exact: true }).click();
  await listed;
  // Response landed → first row painted (the render cost, not the network).
  const paintMs = await page.evaluate(async () => {
    const t0 = performance.now();
    while (!document.querySelector('.sftp-row.entry')) {
      await new Promise((r) => requestAnimationFrame(() => r(null)));
      if (performance.now() - t0 > 10_000) break;
    }
    return performance.now() - t0;
  });
  expect(paintMs, 'listing paints within budget').toBeLessThan(300);
  await expect(page.locator('.list-trunc')).toContainText('first 20,000 entries');
  expect(await domCount(page, '.sftp-row.entry'), 'windowed listing').toBeLessThanOrEqual(200);
});

// ── S3 gate ───────────────────────────────────────────────────────────────────

const S3_PAGE = 25_000;
const S3_PAGES = 4;

async function mockS3(page: Page): Promise<void> {
  await page.route('**/api/v1/aws/status', (route) =>
    json(route, { installed: true, version: 'aws-cli/2.0.0', path: '/usr/local/bin/aws', install: { tool: 'aws', state: 'idle', log_tail: '' } }),
  );
  await page.route(`**/api/v1/aws/accounts/${awsAccountId}/s3/buckets`, (route) =>
    json(route, { buckets: [{ name: 'perf-bucket', creation_date: '2026-01-01T00:00:00Z' }] }),
  );
  await page.route(`**/api/v1/aws/accounts/${awsAccountId}/s3/buckets/perf-bucket/objects**`, (route) => {
    const token = new URL(route.request().url()).searchParams.get('token');
    const n = token ? Number(token.slice(1)) : 0;
    const objects = Array.from({ length: S3_PAGE }, (_, i) => ({
      key: `obj-${String(n * S3_PAGE + i).padStart(6, '0')}`,
      size: 100 + i,
      last_modified: '2026-01-01T00:00:00Z',
    }));
    const last = n + 1 >= S3_PAGES;
    return json(route, { prefixes: [], objects, is_truncated: !last, next_token: last ? null : `p${n + 1}` });
  });
}

test('S3: 100k objects mount ≤ 200 rows; auto-refresh re-reads one page and keeps the loaded ones', async ({ page }) => {
  await mockS3(page);
  await page.goto(`/#/aws/${awsAccountId}/s3/perf-bucket`);
  await expect(page.locator('tr.trow').first()).toBeVisible({ timeout: 30_000 });
  for (let i = 1; i < S3_PAGES; i++) {
    const more = page.locator('.more-row button');
    await expect(more).toBeEnabled();
    await more.click();
    await expect(page.locator('.more-row button')).toHaveCount(i + 1 < S3_PAGES ? 1 : 0, { timeout: 15_000 });
  }
  expect(await domCount(page, 'tr.trow'), 'windowed object table').toBeLessThanOrEqual(200);
  await scrollToEnd(page, '.tbl-wrap');
  await expect(page.locator('tr.trow', { hasText: 'obj-099999' })).toBeVisible();
  const scrolled = await page.locator('.tbl-wrap').evaluate((el) => el.scrollTop);

  const log = requestLog(page, /\/s3\/buckets\/perf-bucket\/objects/);
  await page.getByRole('checkbox', { name: 'Auto', exact: true }).check();
  await page.waitForTimeout(12_500); // one 10 s (±10 %) tick; the next is ≥ 9 s later
  log.stop();
  const s = log.stats();
  expect(s.count, s.paths.join('\n')).toBe(1);
  expect(s.paths[0]).not.toContain('token=');
  // Loaded pages and the scroll position survive the tick.
  await expect(page.locator('tr.trow', { hasText: 'obj-099999' })).toBeVisible();
  expect(await page.locator('.tbl-wrap').evaluate((el) => el.scrollTop)).toBeGreaterThan(scrolled / 2);
  expect(await domCount(page, 'tr.trow')).toBeLessThanOrEqual(200);
});

// ── Kubernetes gates ──────────────────────────────────────────────────────────

const K8S_ID = 'perf-k8s';

async function mockK8s(page: Page): Promise<void> {
  const rows = Array.from({ length: 5000 }, (_, i) => ({
    name: `pod-${String(i).padStart(4, '0')}`,
    namespace: `ns-${i % 5}`,
    kind: 'Pod',
    status: 'Running',
    ready: '1/1',
    restarts: i % 7,
    age_seconds: 3600 + i,
    node: `node-${i % 20}`,
    ip: `10.0.${Math.floor(i / 250)}.${i % 250}`,
    images: ['registry.example.com/app:1.0'],
    labels: { app: `app-${i % 50}` },
    extra: {},
    health: 'ok',
  }));
  const access = (route: Route) => {
    const u = new URL(route.request().url());
    return json(route, {
      kind: 'k8s_cluster',
      resource_id: K8S_ID,
      user_id: 'root',
      child: u.searchParams.get('child'),
      mode: 'legacy',
      operations: {},
    });
  };
  await page.route(`**/api/v1/access/k8s_cluster/${K8S_ID}/capabilities**`, access);
  await page.route(/\/api\/v1\/k8s\/(.*)$/, (route) => {
    const u = new URL(route.request().url());
    const path = u.pathname.replace(/^.*\/api\/v1\/k8s\//, '');
    const idle = (tool: string) => ({ tool, state: 'idle', log_tail: '' });
    if (path === 'status') {
      return json(route, {
        kubectl: { installed: true, version: 'v1.30.0', path: '/usr/local/bin/kubectl' },
        k9s: { installed: false },
        install: { kubectl: idle('kubectl'), k9s: idle('k9s') },
      });
    }
    if (path === 'clusters') {
      return json(route, [{
        id: K8S_ID, name: 'perf-k8s', source: 'kubeconfig', context_name: 'perf', environment: 'dev',
        created_by: 'root', created_at: '2026-01-01T00:00:00Z', updated_at: '2026-01-01T00:00:00Z',
        default_namespace: '', known_namespaces: [],
      }]);
    }
    if (path === `clusters/${K8S_ID}/capabilities`) {
      return json(route, { metrics_server: false, argo_rollouts: false, argocd: false, checked_at: '2026-01-01T00:00:00Z' });
    }
    if (path === `clusters/${K8S_ID}/namespaces`) {
      return json(route, { namespaces: Array.from({ length: 5 }, (_, i) => ({ name: `ns-${i}`, status: 'Active', age_seconds: 1 })) });
    }
    if (path === `clusters/${K8S_ID}/resources`) return json(route, { kind: 'pods', items: rows, has_metrics: false });
    if (path === `clusters/${K8S_ID}/resource`) {
      const name = u.searchParams.get('name') ?? '';
      return json(route, {
        manifest: { apiVersion: 'v1', kind: 'Pod', metadata: { name, namespace: u.searchParams.get('ns') }, spec: { containers: [{ name: 'app', image: 'app:1' }] } },
        describe: `Name: ${name}`,
        events: [],
      });
    }
    return json(route, { error: { code: 'not_found', message: `unmocked k8s ${path}` } }, 404);
  });
  // Keep the 10 s auto-refresh out of the measurements.
  await page.addInitScript(() => localStorage.setItem('otto_k8s_autorefresh', '0'));
}

test('K8s: typing a filter over 5k pods makes no long task and mounts ≤ 150 rows', async ({ page }) => {
  await mockK8s(page);
  await page.goto(`/#/kubernetes/${K8S_ID}/pods`);
  const row = page.getByTestId('k8s-row');
  await expect(row.first()).toBeVisible({ timeout: 30_000 });
  expect(await domCount(page, '[data-testid="k8s-row"]'), 'virtualized table').toBeLessThanOrEqual(150);
  const filter = page.getByTestId('k8s-filter');
  await filter.click();
  await watchLongTasks(page);
  await filter.pressSequentially('pod-49', { delay: 40 });
  await expect(row.first()).toContainText('pod-49');
  const lt = await longTasks(page);
  expect(lt, `long tasks while filtering: ${lt.map((d) => d.toFixed(0)).join(', ')} ms`).toHaveLength(0);
  expect(await domCount(page, '[data-testid="k8s-row"]')).toBeLessThanOrEqual(150);
});

test('K8s: 10 rapid j presses in the drawer cost ≤ 3 /resource loads', async ({ page }) => {
  await mockK8s(page);
  await page.goto(`/#/kubernetes/${K8S_ID}/pods`);
  const row = page.getByTestId('k8s-row');
  await expect(row.first()).toBeVisible({ timeout: 30_000 });
  await row.first().click();
  await expect(page.getByTestId('k8s-drawer')).toBeVisible({ timeout: 15_000 });
  await page.waitForTimeout(800); // the first drawer's own load settles
  const log = requestLog(page, /\/k8s\/clusters\/[^/]+\/resource\?/);
  for (let i = 0; i < 10; i++) {
    await page.keyboard.press('j');
    await page.waitForTimeout(50);
  }
  await page.waitForTimeout(1500);
  log.stop();
  const s = log.stats();
  expect(s.count, s.paths.join('\n')).toBeLessThanOrEqual(3);
  expect(s.count, 'the settled key still loads').toBeGreaterThanOrEqual(1);
  await expect(page.getByTestId('k8s-drawer')).toBeVisible();
});
