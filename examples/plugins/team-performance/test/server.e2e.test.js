// Sidecar E2E: spawns the REAL server.js with a mock Otto host API, a mock
// Jira (fixtures/mock-jira.js), and a scripted temp git repo — then drives the
// HTTP surface end-to-end: scan → overview/assignee → goals → config →
// incremental rescan. Zero dependencies; run: node --test test/
const { test, before, after } = require('node:test');
const assert = require('node:assert/strict');
const http = require('node:http');
const { spawn, execFileSync } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const { startMockJira } = require('./fixtures/mock-jira.js');

let mockJira;
let hostServer;
let hostPort;
let plugin;
let pluginPort;
let dataDir;
let repoDir;
let agentPrompts = [];

// ---- helpers ----------------------------------------------------------------

function api(method, pathname, body) {
  return new Promise((resolve, reject) => {
    const data = body ? JSON.stringify(body) : null;
    const req = http.request(
      { method, hostname: '127.0.0.1', port: pluginPort, path: pathname, headers: data ? { 'Content-Type': 'application/json' } : {} },
      (res) => {
        let buf = '';
        res.on('data', (c) => (buf += c));
        res.on('end', () => resolve({ status: res.statusCode, json: buf ? JSON.parse(buf) : null }));
      },
    );
    req.on('error', reject);
    if (data) req.write(data);
    req.end();
  });
}

async function waitScanDone(timeoutMs = 30000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const r = await api('GET', '/scan/status?account=acc1&project=TP');
    if (r.json.state === 'done') return r.json;
    if (r.json.state === 'error') throw new Error(`scan errored: ${r.json.error}`);
    await new Promise((r2) => setTimeout(r2, 100));
  }
  throw new Error('scan did not finish in time');
}

function gitRepo() {
  repoDir = fs.mkdtempSync(path.join(os.tmpdir(), 'tp-e2e-repo-'));
  const g = (args, when) =>
    execFileSync('git', ['-C', repoDir, ...args], {
      encoding: 'utf8',
      env: {
        ...process.env,
        GIT_AUTHOR_NAME: 't', GIT_AUTHOR_EMAIL: 't@t', GIT_COMMITTER_NAME: 't', GIT_COMMITTER_EMAIL: 't@t',
        ...(when ? { GIT_AUTHOR_DATE: when, GIT_COMMITTER_DATE: when } : {}),
      },
    });
  const commit = (msg, when) => {
    fs.appendFileSync(path.join(repoDir, 'f.txt'), msg + '\n');
    g(['add', '.']);
    g(['commit', '-q', '-m', msg], when);
  };
  g(['init', '-q', '-b', 'main']);
  commit('init', '2026-05-01T09:00:00Z');
  g(['checkout', '-q', '-b', 'develop']);
  // Deliveries for the done fixture issues (TP-5 deliberately has NO commits).
  for (const [key, dev, merge] of [
    ['TP-1', '2026-06-02T12:00:00Z', '2026-06-05T11:00:00Z'],
    ['TP-2', '2026-06-09T10:00:00Z', '2026-06-10T16:00:00Z'],
    ['TP-3', '2026-06-11T10:00:00Z', '2026-06-16T11:00:00Z'],
    ['TP-4', '2026-06-05T10:00:00Z', '2026-06-10T10:00:00Z'],
    ['TP-6', '2026-06-18T10:00:00Z', '2026-06-19T10:00:00Z'],
  ]) {
    g(['checkout', '-q', '-b', `feature/${key}-work`]);
    commit(`${key}: implement`, dev);
    g(['checkout', '-q', 'develop']);
    g(['merge', '-q', '--no-ff', '-m', `Merge branch 'feature/${key}-work' into develop`, `feature/${key}-work`], merge);
  }
  // TP-7 in flight on an unmerged branch.
  g(['checkout', '-q', '-b', 'feature/TP-7-wip']);
  commit('TP-7: wip', '2026-06-24T15:00:00Z');
  g(['checkout', '-q', 'develop']);
}

before(async () => {
  mockJira = await startMockJira();
  gitRepo();
  dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'tp-e2e-data-'));

  // Mock Otto host API: /repos, /jira/accounts, /jira/credentials, /agents/run.
  hostServer = http.createServer((req, res) => {
    const u = new URL(req.url, 'http://localhost');
    const send = (code, obj) => {
      res.writeHead(code, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify(obj));
    };
    if (u.pathname === '/repos') return send(200, [{ id: 'r1', name: 'fixture', path: repoDir, remote_url: null }]);
    if (u.pathname === '/jira/accounts') return send(200, [{ id: 'acc1', label: 'Fixture Jira', base_url: `http://127.0.0.1:${mockJira.port}`, email: 'e@e' }]);
    if (u.pathname === '/jira/credentials') {
      return send(200, { base_url: `http://127.0.0.1:${mockJira.port}`, email: 'e@e', token: 'tok' });
    }
    if (u.pathname === '/agents/run') {
      let b = '';
      req.on('data', (c) => (b += c));
      req.on('end', () => {
        agentPrompts.push(JSON.parse(b).prompt);
        send(200, { text: 'coach says: focus WIP' });
      });
      return undefined;
    }
    return send(404, {});
  });
  await new Promise((r) => hostServer.listen(0, '127.0.0.1', r));
  hostPort = hostServer.address().port;

  // Spawn the real sidecar.
  pluginPort = 20000 + Math.floor(Math.random() * 20000);
  plugin = spawn(process.execPath, ['server.js'], {
    cwd: path.join(__dirname, '..'),
    env: {
      ...process.env,
      OTTO_PLUGIN_PORT: String(pluginPort),
      OTTO_HOST_API: `http://127.0.0.1:${hostPort}`,
      OTTO_PLUGIN_TOKEN: 'ptok',
      OTTO_PLUGIN_DATA_DIR: dataDir,
    },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  plugin.stderr.on('data', (c) => process.stderr.write(`[sidecar] ${c}`));
  // Wait for /health.
  const deadline = Date.now() + 10000;
  for (;;) {
    try {
      const r = await api('GET', '/health');
      if (r.status === 200) break;
    } catch {
      /* not up yet */
    }
    if (Date.now() > deadline) throw new Error('sidecar never became healthy');
    await new Promise((r) => setTimeout(r, 100));
  }
});

after(async () => {
  if (plugin) plugin.kill('SIGKILL');
  if (hostServer) await new Promise((r) => hostServer.close(r));
  if (mockJira) await mockJira.close();
  for (const d of [dataDir, repoDir]) if (d) fs.rmSync(d, { recursive: true, force: true });
});

// ---- tests (serial by declaration order) -------------------------------------

test('accounts + projects come from host API and Jira', async () => {
  const accts = await api('GET', '/accounts');
  assert.equal(accts.json[0].id, 'acc1');
  const projects = await api('GET', '/projects?account=acc1');
  assert.deepEqual(projects.json, [{ key: 'TP', name: 'Team Performance Fixture' }]);
});

test('overview before any scan is 404', async () => {
  const r = await api('GET', '/overview?account=acc1&project=TP');
  assert.equal(r.status, 404);
});

test('scan 409s while running, then completes', async () => {
  mockJira.delayMs = 60; // slow Jira so the double-POST races reliably
  const first = await api('POST', '/scan', { account: 'acc1', project: 'TP' });
  assert.equal(first.status, 200);
  const dup = await api('POST', '/scan', { account: 'acc1', project: 'TP' });
  assert.equal(dup.status, 409);
  mockJira.delayMs = 0;
  const done = await waitScanDone();
  assert.equal(done.errors, 0);
  assert.equal(done.state, 'done');
});

test('overview: assignees, baselines, open predictions, flags', async () => {
  const { json: o } = await api('GET', '/overview?account=acc1&project=TP');
  assert.equal(o.completed, 6);
  assert.equal(o.open, 2);
  assert.equal(o.capped, false);
  assert.equal(o.target_used.fixture, 'develop');

  const alice = o.assignees.find((a) => a.assignee_id === 'u-alice');
  const bob = o.assignees.find((a) => a.assignee_id === 'u-bob');
  assert.ok(alice && bob, 'both devs present');
  assert.equal(alice.completed, 3);
  assert.equal(bob.completed, 3);
  // Alice's cycles: TP-1=4, TP-2=2.25, TP-3=5 -> median 4
  assert.ok(Math.abs(alice.median_cycle - 4) < 0.01, `alice median ${alice.median_cycle}`);

  // Baseline buckets exist, Story|3 has n=4.
  const s3 = o.baseline.find((b) => b.type === 'Story' && b.points === 3);
  assert.ok(s3, 'Story|3 bucket');
  assert.equal(s3.n, 4);

  // Team-level open tasks with predictions.
  assert.equal(o.open_tasks.length, 2);
  const tp7 = o.open_tasks.find((t) => t.key === 'TP-7');
  assert.ok(tp7.prediction, 'TP-7 has a prediction');
  assert.ok(tp7.prediction.total.p50 > 0);
  assert.ok(tp7.projected_done_at > 0);

  // TP-5 was done with no commits -> no_code flag counted.
  assert.ok(o.flags.no_code >= 1, JSON.stringify(o.flags));
});

test('assignee view: per-task actuals vs baseline, verdicts, evidence intervals, goals', async () => {
  const { json: v } = await api('GET', '/assignee?account=acc1&project=TP&assignee=u-alice');
  assert.equal(v.stats.assignee_id, 'u-alice');
  assert.equal(v.completed.length, 3);

  const tp1 = v.completed.find((t) => t.key === 'TP-1');
  assert.ok(Math.abs(tp1.design_days - 1) < 0.01, `design ${tp1.design_days}`);
  assert.ok(Math.abs(tp1.impl_days - 3) < 0.01, `impl ${tp1.impl_days}`);
  assert.ok(Math.abs(tp1.cycle_days - 4) < 0.01);
  assert.ok(tp1.baseline, 'baseline attached');
  assert.equal(tp1.baseline.level, 'type+points');
  assert.ok(['fast', 'on_track', 'slow'].includes(tp1.verdicts.total));
  assert.ok(Array.isArray(tp1.intervals) && tp1.intervals.length >= 4, 'evidence intervals stored');
  assert.ok(tp1.intervals.every((iv) => iv.status && iv.phase));
  assert.ok(tp1.delivered_at > 0, 'git delivery correlated');

  // Open task for alice with prediction.
  assert.equal(v.open.length, 1);
  assert.equal(v.open[0].key, 'TP-7');
  assert.ok(v.open[0].prediction);

  // Goals: auto-suggested with current values + snapshot history from the scan.
  assert.ok(v.goals.length >= 1);
  const cycleGoal = v.goals.find((g) => g.metric === 'median_cycle_days');
  assert.ok(cycleGoal, 'cycle goal suggested');
  assert.ok(cycleGoal.suggested);
  assert.ok(cycleGoal.history.length >= 1);
});

test('goals PUT round-trips and overrides the suggestion', async () => {
  const put = await api('PUT', '/goals', {
    account: 'acc1',
    project: 'TP',
    assignee: 'u-alice',
    goals: [{ metric: 'median_cycle_days', target: 3.5 }],
  });
  assert.equal(put.status, 200);
  const { json: v } = await api('GET', '/assignee?account=acc1&project=TP&assignee=u-alice');
  const g = v.goals.find((x) => x.metric === 'median_cycle_days');
  assert.equal(g.target, 3.5);
  assert.equal(g.suggested, false);
  assert.equal(typeof g.met, 'boolean');

  const bad = await api('PUT', '/goals', { account: 'acc1', project: 'TP', assignee: 'u-alice', goals: [{ metric: 'nope', target: 1 }] });
  assert.equal(bad.status, 400);
});

test('config: invalid rejected; status-map change recomputes locally (no Jira refetch)', async () => {
  const bad = await api('PUT', '/config', { max_issues: 1 });
  assert.equal(bad.status, 400);

  const jiraHitsBefore = mockJira.hits.search + [...mockJira.hits.issue.values()].reduce((a, b) => a + b, 0);
  // Reclassify "In Review" as waiting -> TP-1 impl drops from 3.0 to 2.0.
  const put = await api('PUT', '/config', { status_map: { TP: { 'In Review': 'waiting' } } });
  assert.equal(put.status, 200);
  const { json: v } = await api('GET', '/assignee?account=acc1&project=TP&assignee=u-alice');
  const tp1 = v.completed.find((t) => t.key === 'TP-1');
  assert.ok(Math.abs(tp1.impl_days - 2) < 0.01, `impl after remap ${tp1.impl_days}`);
  const jiraHitsAfter = mockJira.hits.search + [...mockJira.hits.issue.values()].reduce((a, b) => a + b, 0);
  assert.equal(jiraHitsAfter, jiraHitsBefore, 'no Jira traffic for a local recompute');
  // Restore the default map for later tests.
  await api('PUT', '/config', { status_map: {} });
});

test('incremental rescan refetches only the touched issue', async () => {
  const hitsBefore = new Map(mockJira.hits.issue);
  mockJira.touch('TP-6', new Date().toISOString());
  const r = await api('POST', '/scan', { account: 'acc1', project: 'TP' });
  assert.equal(r.status, 200);
  await waitScanDone();
  for (const key of ['TP-1', 'TP-2', 'TP-3', 'TP-4', 'TP-5', 'TP-7', 'TP-8']) {
    assert.equal(mockJira.hits.issue.get(key) || 0, hitsBefore.get(key) || 0, `${key} not refetched`);
  }
  assert.equal((mockJira.hits.issue.get('TP-6') || 0) - (hitsBefore.get('TP-6') || 0), 1, 'TP-6 refetched once');
  // Corpus survives: still 6 completed.
  const { json: o } = await api('GET', '/overview?account=acc1&project=TP');
  assert.equal(o.completed, 6);
});

test('statuses endpoint exposes the map defaults', async () => {
  const { json: sts } = await api('GET', '/statuses?account=acc1&project=TP');
  const design = sts.find((s) => s.name === 'In Design');
  assert.equal(design.mapped, 'design');
  const review = sts.find((s) => s.name === 'In Review');
  assert.equal(review.mapped, 'implementation');
});

test('AI coach builds an enriched prompt and returns the agent text', async () => {
  const r = await api('POST', '/analyze', { account: 'acc1', project: 'TP', assignee: 'u-bob' });
  assert.equal(r.json.summary, 'coach says: focus WIP');
  const prompt = agentPrompts[agentPrompts.length - 1];
  assert.ok(prompt.includes('Bob'), 'prompt names the dev');
  assert.ok(prompt.includes('median_impl'), 'prompt carries phase medians');
});

test('errors never leak details', async () => {
  const r = await api('POST', '/analyze', { account: 'acc1', project: 'NOPE' });
  assert.equal(r.status, 500);
  assert.deepEqual(r.json, { error: 'internal error' });
});
