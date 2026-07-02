// team-performance — Otto runtime plugin (Node sidecar, zero dependencies).
//
// Otto spawns this with: OTTO_PLUGIN_PORT (bind here), OTTO_PLUGIN_TOKEN +
// OTTO_HOST_API (call back for repos/jira/agents), OTTO_PLUGIN_DATA_DIR (state).
// Otto reverse-proxies /api/v1/plugins/team-performance/* to these routes.
//
// What it does: scans EVERY issue of a Jira project (paginated + incremental),
// derives per-task design/implementation/waiting time from the issue's own
// changelog (business days), correlates git delivery (one-pass index over the
// registered repos), builds statistical baselines ("how long it should have
// taken") per (type, points) bucket, predicts timelines for open tasks, and
// tracks editable per-dev goals across scans. Analytics live in lib/ (pure,
// unit-tested); this file is env, routing, and the async scan job.

const http = require('http');
const { URL } = require('url');

const A = require('./lib/analytics.js');
const { makeClient, detectPointsField } = require('./lib/jira.js');
const { buildIndex } = require('./lib/gitscan.js');
const store = require('./lib/store.js');

const PORT = parseInt(process.env.OTTO_PLUGIN_PORT || '0', 10);
const HOST_API = process.env.OTTO_HOST_API || '';
const TOKEN = process.env.OTTO_PLUGIN_TOKEN || '';
const DATA_DIR = process.env.OTTO_PLUGIN_DATA_DIR || '.';

// ---- host API ---------------------------------------------------------------

function hostJson(method, pathname, body) {
  return new Promise((resolve, reject) => {
    const u = new URL(HOST_API + pathname);
    const data = body ? JSON.stringify(body) : null;
    const req = http.request(
      {
        method,
        hostname: u.hostname,
        port: u.port,
        path: u.pathname + u.search,
        headers: {
          Accept: 'application/json',
          Authorization: `Bearer ${TOKEN}`,
          ...(data ? { 'Content-Type': 'application/json' } : {}),
        },
      },
      (res) => {
        let buf = '';
        res.on('data', (c) => (buf += c));
        res.on('end', () => {
          if (res.statusCode >= 200 && res.statusCode < 300) {
            try {
              resolve(buf ? JSON.parse(buf) : null);
            } catch {
              reject(new Error(`bad JSON from host ${pathname}`));
            }
          } else reject(new Error(`${res.statusCode} from host ${pathname}`));
        });
      },
    );
    req.on('error', reject);
    if (data) req.write(data);
    req.end();
  });
}
const hostGet = (p) => hostJson('GET', p);
const hostPost = (p, b) => hostJson('POST', p, b);

// ---- config -----------------------------------------------------------------

const DEFAULT_CONFIG = {
  issue_types: ['Story', 'Task', 'Bug'],
  target_branches: ['develop', 'main', 'master'],
  workweek: [1, 2, 3, 4, 5],
  max_issues: 1000,
  git_depth: 5000,
  status_map: {},
};

function loadConfig() {
  const c = store.readJson(store.configPath(DATA_DIR), {});
  return { ...DEFAULT_CONFIG, ...(c && typeof c === 'object' ? c : {}) };
}

const PHASE_VALUES = ['design', 'implementation', 'waiting', 'excluded'];

function validateConfig(body) {
  const c = { ...loadConfig() };
  if (body.issue_types !== undefined) {
    if (!Array.isArray(body.issue_types) || !body.issue_types.length || !body.issue_types.every((t) => typeof t === 'string' && t.trim())) {
      throw new Error('issue_types must be a non-empty string array');
    }
    c.issue_types = body.issue_types.map((t) => t.trim());
  }
  if (body.target_branches !== undefined) {
    if (!Array.isArray(body.target_branches) || !body.target_branches.length) throw new Error('target_branches must be non-empty');
    c.target_branches = body.target_branches.map(String);
  }
  if (body.workweek !== undefined) {
    if (!Array.isArray(body.workweek) || !body.workweek.length || !body.workweek.every((d) => Number.isInteger(d) && d >= 0 && d <= 6)) {
      throw new Error('workweek must be a non-empty array of weekday numbers 0-6');
    }
    c.workweek = [...new Set(body.workweek)].sort();
  }
  if (body.max_issues !== undefined) {
    const n = Number(body.max_issues);
    if (!Number.isInteger(n) || n < 10 || n > 10000) throw new Error('max_issues must be an integer in 10..10000');
    c.max_issues = n;
  }
  if (body.git_depth !== undefined) {
    const n = Number(body.git_depth);
    if (!Number.isInteger(n) || n < 100 || n > 50000) throw new Error('git_depth must be an integer in 100..50000');
    c.git_depth = n;
  }
  if (body.status_map !== undefined) {
    if (typeof body.status_map !== 'object' || body.status_map === null) throw new Error('status_map must be an object');
    for (const [proj, map] of Object.entries(body.status_map)) {
      if (typeof map !== 'object' || map === null) throw new Error(`status_map.${proj} must be an object`);
      for (const v of Object.values(map)) {
        if (!PHASE_VALUES.includes(v)) throw new Error(`status_map values must be one of ${PHASE_VALUES.join('/')}`);
      }
    }
    c.status_map = body.status_map;
  }
  return c;
}

// A project "has design statuses" when any status seen in its corpus (or its
// explicit map) classifies as design — gates the skipped_design flag.
function projectHasDesign(records, statusMap) {
  if (Object.values(statusMap).includes('design')) return true;
  const seen = new Set();
  for (const r of records) for (const iv of r.intervals || []) seen.add(iv.status);
  return [...seen].some((s) => A.classifyStatus(s, statusMap) === 'design');
}

/** Re-apply the git delivery signal + git-derived flags to an existing record. */
function applyGit(record, gitIndex, workweek) {
  const git = gitIndex.byKey.get(record.key) || {};
  const delivered = git.delivered_at ?? null;
  const first = git.first_commit_at ?? null;
  const flags = (record.flags || []).filter((f) => f !== 'no_code' && f !== 'late_merge');
  if (record.done_at !== null && gitIndex.hasRepos && delivered === null) flags.push('no_code');
  if (record.done_at !== null && delivered !== null && A.businessDays(record.done_at, delivered, workweek) > 2) flags.push('late_merge');
  return { ...record, delivered_at: delivered, first_commit_at: first, flags };
}

/** Recompute phase-dependent fields of every stored corpus after a config change. */
function recomputeCorpora(config) {
  for (const file of store.listCorpora(DATA_DIR)) {
    const corpus = store.readJson(file, null);
    if (!corpus || !corpus.issues) continue;
    const statusMap = config.status_map[corpus.project] || {};
    const records = Object.values(corpus.issues);
    const hasDesign = projectHasDesign(records, statusMap);
    for (const r of records) {
      corpus.issues[r.key] = A.reanalyzeRecord(r, {
        statusMap,
        workweek: config.workweek,
        hasDesignStatuses: hasDesign,
      });
    }
    store.writeJsonAtomic(file, corpus);
  }
}

// ---- scan job ---------------------------------------------------------------

const jobs = new Map(); // `${account}__${project}` -> status object

function jobKey(account, project) {
  return `${account}__${project}`;
}

const SEARCH_FIELDS_BASE = ['summary', 'issuetype', 'status', 'assignee', 'created', 'resolutiondate', 'updated', 'timeoriginalestimate'];

function fmtJqlUtc(ms) {
  const d = new Date(ms);
  const p = (n) => String(n).padStart(2, '0');
  return `${d.getUTCFullYear()}-${p(d.getUTCMonth() + 1)}-${p(d.getUTCDate())} ${p(d.getUTCHours())}:${p(d.getUTCMinutes())}`;
}

async function runScan(account, project, full) {
  const key = jobKey(account, project);
  const job = jobs.get(key);
  const scanStart = Date.now();
  try {
    const config = loadConfig();
    const creds = await hostGet(`/jira/credentials?account=${encodeURIComponent(account)}`);
    const client = makeClient(creds);
    const corpusFile = store.corpusPath(DATA_DIR, account, project);
    const corpus = (!full && store.readJson(corpusFile, null)) || { project, account, issues: {} };

    job.step = 'fields';
    const pointsField = corpus.points_field || detectPointsField(await client.fields());

    const types = config.issue_types.map((t) => `"${t.replace(/"/g, '')}"`).join(', ');
    let jql = `project = "${project.replace(/"/g, '')}" AND issuetype IN (${types})`;
    // Incremental: everything updated since the previous scan STARTED (minus a
    // 1-day buffer — JQL datetimes are interpreted in the Jira account's
    // timezone; the buffer absorbs the offset).
    if (!full && corpus.last_scan_start) jql += ` AND updated >= "${fmtJqlUtc(corpus.last_scan_start - A.DAY)}"`;
    jql += ' ORDER BY updated DESC';

    job.total = await client.approxCount(jql);
    job.step = 'search';
    const fields = [...SEARCH_FIELDS_BASE, pointsField];
    const found = await client.searchAll(jql, ['updated'], {
      maxIssues: config.max_issues,
      onPage: (n) => {
        job.fetched = n;
        job.retries = client.retries;
      },
    });
    const capped = found.length >= config.max_issues;

    job.step = 'changelogs';
    job.total = found.length;
    job.fetched = 0;
    const rawIssues = [];
    for (const stub of found) {
      const known = corpus.issues[stub.key];
      const updatedMs = Date.parse(stub.fields && stub.fields.updated) || null;
      if (known && known.updated && updatedMs && known.updated === updatedMs) {
        job.fetched++;
        continue; // unchanged since last scan — no refetch
      }
      try {
        rawIssues.push(await client.issueWithChangelog(stub.key, fields));
      } catch (e) {
        job.errors = (job.errors || 0) + 1;
        console.error(`scan: issue ${stub.key} failed:`, e.message);
      }
      job.fetched++;
      job.retries = client.retries;
    }

    job.step = 'git';
    const repos = (await hostGet('/repos')) || [];
    const gitIndex = buildIndex(repos, config);

    job.step = 'analyze';
    const statusMap = config.status_map[project] || {};
    const nowMs = Date.now();
    // First pass assumes design statuses exist; the flag is fixed after the
    // whole corpus is known (projectHasDesign needs every interval).
    for (const raw of rawIssues) {
      corpus.issues[raw.key] = A.analyzeIssue(raw, {
        statusMap,
        workweek: config.workweek,
        pointsField,
        gitIndex,
        hasDesignStatuses: true,
        nowMs,
      });
    }
    const records = Object.values(corpus.issues);
    const hasDesign = projectHasDesign(records, statusMap);
    for (const r of records) {
      let rec = applyGit(r, gitIndex, config.workweek);
      if (!hasDesign) rec = { ...rec, flags: rec.flags.filter((f) => f !== 'skipped_design') };
      corpus.issues[rec.key] = rec;
    }

    job.step = 'persist';
    corpus.points_field = pointsField;
    corpus.scanned_at = Date.now();
    corpus.last_scan_start = scanStart;
    corpus.capped = capped;
    corpus.target_used = gitIndex.target_used;
    store.writeJsonAtomic(corpusFile, corpus);
    appendGoalSnapshots(account, project, corpus, config);

    job.state = 'done';
    job.finished_at = Date.now();
    job.last_scan = corpus.scanned_at;
  } catch (e) {
    console.error('scan failed:', e);
    job.state = 'error';
    job.error = 'scan failed — see plugin logs';
    job.finished_at = Date.now();
  }
}

// ---- goals ------------------------------------------------------------------

const nowStats = (corpus, config) => {
  const records = Object.values(corpus.issues || {});
  const base = A.baselines(records);
  return { records, base, stats: A.assigneeStats(records, base, config.workweek, Date.now()) };
};

function loadGoals(account, project) {
  const g = store.readJson(store.goalsPath(DATA_DIR, account, project), null);
  return g && g.assignees ? g : { assignees: {} };
}

function appendGoalSnapshots(account, project, corpus, config) {
  const goals = loadGoals(account, project);
  const { stats } = nowStats(corpus, config);
  for (const s of stats) {
    const slot = (goals.assignees[s.assignee_id] ||= { goals: [], snapshots: [] });
    slot.snapshots.push({
      scanned_at: corpus.scanned_at,
      values: {
        median_cycle_days: s.median_cycle,
        median_impl_days: s.median_impl,
        median_design_days: s.median_design,
        estimate_mape: s.mape,
        avg_wip: s.avg_wip,
      },
    });
    if (slot.snapshots.length > 100) slot.snapshots = slot.snapshots.slice(-100);
  }
  store.writeJsonAtomic(store.goalsPath(DATA_DIR, account, project), goals);
}

// ---- view assembly ----------------------------------------------------------

function loadCorpus(account, project) {
  return store.readJson(store.corpusPath(DATA_DIR, account, project), null);
}

function openTaskRow(r, base, factors, config) {
  const prediction = A.predict(r, base, factors, Date.now(), config.workweek);
  return {
    key: r.key,
    summary: r.summary,
    status: r.status,
    type: r.type,
    points: r.points,
    assignee_id: r.assignee_id,
    assignee_name: r.assignee_name,
    design_days: r.design_days,
    impl_days: r.impl_days,
    prediction,
    projected_done_at: prediction ? prediction.projected_done_at : null,
    pct_consumed: prediction ? prediction.pct_consumed : null,
  };
}

function overview(account, project) {
  const corpus = loadCorpus(account, project);
  const config = loadConfig();
  if (!corpus) return null;
  const { records, base, stats } = nowStats(corpus, config);
  const factors = A.assigneeFactor(records, base);
  const completed = records.filter((r) => r.done_at !== null);
  const open = records.filter((r) => r.done_at === null);
  const flags = {};
  for (const r of completed) for (const f of r.flags || []) flags[f] = (flags[f] || 0) + 1;
  const onTrack = completed.filter((r) => {
    const hit = base.lookup(r.type, r.points);
    const v = hit ? A.verdict(r.cycle_days, hit.bucket.total.p50) : null;
    return v === 'fast' || v === 'on_track';
  }).length;
  return {
    scanned_at: corpus.scanned_at || null,
    capped: !!corpus.capped,
    target_used: corpus.target_used || {},
    completed: completed.length,
    open: open.length,
    on_track: onTrack,
    baseline_n: base.completed_n,
    assignees: stats,
    team: A.teamMedians(stats),
    baseline: base.buckets.filter((b) => b.n >= 2),
    flags,
    open_tasks: open
      .map((r) => openTaskRow(r, base, factors, config))
      .sort((a, b) => (a.assignee_name || '').localeCompare(b.assignee_name || '') || a.key.localeCompare(b.key)),
  };
}

function assigneeView(account, project, assigneeId) {
  const corpus = loadCorpus(account, project);
  const config = loadConfig();
  if (!corpus) return null;
  const { records, base, stats } = nowStats(corpus, config);
  const factors = A.assigneeFactor(records, base);
  const devStats = stats.find((s) => s.assignee_id === assigneeId);
  if (!devStats) return null;
  const team = A.teamMedians(stats);

  const goalsFile = loadGoals(account, project);
  const slot = goalsFile.assignees[assigneeId] || { goals: [], snapshots: [] };
  const custom = new Map(slot.goals.map((g) => [g.metric, g]));
  const goals = [];
  for (const suggestion of A.suggestGoals(devStats, team)) {
    const goal = custom.get(suggestion.metric) || suggestion;
    const progress = A.goalProgress(goal, devStats);
    goals.push({
      metric: goal.metric,
      target: goal.target,
      suggested: !custom.has(goal.metric),
      current: progress.current,
      met: progress.met,
      history: slot.snapshots.slice(-20).map((s) => ({ at: s.scanned_at, value: s.values[goal.metric] ?? null })),
    });
  }
  // Custom goals for metrics without an auto-suggestion still show.
  for (const g of slot.goals) {
    if (!goals.some((x) => x.metric === g.metric)) {
      const progress = A.goalProgress(g, devStats);
      goals.push({
        metric: g.metric,
        target: g.target,
        suggested: false,
        current: progress.current,
        met: progress.met,
        history: slot.snapshots.slice(-20).map((s) => ({ at: s.scanned_at, value: s.values[g.metric] ?? null })),
      });
    }
  }

  const mine = records.filter((r) => r.assignee_id === assigneeId);
  const completed = mine
    .filter((r) => r.done_at !== null)
    .sort((a, b) => b.done_at - a.done_at)
    .map((r) => {
      const hit = base.lookup(r.type, r.points);
      const bucket = hit ? hit.bucket : null;
      return {
        ...r,
        baseline: bucket
          ? { level: hit.level, n: bucket.n, design: bucket.design, impl: bucket.impl, total: bucket.total }
          : null,
        verdicts: bucket
          ? {
              design: A.verdict(r.design_days, bucket.design.p50),
              impl: A.verdict(r.impl_days, bucket.impl.p50),
              total: A.verdict(r.cycle_days, bucket.total.p50),
            }
          : { design: null, impl: null, total: null },
      };
    });
  const open = mine
    .filter((r) => r.done_at === null)
    .map((r) => ({ ...openTaskRow(r, base, factors, config), intervals: r.intervals }));

  return { account, project, stats: devStats, team, goals, completed, open };
}

// ---- AI coach ---------------------------------------------------------------

async function coach(account, project, assigneeId) {
  const corpus = loadCorpus(account, project);
  const config = loadConfig();
  if (!corpus) throw new Error('no scan yet');
  const { stats } = nowStats(corpus, config);
  const team = A.teamMedians(stats);
  const scope = assigneeId ? stats.filter((s) => s.assignee_id === assigneeId) : stats;
  const lines = scope.map(
    (s) =>
      `- ${s.assignee_name}: completed=${s.completed} wip=${s.wip} median_design=${s.median_design}d median_impl=${s.median_impl}d median_cycle=${s.median_cycle}d vs_team_factor=${s.factor ?? 'n/a'} estimate_error=${s.mape ?? 'n/a'} avg_wip=${s.avg_wip ?? 'n/a'} trend=${s.trend ?? 'n/a'} flags=${JSON.stringify(s.flags)}`,
  );
  const prompt = `You are an engineering-delivery coach for a team lead. Data below comes from Jira changelog analysis (business days, split into design vs implementation phases) and git delivery signals for project ${project}. Team medians: ${JSON.stringify(team)}.\n\nPer-developer stats:\n${lines.join('\n')}\n\nGive concise, concrete coaching: for each developer, 2-3 specific observations (phase imbalance, estimation accuracy, WIP habits, trend) and one actionable goal. Avoid generic advice.`;
  const r = await hostPost('/agents/run', { prompt });
  return { summary: r && r.text ? r.text : '' };
}

// ---- routing ----------------------------------------------------------------

function send(res, code, obj) {
  res.writeHead(code, { 'Content-Type': 'application/json' });
  res.end(JSON.stringify(obj));
}

function readBody(req) {
  return new Promise((resolve) => {
    let b = '';
    req.on('data', (c) => (b += c));
    req.on('end', () => {
      try {
        resolve(b ? JSON.parse(b) : {});
      } catch {
        resolve({});
      }
    });
  });
}

const GOAL_METRICS = new Set(A.GOAL_METRICS);

const server = http.createServer(async (req, res) => {
  const u = new URL(req.url, 'http://localhost');
  const q = u.searchParams;
  try {
    if (u.pathname === '/health') return send(res, 200, { ok: true });

    if (u.pathname === '/accounts' && req.method === 'GET') {
      return send(res, 200, await hostGet('/jira/accounts'));
    }

    if (u.pathname === '/projects' && req.method === 'GET') {
      const creds = await hostGet(`/jira/credentials?account=${encodeURIComponent(q.get('account') || '')}`);
      return send(res, 200, await makeClient(creds).searchProjects(q.get('query') || ''));
    }

    if (u.pathname === '/statuses' && req.method === 'GET') {
      const project = q.get('project') || '';
      const creds = await hostGet(`/jira/credentials?account=${encodeURIComponent(q.get('account') || '')}`);
      const statuses = await makeClient(creds).projectStatuses(project);
      const map = loadConfig().status_map[project] || {};
      return send(
        res,
        200,
        statuses.map((s) => ({ ...s, mapped: A.classifyStatus(s.name, map) })),
      );
    }

    if (u.pathname === '/config' && req.method === 'GET') {
      return send(res, 200, loadConfig());
    }
    if (u.pathname === '/config' && req.method === 'PUT') {
      const body = await readBody(req);
      let cfg;
      try {
        cfg = validateConfig(body);
      } catch (e) {
        return send(res, 400, { error: e.message });
      }
      store.writeJsonAtomic(store.configPath(DATA_DIR), cfg);
      recomputeCorpora(cfg); // status-map edits re-derive phases locally — no Jira refetch
      return send(res, 200, cfg);
    }

    if (u.pathname === '/scan' && req.method === 'POST') {
      const body = await readBody(req);
      const { account, project } = body;
      if (!account || !project) return send(res, 400, { error: 'account and project are required' });
      const key = jobKey(account, project);
      const existing = jobs.get(key);
      if (existing && existing.state === 'running') return send(res, 409, { error: 'scan already running' });
      const job = {
        state: 'running',
        step: 'starting',
        fetched: 0,
        total: null,
        retries: 0,
        errors: 0,
        started_at: Date.now(),
        finished_at: null,
        error: null,
        last_scan: existing ? existing.last_scan : null,
      };
      jobs.set(key, job);
      runScan(account, project, !!body.full); // fire and forget; job records progress
      return send(res, 200, { started: true });
    }

    if (u.pathname === '/scan/status' && req.method === 'GET') {
      const key = jobKey(q.get('account') || '', q.get('project') || '');
      const job = jobs.get(key);
      if (job) return send(res, 200, job);
      const corpus = loadCorpus(q.get('account') || '', q.get('project') || '');
      return send(res, 200, { state: 'idle', last_scan: corpus ? corpus.scanned_at : null });
    }

    if (u.pathname === '/overview' && req.method === 'GET') {
      const o = overview(q.get('account') || '', q.get('project') || '');
      return o ? send(res, 200, o) : send(res, 404, { error: 'no scan for this project yet' });
    }

    if (u.pathname === '/assignee' && req.method === 'GET') {
      const v = assigneeView(q.get('account') || '', q.get('project') || '', q.get('assignee') || '');
      return v ? send(res, 200, v) : send(res, 404, { error: 'unknown assignee or no scan yet' });
    }

    if (u.pathname === '/goals' && req.method === 'PUT') {
      const body = await readBody(req);
      const { account, project, assignee, goals } = body;
      if (!account || !project || !assignee || !Array.isArray(goals)) {
        return send(res, 400, { error: 'account, project, assignee, goals[] required' });
      }
      for (const g of goals) {
        if (!GOAL_METRICS.has(g.metric)) return send(res, 400, { error: `unknown metric ${g.metric}` });
        const t = Number(g.target);
        if (!Number.isFinite(t) || t <= 0) return send(res, 400, { error: 'target must be a positive number' });
      }
      const file = loadGoals(account, project);
      const slot = (file.assignees[assignee] ||= { goals: [], snapshots: [] });
      for (const g of goals) {
        slot.goals = slot.goals.filter((x) => x.metric !== g.metric);
        slot.goals.push({ metric: g.metric, target: Number(g.target), set_at: Date.now() });
      }
      store.writeJsonAtomic(store.goalsPath(DATA_DIR, account, project), file);
      return send(res, 200, { saved: slot.goals });
    }

    if (u.pathname === '/analyze' && req.method === 'POST') {
      const body = await readBody(req);
      return send(res, 200, await coach(body.account, body.project, body.assignee || null));
    }

    return send(res, 404, { error: 'not found' });
  } catch (e) {
    // Log the real error server-side; never leak details to the client.
    console.error('request failed:', e);
    return send(res, 500, { error: 'internal error' });
  }
});

server.listen(PORT, '127.0.0.1', () => {
  console.log(`team-performance sidecar on :${PORT}`);
});
