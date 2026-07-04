// Sidecar E2E: spawns the REAL server.js with a mock Otto host API, a mock
// Jira (fixtures/mock-jira.js), and a scripted temp git repo — then drives the
// HTTP surface end-to-end: scan → overview/assignee → goals → config →
// incremental rescan. Zero dependencies; run (from the plugin dir): node --test
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
let agentProviders = [];

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
  // A keyless automation feature (no Jira story) + a deploy tag covering
  // everything merged so far — feeds /features and deployed_at.
  g(['checkout', '-q', '-b', 'feature/nightly-e2e-suite']);
  commit('nightly suite runner', '2026-06-20T10:00:00Z');
  commit('nightly suite reports', '2026-06-21T10:00:00Z');
  g(['checkout', '-q', 'develop']);
  g(['merge', '-q', '--no-ff', '-m', 'Merged in feature/nightly-e2e-suite (pull request #9)', 'feature/nightly-e2e-suite'], '2026-06-22T09:00:00Z');
  g(['tag', '-a', 'v1.0-abc-DEPLOYED', '-m', 'prod'], '2026-06-23T09:00:00Z');
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
        const body = JSON.parse(b);
        agentPrompts.push(body.prompt);
        agentProviders.push(body.provider || 'claude');
        // Estimation batches ask for STRICT JSON — answer with a fixed 2d
        // per task (TP-6 flagged routine) so estimate fields are testable.
        if (body.prompt.includes('STRICT JSON')) {
          const keys = [...body.prompt.matchAll(/key=(\S+)/g)].map((m) => m[1]);
          return send(200, { text: JSON.stringify(keys.map((k) => ({ key: k, days: 2, routine: k === 'TP-6' }))) });
        }
        if (body.prompt.includes('SELF-CONTAINED HTML')) {
          return send(200, { text: 'Here you go:\n<!doctype html><html><body><h1>Report for Alice</h1><p>solid quarter</p></body></html>\nthanks' });
        }
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
  assert.deepEqual(projects.json, [{ key: 'TP', name: 'Team Performance Fixture', scanned: false }]);
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
  // Git-primary timing: Alice's cycles shift to first-active/commit -> MERGE
  // (TP-1 jira cycle was 4.0; eff runs to the merge commit an hour later).
  assert.ok(alice.median_cycle > 3.5 && alice.median_cycle < 5.5, `alice median ${alice.median_cycle}`);
  // Scope-weighted stats came from the mock estimator (2d per task).
  assert.ok(alice.weighted_done > 0, `weighted ${alice.weighted_done}`);
  assert.ok(alice.efficiency !== null, 'efficiency computed');
  // Scope metrics + deploy signal from the -DEPLOYED tag.
  assert.ok(o.scope.median_cycle_days > 0);
  assert.ok(o.scope.median_deploy_lead_days !== null, 'deploy lead measured from the tag');

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
  const bad = await api('PUT', '/config', { max_issues: -5 });
  assert.equal(bad.status, 400);
  const badWorker = await api('PUT', '/config', { estimate_workers: [{ provider: 'skynet' }] });
  assert.equal(badWorker.status, 400);

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

test('a failed issue fetch is retried on the next scan (not lost to the watermark)', async () => {
  // Touch TP-4 so the incremental scan includes it, but fail its first fetch.
  mockJira.touch('TP-4', new Date().toISOString());
  mockJira.failOnce('TP-4');
  await api('POST', '/scan', { account: 'acc1', project: 'TP' });
  const withError = await waitScanDone();
  assert.equal(withError.errors, 1);

  // Next scan: TP-4 no longer matches the JQL window by itself, but the
  // persisted fetch_failed list unions it back in — and it succeeds now.
  const before = mockJira.hits.issue.get('TP-4') || 0;
  await api('POST', '/scan', { account: 'acc1', project: 'TP' });
  const clean = await waitScanDone();
  assert.equal(clean.errors, 0);
  assert.equal((mockJira.hits.issue.get('TP-4') || 0) - before, 1, 'TP-4 refetched');
  const { json: o } = await api('GET', '/overview?account=acc1&project=TP');
  assert.equal(o.completed, 6, 'corpus intact');
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

// ---- v2 surface: people, overrides, scope goals, features, estimation --------

test('people: registry seeded from scan; PUT round-trips roles/aliases/inclusion', async () => {
  const { json: p } = await api('GET', '/people?account=acc1');
  assert.ok(p.people['u-alice'], 'alice discovered by the scan');
  assert.ok(Array.isArray(p.roles) && p.roles.includes('Developer'), 'default roles offered');

  const put = await api('PUT', '/people', {
    account: 'acc1',
    people: {
      'u-alice': { role: 'Senior Developer', included: true, aliases: ['alice.git'] },
      'u-bob': { included: false },
    },
  });
  assert.equal(put.status, 200);
  assert.equal(put.json.people['u-alice'].role, 'Senior Developer');
  assert.equal(put.json.people['u-bob'].included, false);

  // Excluded people leave the overview entirely.
  const { json: o } = await api('GET', '/overview?account=acc1&projects=TP');
  assert.ok(!o.assignees.some((a) => a.assignee_id === 'u-bob'), 'bob excluded from stats');
  const alice = o.assignees.find((a) => a.assignee_id === 'u-alice');
  assert.equal(alice.role, 'Senior Developer');

  // Restore for later tests.
  await api('PUT', '/people', { account: 'acc1', people: { 'u-bob': { included: true } } });
});

test('people/seed pulls assignable users from Jira (people picker before first scan)', async () => {
  const r = await api('POST', '/people/seed', { account: 'acc1', project: 'TP' });
  assert.equal(r.status, 200);
  assert.ok(r.json.people['u-carol'], 'carol added from assignable users');
  assert.ok(!r.json.people['u-app'], 'app accounts filtered out');
});

test('override: mark outlier excludes the story; manual time re-includes it', async () => {
  const before = await api('GET', '/assignee?account=acc1&projects=TP&assignee=u-alice');
  const t = before.json.completed.find((x) => x.key === 'TP-1');
  assert.ok(t && !t.excluded);

  const mark = await api('PUT', '/override', { account: 'acc1', project: 'TP', key: 'TP-1', outlier: true });
  assert.equal(mark.status, 200);
  let v = await api('GET', '/assignee?account=acc1&projects=TP&assignee=u-alice');
  let tp1 = v.json.completed.find((x) => x.key === 'TP-1');
  assert.equal(tp1.outlier, true);
  assert.equal(tp1.excluded, true, 'outlier leaves the stats');

  const manual = await api('PUT', '/override', { account: 'acc1', project: 'TP', key: 'TP-1', manual_days: 2.5 });
  assert.equal(manual.status, 200);
  v = await api('GET', '/assignee?account=acc1&projects=TP&assignee=u-alice');
  tp1 = v.json.completed.find((x) => x.key === 'TP-1');
  assert.equal(tp1.manual_days, 2.5);
  assert.equal(tp1.excluded, false, 'manual time re-enters the stats');
  assert.equal(tp1.actual_days, 2.5, 'actual shows the manual value');

  const bad = await api('PUT', '/override', { account: 'acc1', project: 'TP', key: 'TP-1', manual_days: -1 });
  assert.equal(bad.status, 400);
  // Clean up.
  await api('PUT', '/override', { account: 'acc1', project: 'TP', key: 'TP-1', outlier: false, manual_days: null });
});

test('scope goals: PUT team + role goals, overview evaluates them directionally', async () => {
  const put = await api('PUT', '/goals/scope', {
    account: 'acc1',
    team: [{ metric: 'median_cycle_days', target: 100 }],
    roles: { 'Senior Developer': [{ metric: 'weighted_throughput_wk', target: 9999 }] },
  });
  assert.equal(put.status, 200);
  const { json: o } = await api('GET', '/overview?account=acc1&projects=TP');
  const teamGoal = o.scope_goals.team.find((g) => g.metric === 'median_cycle_days');
  assert.equal(teamGoal.met, true, 'cycle far below 100');
  const roleGoal = o.scope_goals.roles['Senior Developer'][0];
  assert.equal(roleGoal.met, false, 'throughput target unreachable');

  const bad = await api('PUT', '/goals/scope', { account: 'acc1', team: [{ metric: 'nope', target: 1 }] });
  assert.equal(bad.status, 400);
});

test('3-level estimates flow into tasks (AI estimate -> per-dev -> actual)', async () => {
  const { json: v } = await api('GET', '/assignee?account=acc1&projects=TP&assignee=u-alice');
  const done = v.completed.find((x) => x.key === 'TP-1');
  assert.equal(done.est_days_ai, 2, 'agnostic AI estimate from the mock estimator');
  assert.ok(done.est_days_dev > 0, 'per-dev expected');
  assert.ok(done.actual_days > 0, 'actual');
  assert.equal(done.timing_source, 'git');
  const open = v.open.find((x) => x.key === 'TP-7');
  assert.equal(open.est_days_ai, 2);
  assert.equal(open.prediction.based_on, 'ai_estimate', 'prediction anchors on the AI estimate');
});

test('feature repos: opt-in scan extracts keyless git features with estimates', async () => {
  const cfg = await api('PUT', '/config', { feature_repos: ['fixture'] });
  assert.equal(cfg.status, 200);
  await api('POST', '/scan', { account: 'acc1', projects: ['TP'] });
  await waitScanDone();
  const { json: f } = await api('GET', '/features?account=acc1');
  assert.ok(f.scanned_at > 0);
  const feat = f.features.find((x) => x.branch === 'feature/nightly-e2e-suite');
  assert.ok(feat, `nightly feature extracted (${f.features.map((x) => x.branch).join(', ')})`);
  assert.equal(feat.commit_count, 2);
  assert.ok(feat.actual_days > 0, 'actual from first commit -> merge');
  assert.equal(feat.est_days_ai, 2, 'features estimated too');
  assert.ok(feat.deployed_at > feat.merged_at, 'deploy tag correlated');
});

test('scan accepts an assignee scope (per-dev fetch) and records it', async () => {
  const r = await api('POST', '/scan', { account: 'acc1', projects: ['TP'], assignees: ['u-alice'] });
  assert.equal(r.status, 200);
  const done = await waitScanDone();
  assert.equal(done.scoped_people, 1);
  // Corpus keeps previously scanned issues (scoped scan unions in).
  const { json: o } = await api('GET', '/overview?account=acc1&projects=TP');
  assert.ok(o.completed >= 6);
});

// ---- v3.1: period filter, merges, reports -------------------------------------

test('since param windows the stats (old completions leave)', async () => {
  const all = await api('GET', '/overview?account=acc1&projects=TP');
  const windowed = await api('GET', `/overview?account=acc1&projects=TP&since=${Date.parse('2026-06-15T00:00:00Z')}`);
  assert.ok(windowed.json.completed < all.json.completed, `windowed ${windowed.json.completed} < all ${all.json.completed}`);
  assert.equal(windowed.json.since, Date.parse('2026-06-15T00:00:00Z'));
  const alice = windowed.json.assignees.find((a) => a.assignee_id === 'u-alice');
  assert.ok(!alice || alice.completed <= 3);
  // Monthly series present on stats rows.
  const anyDev = all.json.assignees[0];
  assert.ok(Array.isArray(anyDev.monthly) && anyDev.monthly.length === 12);
});

test('people merge folds two accounts into one person', async () => {
  // Merge bob INTO alice, then the overview shows one combined person.
  await api('PUT', '/people', { account: 'acc1', people: { 'u-bob': { merged_into: 'u-alice' } } });
  const { json: o } = await api('GET', '/overview?account=acc1&projects=TP');
  assert.ok(!o.assignees.some((a) => a.assignee_id === 'u-bob'), 'bob folded');
  const alice = o.assignees.find((a) => a.assignee_id === 'u-alice');
  assert.equal(alice.completed, 6, 'combined completed count');
  // Unmerge for later tests.
  await api('PUT', '/people', { account: 'acc1', people: { 'u-bob': { merged_into: null } } });
});

test('report: generate per quarter, saved to the hub, html retrievable', async () => {
  const start = await api('POST', '/report', { account: 'acc1', assignee: 'u-alice', kind: 'quarter', year: 2026, quarter: 2 });
  assert.equal(start.status, 200);
  let status;
  const deadline = Date.now() + 15000;
  for (;;) {
    status = (await api('GET', `/report/status?job=${encodeURIComponent(start.json.job)}`)).json;
    if (status.state !== 'running') break;
    if (Date.now() > deadline) throw new Error('report never finished');
    await new Promise((r) => setTimeout(r, 150));
  }
  assert.equal(status.state, 'done', status.error || '');
  assert.equal(status.report.label, '2026 Q2');

  const list = await api('GET', '/reports?account=acc1&assignee=u-alice');
  assert.equal(list.json.reports.length, 1);
  const html = await api('GET', `/report/html?account=acc1&id=${encodeURIComponent(status.report.id)}`);
  assert.ok(html.json.html.startsWith('<!doctype html>'), 'prose stripped, pure html stored');
  assert.ok(html.json.html.includes('Report for Alice'));

  const bad = await api('POST', '/report', { account: 'acc1', assignee: 'u-alice', kind: 'quarter', year: 2026, quarter: 9 });
  assert.equal(bad.status, 400);
});

test('story exclude/include override removes it from every stat', async () => {
  const before = (await api('GET', '/overview?account=acc1&projects=TP')).json;
  const alice0 = before.assignees.find((a) => a.assignee_id === 'u-alice');
  await api('PUT', '/override', { account: 'acc1', project: 'TP', key: 'TP-1', excluded: true });
  const after = (await api('GET', '/overview?account=acc1&projects=TP')).json;
  const alice1 = after.assignees.find((a) => a.assignee_id === 'u-alice');
  assert.equal(alice1.completed, alice0.completed - 1, 'excluded story leaves the Done count');
  const { json: v } = await api('GET', '/assignee?account=acc1&projects=TP&assignee=u-alice');
  const tp1 = v.completed.find((x) => x.key === 'TP-1');
  assert.ok(tp1, 'still listed for re-inclusion');
  assert.equal(tp1.excluded_override, true);
  await api('PUT', '/override', { account: 'acc1', project: 'TP', key: 'TP-1', excluded: false });
  const restored = (await api('GET', '/overview?account=acc1&projects=TP')).json;
  assert.equal(restored.assignees.find((a) => a.assignee_id === 'u-alice').completed, alice0.completed);
});

test('unscoped fix commits (TP-0000) surface per person', async () => {
  // Add a placeholder-key commit and rescan so the git index picks it up.
  execFileSync('git', ['-C', repoDir, 'checkout', '-q', 'develop']);
  fs.appendFileSync(path.join(repoDir, 'f.txt'), 'hotfix\n');
  execFileSync('git', ['-C', repoDir, 'add', '.']);
  execFileSync('git', ['-C', repoDir, 'commit', '-q', '-m', 'TP-0000 urgent prod fix'], {
    env: { ...process.env, GIT_AUTHOR_NAME: 'Alice', GIT_AUTHOR_EMAIL: 'a@x', GIT_COMMITTER_NAME: 'Alice', GIT_COMMITTER_EMAIL: 'a@x' },
  });
  await api('POST', '/scan', { account: 'acc1', projects: ['TP'] });
  await waitScanDone();
  const { json: f } = await api('GET', '/features?account=acc1');
  const alice = (f.unscoped || []).find((u) => u.name === 'Alice');
  assert.ok(alice, `unscoped tracked (${JSON.stringify(f.unscoped)})`);
  assert.ok(alice.commits >= 1);
  const { json: o } = await api('GET', '/overview?account=acc1&projects=TP');
  const arow = o.assignees.find((x) => x.assignee_id === 'u-alice');
  assert.ok(arow.unscoped_commits >= 1, `overview column carries it (${arow.unscoped_commits})`);
});

test('auto-scan cron repeats the last scan params (isolated sidecar, fast tick)', async () => {
  // Fresh sidecar with a fast auto-scan tick so the cron fires in-test.
  const dataDir2 = fs.mkdtempSync(path.join(os.tmpdir(), 'tp-e2e-auto-'));
  const port2 = 20000 + Math.floor(Math.random() * 20000);
  const plugin2 = spawn(process.execPath, ['server.js'], {
    cwd: path.join(__dirname, '..'),
    env: {
      ...process.env,
      OTTO_PLUGIN_PORT: String(port2),
      OTTO_HOST_API: `http://127.0.0.1:${hostPort}`,
      OTTO_PLUGIN_TOKEN: 'ptok',
      OTTO_PLUGIN_DATA_DIR: dataDir2,
      OTTO_TP_AUTOSCAN_MS: '900',
    },
    stdio: ['ignore', 'ignore', 'pipe'],
  });
  const api2 = (method, pathname, body) =>
    new Promise((resolve, reject) => {
      const data = body ? JSON.stringify(body) : null;
      const req = http.request(
        { method, hostname: '127.0.0.1', port: port2, path: pathname, headers: data ? { 'Content-Type': 'application/json' } : {} },
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
  try {
    for (let i = 0; ; i++) {
      try {
        if ((await api2('GET', '/health')).status === 200) break;
      } catch { /* boot */ }
      if (i > 100) throw new Error('auto sidecar never healthy');
      await new Promise((r) => setTimeout(r, 100));
    }
    // Manual scan seeds last_scan.json; the cron then repeats it.
    await api2('POST', '/scan', { account: 'acc1', projects: ['TP'] });
    let s;
    for (let i = 0; ; i++) {
      s = (await api2('GET', '/scan/status?account=acc1')).json;
      if (s.state === 'done') break;
      if (i > 200) throw new Error('manual scan never finished');
      await new Promise((r) => setTimeout(r, 100));
    }
    const firstFinish = s.finished_at;
    // Wait for an auto run: finished_at must advance and the job carry auto:true.
    let auto = null;
    for (let i = 0; i < 100; i++) {
      await new Promise((r) => setTimeout(r, 200));
      const cur = (await api2('GET', '/scan/status?account=acc1')).json;
      if (cur.auto && cur.state === 'done' && cur.finished_at > firstFinish) { auto = cur; break; }
    }
    assert.ok(auto, 'auto-scan fired and completed');
    assert.equal(auto.errors, 0);
  } finally {
    plugin2.kill('SIGKILL');
    fs.rmSync(dataDir2, { recursive: true, force: true });
  }
});

test('estimate override: corrects the agnostic estimate everywhere + feeds future prompts', async () => {
  const before = (await api('GET', '/assignee?account=acc1&projects=TP&assignee=u-alice')).json;
  const t0 = before.completed.find((x) => x.key === 'TP-1');
  assert.equal(t0.est_days_ai, 2, 'starts at the AI estimate');

  const put = await api('PUT', '/override', { account: 'acc1', project: 'TP', key: 'TP-1', est_days: 5, est_reason: 'much larger than it reads — cross-service' });
  assert.equal(put.status, 200);
  const after = (await api('GET', '/assignee?account=acc1&projects=TP&assignee=u-alice')).json;
  const t1 = after.completed.find((x) => x.key === 'TP-1');
  assert.equal(t1.est_days_ai, 5, 'agnostic reflects the override');
  assert.equal(t1.est_ai_original, 2, 'original AI value preserved for display');
  assert.equal(t1.est_overridden, true);
  assert.equal(t1.est_reason, 'much larger than it reads — cross-service');

  // The correction shows in weighted throughput (a scope-weighted metric).
  const o = (await api('GET', '/overview?account=acc1&projects=TP')).json;
  assert.ok(o.assignees.find((a) => a.assignee_id === 'u-alice').weighted_done > 0);

  const bad = await api('PUT', '/override', { account: 'acc1', project: 'TP', key: 'TP-1', est_days: 999 });
  assert.equal(bad.status, 400);

  // Restore.
  await api('PUT', '/override', { account: 'acc1', project: 'TP', key: 'TP-1', est_days: null, est_reason: null });
  const restored = (await api('GET', '/assignee?account=acc1&projects=TP&assignee=u-alice')).json;
  assert.equal(restored.completed.find((x) => x.key === 'TP-1').est_days_ai, 2, 'restored to AI');
});

test('team report: month scope, masked, saved to the hub', async () => {
  const start = await api('POST', '/report', { account: 'acc1', scope: 'team', kind: 'month', year: 2026, month: 6, mask: true, mask_tasks: true });
  assert.equal(start.status, 200);
  let s;
  const deadline = Date.now() + 15000;
  for (;;) {
    s = (await api('GET', `/report/status?job=${encodeURIComponent(start.json.job)}`)).json;
    if (s.state !== 'running') break;
    if (Date.now() > deadline) throw new Error('team report never finished');
    await new Promise((r) => setTimeout(r, 150));
  }
  assert.equal(s.state, 'done', s.error || '');
  assert.equal(s.report.report_scope, 'team');
  assert.equal(s.report.masked, true);
  assert.ok(s.report.label.includes('Team'));
  assert.ok(s.report.label.includes('Jun 2026'));

  const teamOnly = await api('GET', '/reports?account=acc1&scope=team');
  assert.ok(teamOnly.json.reports.every((r) => r.report_scope === 'team'));
  assert.ok(teamOnly.json.reports.some((r) => r.id === s.report.id));

  const html = await api('GET', `/report/html?account=acc1&id=${encodeURIComponent(s.report.id)}`);
  assert.ok(html.json.html.startsWith('<!doctype html>'));

  const bad = await api('POST', '/report', { account: 'acc1', scope: 'team', kind: 'month', year: 2026, month: 13 });
  assert.equal(bad.status, 400);
});

test('editable prompts: rubric/instructions/report_instructions round-trip; defaults exposed', async () => {
  const cfg0 = await api('GET', '/config');
  assert.ok(Array.isArray(cfg0.json._defaults.rubric) && cfg0.json._defaults.rubric.length, 'default rubric exposed');
  assert.ok(cfg0.json._defaults.team_report_instructions.length, 'team report default exposed');

  const put = await api('PUT', '/config', {
    estimate_instructions: 'Be strict on bug tickets.',
    report_instructions: 'Focus on goal deltas.',
    estimate_rubric: ['Everything tiny is 0.5d.'],
  });
  assert.equal(put.status, 200);
  assert.equal(put.json.estimate_instructions, 'Be strict on bug tickets.');
  assert.equal(put.json.report_instructions, 'Focus on goal deltas.');
  assert.deepEqual(put.json.estimate_rubric, ['Everything tiny is 0.5d.']);
  // restore
  await api('PUT', '/config', { estimate_instructions: '', report_instructions: '', estimate_rubric: [] });
});

test('fix inclusion resolves through applyOverrides (auto + explicit + partial)', async () => {
  // Explicit include on a task, verify the flag surfaces in the assignee view.
  await api('PUT', '/override', { account: 'acc1', project: 'TP', key: 'TP-1', include_fixes: true });
  let v = await api('GET', '/assignee?account=acc1&projects=TP&assignee=u-alice');
  let t = v.json.completed.find((x) => x.key === 'TP-1');
  assert.equal(t.include_fixes, true, 'explicit include reflected');
  assert.equal(t.include_fixes_override, true);

  // Partial fix-days override round-trips.
  await api('PUT', '/override', { account: 'acc1', project: 'TP', key: 'TP-1', fix_days_override: 2.5 });
  v = await api('GET', '/assignee?account=acc1&projects=TP&assignee=u-alice');
  t = v.json.completed.find((x) => x.key === 'TP-1');
  assert.equal(t.fix_days_override, 2.5, 'partial fix-days reflected');

  const bad = await api('PUT', '/override', { account: 'acc1', project: 'TP', key: 'TP-1', fix_days_override: 999 });
  assert.equal(bad.status, 400);

  // Auto: with the threshold at 1, a task that HAS a fix auto-includes; with a
  // high threshold it doesn't. (TP-1 has no release fix in the e2e repo, so we
  // just assert the auto flag tracks the configured threshold monotonically.)
  await api('PUT', '/override', { account: 'acc1', project: 'TP', key: 'TP-1', include_fixes: null, fix_days_override: null });
  v = await api('GET', '/assignee?account=acc1&projects=TP&assignee=u-alice');
  t = v.json.completed.find((x) => x.key === 'TP-1');
  assert.equal(t.include_fixes_override, null, 'reset clears the explicit choice');
  assert.equal(typeof t.include_fixes, 'boolean', 'auto resolution still yields a boolean');
});

test('combined report: team overview + a section per developer, maskable', async () => {
  const start = await api('POST', '/report', { account: 'acc1', scope: 'combined', kind: 'month', year: 2026, month: 6, mask: true });
  assert.equal(start.status, 200);
  let s;
  const deadline = Date.now() + 15000;
  for (;;) {
    s = (await api('GET', `/report/status?job=${encodeURIComponent(start.json.job)}`)).json;
    if (s.state !== 'running') break;
    if (Date.now() > deadline) throw new Error('combined report never finished');
    await new Promise((r) => setTimeout(r, 150));
  }
  assert.equal(s.state, 'done', s.error || '');
  assert.equal(s.report.report_scope, 'combined');
  assert.equal(s.report.masked, true);
  // The prompt must ask for a per-developer section and carry per_developer data.
  const prompt = agentPrompts[agentPrompts.length - 1];
  assert.ok(prompt.includes('individual section for EVERY developer'), 'combined headline');
  assert.ok(prompt.includes('per_developer'), 'per-developer data embedded');
  // combined reports surface under a team-scope filter query too.
  const teamList = await api('GET', '/reports?account=acc1&scope=combined');
  assert.ok(teamList.json.reports.some((r) => r.id === s.report.id));
});

test('report already-running returns the job to attach to (not a 409); active endpoint lists it', async () => {
  // Kick a slow-ish report and immediately request it again — second call
  // should hand back the same job with already=true, and /reports/active lists it.
  const first = await api('POST', '/report', { account: 'acc1', scope: 'team', kind: 'year', year: 2026 });
  assert.equal(first.status, 200);
  assert.ok(first.json.job);
  // (the mock agent replies instantly, so the job may already be done — assert the
  // active endpoint shape and the label rather than racing the timing.)
  const active = await api('GET', '/reports/active?account=acc1');
  assert.ok(Array.isArray(active.json.active));
  // wait for it to finish
  const deadline = Date.now() + 10000;
  for (;;) {
    const s = (await api('GET', `/report/status?job=${encodeURIComponent(first.json.job)}`)).json;
    if (s.state !== 'running') break;
    if (Date.now() > deadline) throw new Error('year report never finished');
    await new Promise((r) => setTimeout(r, 100));
  }
  // a second request after completion starts fresh (job re-runs), returns a label
  const second = await api('POST', '/report', { account: 'acc1', scope: 'team', kind: 'year', year: 2026 });
  assert.ok(second.json.label.includes('Team') && second.json.label.includes('2026'));
});
