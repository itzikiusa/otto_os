// team-performance — Otto runtime plugin (Node sidecar, zero dependencies).
//
// Otto spawns this with: OTTO_PLUGIN_PORT (bind here), OTTO_PLUGIN_TOKEN +
// OTTO_HOST_API (call back for repos/jira/agents), OTTO_PLUGIN_DATA_DIR (state).
// Otto reverse-proxies /api/v1/plugins/team-performance/* to these routes.
//
// What it does: scans Jira projects (multi-project, unlimited size, paced so
// Jira never rate-limits us; optionally scoped to selected people), derives
// per-task timing with GIT as the primary signal (first commit → merge to
// develop/release = done; commits after = fixes; *-DEPLOYED* tag = prod) and
// the Jira changelog as the secondary indication, adds a 3-level estimate per
// story (dev-agnostic AI estimate → per-dev expected → actual), detects
// routine work, splits multi-dev credit by commit share, extracts git-only
// features from opted-in repos (work without Jira stories), and tracks goals
// at team / role / developer level. Analytics live in lib/ (pure,
// unit-tested); this file is env, routing, and the async scan job.

const http = require('http');
const path = require('path');
const { spawn } = require('child_process');
const { URL } = require('url');

const A = require('./lib/analytics.js');
const { makeClient, detectPointsField, adfToText } = require('./lib/jira.js');
const E = require('./lib/estimates.js');
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

/** Registered repos, deduped by path (the registry may hold duplicates). */
async function hostRepos() {
  const repos = (await hostGet('/repos')) || [];
  const seen = new Set();
  return repos.filter((r) => (seen.has(r.path) ? false : (seen.add(r.path), true)));
}

/**
 * Build the git index in a CHILD process (lib/gitscan.js worker mode): the
 * walk is blocking execFileSync end to end, and fetch across many repos can
 * hold the CPU for minutes — a child keeps this event loop (scan status,
 * views) responsive. 20-minute cap.
 */
function buildIndexAsync(repos, config) {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [path.join(__dirname, 'lib', 'gitscan.js')], {
      stdio: ['pipe', 'pipe', 'inherit'],
    });
    const timer = setTimeout(() => {
      child.kill('SIGKILL');
      reject(new Error('git index timed out'));
    }, 20 * 60 * 1000);
    let out = '';
    child.stdout.on('data', (c) => (out += c));
    child.on('error', (e) => {
      clearTimeout(timer);
      reject(e);
    });
    child.on('close', (code) => {
      clearTimeout(timer);
      if (code !== 0) return reject(new Error(`git index worker exited ${code}`));
      try {
        const idx = JSON.parse(out);
        resolve({
          byKey: new Map(Object.entries(idx.by_key || {})),
          features: idx.features || [],
          unscoped: idx.unscoped || [],
          target_used: idx.target_used || {},
          fetched: idx.fetched || {},
          hasRepos: Boolean(idx.hasRepos),
        });
      } catch {
        reject(new Error('git index worker returned bad JSON'));
      }
    });
    child.stdin.end(JSON.stringify({ repos, config }));
  });
}

// ---- config -----------------------------------------------------------------

const DEFAULT_ROLES = ['Developer', 'Senior Developer', 'Team Lead', 'QA/Automation', 'Product Manager', 'VP RND'];

const DEFAULT_CONFIG = {
  v2: true,
  issue_types: [], // empty = ALL issue types
  target_branches: ['develop', 'main', 'master'],
  workweek: [1, 2, 3, 4, 5],
  max_issues: 0, // 0 = unlimited (pacing keeps Jira happy)
  git_depth: 0, // 0 = full history
  pace_ms: 150,
  git_fetch: true,
  deploy_tag_pattern: 'deployed',
  stale_days: 45,
  estimate_enabled: true,
  estimate_window_months: 6,
  estimate_since: '', // ISO date; when set it wins over the month window
  estimate_max_batches: 40,
  estimate_workers: [{ provider: 'claude', model: '' }],
  feature_repos: [],
  auto_scan_minutes: 15, // 0 = off — silently re-runs the last scan's params
  roles: DEFAULT_ROLES,
  status_map: {},
};

function loadConfig() {
  const raw = store.readJson(store.configPath(DATA_DIR), {});
  const c = { ...DEFAULT_CONFIG, ...(raw && typeof raw === 'object' ? raw : {}) };
  // One-time v2 migration: the old defaults capped the corpus and filtered
  // types — v2 wants everything (moderated), so reset those three knobs once.
  if (raw && typeof raw === 'object' && Object.keys(raw).length && !raw.v2) {
    c.v2 = true;
    c.issue_types = [];
    c.max_issues = 0;
    c.git_depth = 0;
    store.writeJsonAtomic(store.configPath(DATA_DIR), c);
  }
  return c;
}

const PHASE_VALUES = ['design', 'implementation', 'waiting', 'excluded'];
const PROVIDERS = ['claude', 'codex'];

function validateConfig(body) {
  const c = { ...loadConfig() };
  if (body.issue_types !== undefined) {
    if (!Array.isArray(body.issue_types) || !body.issue_types.every((t) => typeof t === 'string')) {
      throw new Error('issue_types must be a string array (empty = all types)');
    }
    c.issue_types = body.issue_types.map((t) => t.trim()).filter(Boolean);
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
  for (const [k, lo, hi] of [
    ['max_issues', 0, 1000000],
    ['git_depth', 0, 1000000],
    ['pace_ms', 0, 5000],
    ['stale_days', 5, 365],
    ['estimate_window_months', 0, 60],
    ['estimate_max_batches', 1, 200],
    ['auto_scan_minutes', 0, 1440],
  ]) {
    if (body[k] !== undefined) {
      const n = Number(body[k]);
      if (!Number.isInteger(n) || n < lo || n > hi) throw new Error(`${k} must be an integer in ${lo}..${hi}`);
      c[k] = n;
    }
  }
  for (const k of ['git_fetch', 'estimate_enabled']) {
    if (body[k] !== undefined) c[k] = Boolean(body[k]);
  }
  if (body.deploy_tag_pattern !== undefined) {
    const p = String(body.deploy_tag_pattern).trim();
    if (!p || p.length > 100) throw new Error('deploy_tag_pattern must be a short non-empty substring');
    c.deploy_tag_pattern = p;
  }
  if (body.estimate_since !== undefined) {
    const s = String(body.estimate_since).trim();
    if (s && Number.isNaN(Date.parse(s))) throw new Error('estimate_since must be an ISO date (or empty)');
    c.estimate_since = s;
  }
  if (body.estimate_workers !== undefined) {
    if (!Array.isArray(body.estimate_workers) || !body.estimate_workers.length || body.estimate_workers.length > 8) {
      throw new Error('estimate_workers must be 1..8 entries');
    }
    c.estimate_workers = body.estimate_workers.map((w) => {
      if (!w || !PROVIDERS.includes(w.provider)) throw new Error(`estimate_workers provider must be one of ${PROVIDERS.join('/')}`);
      return { provider: w.provider, model: String(w.model || '').trim().slice(0, 60) };
    });
  }
  if (body.feature_repos !== undefined) {
    if (!Array.isArray(body.feature_repos) || !body.feature_repos.every((r) => typeof r === 'string')) {
      throw new Error('feature_repos must be a string array of repo names');
    }
    c.feature_repos = body.feature_repos.map((r) => r.trim()).filter(Boolean);
  }
  if (body.roles !== undefined) {
    if (!Array.isArray(body.roles) || !body.roles.length || !body.roles.every((r) => typeof r === 'string' && r.trim())) {
      throw new Error('roles must be a non-empty string array');
    }
    c.roles = [...new Set(body.roles.map((r) => r.trim()))];
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

/** Recompute phase-dependent fields of every stored corpus after a config change. */
function recomputeCorpora(config) {
  for (const file of store.listCorpora(DATA_DIR)) {
    const corpus = store.readJson(file, null);
    if (!corpus || !corpus.issues) continue;
    const statusMap = config.status_map[corpus.project] || {};
    const records = Object.values(corpus.issues);
    const hasDesign = projectHasDesign(records, statusMap);
    const hasRepos = Object.keys(corpus.target_used || {}).length > 0;
    for (const r of records) {
      corpus.issues[r.key] = A.reanalyzeRecord(r, {
        statusMap,
        workweek: config.workweek,
        hasDesignStatuses: hasDesign,
        hasRepos,
        staleDays: config.stale_days,
      });
    }
    store.writeJsonAtomic(file, corpus);
  }
}

// ---- people registry ---------------------------------------------------------

function loadPeople() {
  const p = store.readJson(store.peoplePath(DATA_DIR), null);
  return p && p.people ? p : { people: {} };
}

function savePeople(reg) {
  store.writeJsonAtomic(store.peoplePath(DATA_DIR), reg);
}

/** Ensure every discovered assignee exists in the registry (included by default). */
function seedPeopleFromRecords(records) {
  const reg = loadPeople();
  let dirty = false;
  for (const r of records) {
    if (!r.assignee_id || reg.people[r.assignee_id]) continue;
    reg.people[r.assignee_id] = { name: r.assignee_name || r.assignee_id, role: '', included: true, aliases: [] };
    dirty = true;
  }
  if (dirty) savePeople(reg);
  return reg;
}

// ---- scan job (one per account; loops the selected projects) -----------------

const jobs = new Map(); // account -> status object

// ---- auto-scan cron -----------------------------------------------------------
// Every `auto_scan_minutes` the sidecar silently re-runs the LAST manual scan's
// parameters incrementally: created tickets are fetched + estimated, updated
// ones re-derive their timelines — no manual scanning needed. The last params
// persist across restarts (data/last_scan.json).

const lastScanParamsPath = () => require('path').join(DATA_DIR, 'last_scan.json');

function rememberScanParams(account, projects, assignees) {
  store.writeJsonAtomic(lastScanParamsPath(), { account, projects, assignees: assignees || null, at: Date.now() });
}

function maybeAutoScan() {
  const config = loadConfig();
  if (!config.auto_scan_minutes) return;
  const params = store.readJson(lastScanParamsPath(), null);
  if (!params || !params.account || !Array.isArray(params.projects) || !params.projects.length) return;
  const job = jobs.get(params.account);
  if (job && job.state === 'running') return;
  const intervalMs = Number(process.env.OTTO_TP_AUTOSCAN_MS) || config.auto_scan_minutes * 60000;
  const lastFinished = job && job.finished_at ? job.finished_at : params.at || 0;
  if (Date.now() - lastFinished < intervalMs) return;
  jobs.set(params.account, {
    state: 'running', step: 'starting', auto: true,
    project: params.projects[0], project_i: 0, project_n: params.projects.length,
    fetched: 0, total: null, retries: 0, pace_ms: null, errors: 0, estimate_remaining: 0,
    started_at: Date.now(), finished_at: null, error: null, full: false,
    scoped_people: params.assignees ? params.assignees.length : 0,
    last_scan: job ? job.last_scan : null,
  });
  runScan(params.account, params.projects, false, params.assignees || null);
}

setInterval(maybeAutoScan, Number(process.env.OTTO_TP_AUTOSCAN_MS) ? 500 : 60000).unref();

const SEARCH_FIELDS_BASE = ['summary', 'description', 'issuetype', 'status', 'assignee', 'created', 'resolutiondate', 'updated', 'timeoriginalestimate', 'parent'];

function fmtJqlUtc(ms) {
  const d = new Date(ms);
  const p = (n) => String(n).padStart(2, '0');
  return `${d.getUTCFullYear()}-${p(d.getUTCMonth() + 1)}-${p(d.getUTCDate())} ${p(d.getUTCHours())}:${p(d.getUTCMinutes())}`;
}

const esc = (s) => String(s).replace(/["\\]/g, '');

async function scanProject(client, account, project, full, assignees, config, gitIndex, job) {
  const scanStart = Date.now();
  const corpusFile = store.corpusPath(DATA_DIR, account, project);
  const corpus = (!full && store.readJson(corpusFile, null)) || { project, account, issues: {} };

  job.step = 'fields';
  const pointsField = corpus.points_field || detectPointsField(await client.fields());

  let jql = `project = "${esc(project)}"`;
  if (config.issue_types.length) {
    jql += ` AND issuetype IN (${config.issue_types.map((t) => `"${esc(t)}"`).join(', ')})`;
  }
  if (assignees && assignees.length) {
    jql += ` AND assignee IN (${assignees.map((a) => `"${esc(a)}"`).join(', ')})`;
  }
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
      job.pace_ms = client.paceMs;
    },
  });
  const capped = config.max_issues > 0 && found.length >= config.max_issues;

  // Issues whose changelog fetch failed last scan would otherwise be lost
  // until touched again in Jira (the watermark JQL excludes them) — union
  // them into this scan's fetch set.
  for (const key of corpus.fetch_failed || []) {
    if (!found.some((s) => s.key === key)) found.push({ key, fields: {} });
  }

  job.step = 'changelogs';
  job.total = found.length;
  job.fetched = 0;
  const rawIssues = [];
  const failedKeys = [];
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
      failedKeys.push(stub.key);
      console.error(`scan: issue ${stub.key} failed:`, e.message);
    }
    job.fetched++;
    job.retries = client.retries;
    job.pace_ms = client.paceMs;
  }

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
      staleDays: config.stale_days,
      nowMs,
      descText: adfToText,
    });
  }
  const records = Object.values(corpus.issues);
  const hasDesign = projectHasDesign(records, statusMap);
  for (const r of records) {
    // Re-derive git-primary timing on EVERY record (also the unchanged ones —
    // new commits/tags may have landed since their last Jira update).
    let rec = A.deriveGit(r, gitIndex.byKey.get(r.key), {
      workweek: config.workweek,
      hasRepos: gitIndex.hasRepos,
      staleDays: config.stale_days,
    });
    if (!hasDesign) rec = { ...rec, flags: rec.flags.filter((f) => f !== 'skipped_design') };
    corpus.issues[rec.key] = rec;
  }

  job.step = 'persist';
  corpus.points_field = pointsField;
  corpus.scanned_at = Date.now();
  corpus.last_scan_start = scanStart;
  // A small incremental fetch must not clear the banner while the corpus is
  // still the truncated set from an earlier capped scan.
  corpus.capped = full ? capped : Boolean(corpus.capped) || capped;
  corpus.fetch_failed = failedKeys;
  corpus.target_used = gitIndex.target_used;
  corpus.scan_scope = assignees && assignees.length ? assignees : null;
  store.writeJsonAtomic(corpusFile, corpus);
  seedPeopleFromRecords(records);
  return corpus;
}

/** Agent runner for estimation workers: provider+model routed via the host. */
function agentRunner() {
  return async (prompt, worker) => {
    const r = await hostPost('/agents/run', {
      prompt,
      provider: worker && worker.provider ? worker.provider : 'claude',
      model: worker && worker.model ? worker.model : undefined,
    });
    return r && r.text ? r.text : '';
  };
}

/** Git features → estimation-record shape (shared cache/prompt machinery). */
function featureAsRecord(f) {
  return {
    key: f.id,
    type: 'GitFeature',
    points: null,
    summary: `${f.repo}: ${f.summary}`,
    description_snippet: (f.subjects || []).join('; ').slice(0, 600),
    eff_done_at: f.merged_at,
    done_at: f.merged_at,
    updated: f.merged_at,
  };
}

async function estimateScope(account, projects, config, job) {
  const agentRun = agentRunner();
  const workers = config.estimate_workers;
  // Estimation spends real agent calls — cover only the people the registry
  // includes (new assignees are auto-included at discovery, so a new hire's
  // tickets estimate on the next cron tick; excluded people never burn
  // batches). Unassigned/open tickets always estimate.
  const reg = loadPeople();
  const canonical = A.makeCanonical(reg.people);
  const included = (id) => {
    if (!id) return true;
    const p = reg.people[canonical(id)];
    return !p || p.included !== false;
  };
  for (const project of projects) {
    const corpus = store.readJson(store.corpusPath(DATA_DIR, account, project), null);
    if (!corpus || !corpus.issues) continue;
    const cacheFile = store.estimatesPath(DATA_DIR, account, project);
    const cache = store.readJson(cacheFile, {}) || {};
    delete cache.schema;
    job.step = 'estimate';
    job.project = project;
    // Epic/parent context helps the estimator size stories that are one slice
    // of a bigger system (prompt-only — not part of the content hash).
    const byKey = corpus.issues;
    const withHints = Object.values(corpus.issues).map((r) => {
      const parent = r.parent_key ? byKey[r.parent_key] : null;
      return parent ? { ...r, epic_hint: parent.summary } : r;
    });
    const res = await E.runEstimation({
      records: withHints.filter((r) => included(r.assignee_id)),
      cache,
      windowMonths: config.estimate_window_months,
      sinceMs: config.estimate_since ? Date.parse(config.estimate_since) || 0 : 0,
      maxBatches: config.estimate_max_batches,
      workers,
      agentRun,
      onProgress: (done, total) => {
        job.fetched = done;
        job.total = total;
      },
    });
    store.writeJsonAtomic(cacheFile, cache);
    job.estimate_remaining = (job.estimate_remaining || 0) + res.remaining;
    job.errors = (job.errors || 0) + res.failed_batches;
  }
  // Git-only features share the same machinery under a synthetic project.
  const featFile = store.featuresPath(DATA_DIR);
  const feats = store.readJson(featFile, null);
  if (feats && Array.isArray(feats.features) && feats.features.length) {
    const cacheFile = store.estimatesPath(DATA_DIR, account, '__features__');
    const cache = store.readJson(cacheFile, {}) || {};
    delete cache.schema;
    job.step = 'estimate features';
    const res = await E.runEstimation({
      records: feats.features.filter((f) => !(f.jira_keys || []).length).map(featureAsRecord),
      cache,
      windowMonths: config.estimate_window_months,
      sinceMs: config.estimate_since ? Date.parse(config.estimate_since) || 0 : 0,
      maxBatches: config.estimate_max_batches,
      workers,
      agentRun,
      onProgress: (done, total) => {
        job.fetched = done;
        job.total = total;
      },
    });
    store.writeJsonAtomic(cacheFile, cache);
    job.estimate_remaining = (job.estimate_remaining || 0) + res.remaining;
  }
}

async function runScan(account, projects, full, assignees) {
  const job = jobs.get(account);
  try {
    const config = loadConfig();
    const creds = await hostGet(`/jira/credentials?account=${encodeURIComponent(account)}`);
    const client = makeClient(creds, { paceMs: config.pace_ms });

    // One git pass for the whole scan — every project shares the same repos.
    job.step = config.git_fetch ? 'git fetch + index' : 'git index';
    const repos = await hostRepos();
    const gitIndex = await buildIndexAsync(repos, config);
    // Always persisted: git-only features (opted-in repos) + unscoped fix work
    // (ABC-0000-style commits — real work that belongs to no story).
    store.writeJsonAtomic(store.featuresPath(DATA_DIR), {
      features: gitIndex.features,
      unscoped: gitIndex.unscoped || [],
      scanned_at: Date.now(),
    });

    job.project_n = projects.length;
    for (const [i, project] of projects.entries()) {
      job.project = project;
      job.project_i = i + 1;
      job.fetched = 0;
      job.total = null;
      await scanProject(client, account, project, full, assignees, config, gitIndex, job);
    }

    if (config.estimate_enabled) await estimateScope(account, projects, config, job);

    appendGoalSnapshots(account, config);

    job.state = 'done';
    job.finished_at = Date.now();
    job.last_scan = Date.now();
  } catch (e) {
    console.error('scan failed:', e);
    job.state = 'error';
    job.error = 'scan failed — see plugin logs';
    job.finished_at = Date.now();
  }
}

// ---- scope loading (multi-project views) --------------------------------------

function applyOverrides(records, overrides) {
  if (!overrides || !overrides.issues) return records;
  return records.map((r) => {
    const o = overrides.issues[r.key];
    return o
      ? {
          ...r,
          outlier: o.outlier === true,
          manual_days: typeof o.manual_days === 'number' ? o.manual_days : null,
          excluded_override: o.excluded === true,
        }
      : r;
  });
}

/** Load estimates for a project as a plain {key: {days, routine}} map. */
function loadEstimates(account, project) {
  const m = store.readJson(store.estimatesPath(DATA_DIR, account, project), {}) || {};
  delete m.schema;
  return m;
}

/**
 * Resolve a view scope: the selected projects' corpora merged, overrides and
 * estimates applied, people registry + author matcher ready.
 */
function loadScope(account, projectsParam) {
  const all = store.listProjects(DATA_DIR, account).filter((p) => p !== '__features__');
  let projects = String(projectsParam || '')
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean);
  if (!projects.length || projectsParam === '*') projects = all;
  projects = projects.filter((p) => all.includes(p));
  if (!projects.length) return null;

  const config = loadConfig();
  const scanned = {};
  const targetUsed = {};
  let capped = false;
  let records = [];
  const estimates = {};
  for (const p of projects) {
    const corpus = store.readJson(store.corpusPath(DATA_DIR, account, p), null);
    if (!corpus || !corpus.issues) continue;
    scanned[p] = corpus.scanned_at || null;
    capped = capped || Boolean(corpus.capped);
    Object.assign(targetUsed, corpus.target_used || {});
    const overrides = store.readJson(store.overridesPath(DATA_DIR, account, p), null);
    records = records.concat(applyOverrides(Object.values(corpus.issues), overrides));
    Object.assign(estimates, loadEstimates(account, p));
  }
  // Hierarchy pass: dev sub-tasks roll up into their parent story; design
  // sub-tasks paint the parent's design phase.
  records = A.enrichHierarchy(records);

  // Keyless git features (opted-in repos — automation work without tickets)
  // join the corpus as pseudo-records: their authors get weighted/pace/monthly
  // credit via commit share, exactly like ticketed work. They carry no
  // assignee, so Jira task counts and medians stay untouched.
  const featFile = store.readJson(store.featuresPath(DATA_DIR), null);
  if (featFile && Array.isArray(featFile.features)) {
    Object.assign(estimates, loadEstimates(account, '__features__'));
    for (const f of featFile.features) {
      if ((f.jira_keys || []).length) continue;
      const impl = Math.round(A.businessDays(f.first_commit_at, f.merged_at, config.workweek) * 100) / 100;
      records.push({
        key: f.id, project: '__git__', type: 'GitFeature', feature: true,
        summary: `${f.repo}: ${f.summary}`, description_snippet: '',
        subtask: false, parent_key: null,
        assignee_id: null, assignee_name: null,
        status: 'Merged', status_category: 'done',
        created: f.first_commit_at, points: null, estimate_days: null,
        intervals: [], design_days: 0, impl_days: null, wait_days: 0,
        first_active_at: f.first_commit_at, cycle_days: null, lead_days: null,
        done_at: f.merged_at, updated: f.merged_at,
        first_commit_at: f.first_commit_at, done_git_at: f.merged_at,
        delivered_at: f.merged_at, deployed_at: f.deployed_at ?? null,
        fix_count: 0, last_fix_at: null, git_authors: f.authors || [],
        impl_days_git: impl, fix_days: null, deploy_wait_days: null,
        eff_done_at: f.merged_at, eff_start_at: f.first_commit_at,
        eff_impl_days: impl, eff_cycle_days: impl,
        timing_source: 'git', flags: [],
      });
    }
  }

  const reg = loadPeople();
  const canonical = A.makeCanonical(reg.people);
  // Flatten merged accounts: one entry per canonical person carrying every
  // merged account's name + aliases (so git authors match whichever identity).
  const flat = {};
  for (const [id, p] of Object.entries(reg.people)) {
    const cid = canonical(id);
    const root = reg.people[cid] || p;
    const slot = (flat[cid] ||= { name: root.name, role: root.role || '', included: root.included !== false, aliases: [] });
    slot.aliases.push(...(p.aliases || []));
    if (id !== cid && p.name) slot.aliases.push(p.name);
  }
  const included = (id) => {
    const cid = canonical(id);
    return !flat[cid] || flat[cid].included !== false;
  };
  // Excluded people disappear from the analysis entirely (their tasks would
  // otherwise poison baselines with non-engineering work).
  const visible = records.filter((r) => !r.assignee_id || included(r.assignee_id));
  const matcher = A.makeAuthorMatcher(Object.fromEntries(Object.entries(flat).filter(([, p]) => p.included !== false)));
  return {
    account, projects, all_projects: all, config, scanned, capped, target_used: targetUsed,
    records: visible, all_records: records, estimates, people: reg.people, flat_people: flat, canonical, matcher,
  };
}

const nowStats = (scope, sinceMs = 0, untilMs = 0) => {
  const base = A.baselines(scope.records);
  const sigCounts = A.routineSignatures(scope.records);
  const stats = A.assigneeStats(scope.records, base, scope.config.workweek, Date.now(), {
    estimates: scope.estimates,
    matcher: scope.matcher,
    sigCounts,
    sinceMs,
    untilMs,
    canonical: scope.canonical,
  });
  // Only people the registry includes (contributor credit may add ids).
  const visibleStats = stats.filter((s) => !scope.flat_people[s.assignee_id] || scope.flat_people[s.assignee_id].included !== false);
  return { base, sigCounts, stats: visibleStats };
};

/** Effective completion time within the current period? (view-side helper) */
const inViewPeriod = (r, sinceMs) => !sinceMs || (r.eff_done_at ?? r.done_at ?? 0) >= sinceMs;

// ---- goals ------------------------------------------------------------------

const DEV_GOALS_SLOT = '__devs__';

function loadDevGoals(account) {
  const g = store.readJson(store.goalsPath(DATA_DIR, account, DEV_GOALS_SLOT), null);
  if (g && g.assignees) return g;
  // Migrate legacy per-project goal files (first writer wins per metric).
  const merged = { assignees: {} };
  for (const p of store.listProjects(DATA_DIR, account)) {
    const legacy = store.readJson(store.goalsPath(DATA_DIR, account, p), null);
    if (!legacy || !legacy.assignees) continue;
    for (const [id, slot] of Object.entries(legacy.assignees)) {
      const target = (merged.assignees[id] ||= { goals: [], snapshots: [] });
      for (const goal of slot.goals || []) {
        if (!target.goals.some((x) => x.metric === goal.metric)) target.goals.push(goal);
      }
      if ((slot.snapshots || []).length > (target.snapshots || []).length) target.snapshots = slot.snapshots;
    }
  }
  return merged;
}

function saveDevGoals(account, file) {
  store.writeJsonAtomic(store.goalsPath(DATA_DIR, account, DEV_GOALS_SLOT), file);
}

function appendGoalSnapshots(account, config) {
  const scope = loadScope(account, '*');
  if (!scope) return;
  const { stats } = nowStats(scope);
  const goals = loadDevGoals(account);
  const at = Date.now();
  for (const s of stats) {
    const slot = (goals.assignees[s.assignee_id] ||= { goals: [], snapshots: [] });
    slot.snapshots.push({
      scanned_at: at,
      values: {
        median_cycle_days: s.median_cycle,
        median_impl_days: s.median_impl,
        median_design_days: s.median_design,
        estimate_mape: s.mape,
        avg_wip: s.avg_wip,
        weighted_throughput: s.weighted_done,
        efficiency: s.efficiency,
      },
    });
    if (slot.snapshots.length > 100) slot.snapshots = slot.snapshots.slice(-100);
  }
  saveDevGoals(account, goals);
}

function loadScopeGoals(account) {
  const g = store.readJson(store.scopeGoalsPath(DATA_DIR, account), null);
  return g && (g.team || g.roles) ? { team: g.team || [], roles: g.roles || {} } : { team: [], roles: {} };
}

/** Evaluated goals for one dev (custom ∪ suggested) against current stats. */
function devGoalRows(slot, devStats, team) {
  const custom = new Map((slot.goals || []).map((g) => [g.metric, g]));
  const rows = [];
  for (const suggestion of A.suggestGoals(devStats, team)) {
    const goal = custom.get(suggestion.metric) || suggestion;
    const progress = A.goalProgress(goal, devStats);
    rows.push({
      metric: goal.metric,
      target: goal.target,
      suggested: !custom.has(goal.metric),
      current: progress.current,
      met: progress.met,
      dir: progress.dir,
      history: (slot.snapshots || []).slice(-20).map((s) => ({ at: s.scanned_at, value: s.values[goal.metric] ?? null })),
    });
  }
  for (const g of slot.goals || []) {
    if (!rows.some((x) => x.metric === g.metric)) {
      const progress = A.goalProgress(g, devStats);
      rows.push({
        metric: g.metric,
        target: g.target,
        suggested: false,
        current: progress.current,
        met: progress.met,
        dir: progress.dir,
        history: (slot.snapshots || []).slice(-20).map((s) => ({ at: s.scanned_at, value: s.values[g.metric] ?? null })),
      });
    }
  }
  return rows;
}

// ---- view assembly ----------------------------------------------------------

/** Per-task 3-level estimate: agnostic AI → per-dev expected → actual. */
function estLevels(r, estimates, factorMap) {
  const est = estimates[r.key];
  const agnostic = est && est.days > 0 ? est.days : null;
  const f = r.assignee_id && factorMap.has(r.assignee_id) ? factorMap.get(r.assignee_id).factor : 1.0;
  return {
    est_days_ai: agnostic,
    est_days_dev: agnostic !== null ? Math.round(agnostic * f * 100) / 100 : null,
    actual_days: r.manual_days ?? r.eff_cycle_days ?? r.cycle_days ?? null,
  };
}

/** Outlier suspect: way past its baseline band and not yet handled. */
function suspectOutlier(r, base) {
  if (r.outlier === true || r.manual_days != null) return false;
  const actual = r.manual_days ?? r.eff_cycle_days ?? r.cycle_days;
  if (actual === null || actual === undefined) return false;
  const hit = base.lookup(r.type, r.points);
  if (!hit || !hit.bucket.total.p75) return actual > 60;
  return actual > 3 * hit.bucket.total.p75 && actual > 10;
}

function taskGit(r) {
  return {
    first_commit_at: r.first_commit_at ?? null,
    done_git_at: r.done_git_at ?? null,
    deployed_at: r.deployed_at ?? null,
    fix_count: r.fix_count ?? 0,
    fix_days: r.fix_days ?? null,
    deploy_wait_days: r.deploy_wait_days ?? null,
    impl_days_git: r.impl_days_git ?? null,
    timing_source: r.timing_source || 'jira',
  };
}

function openTaskRow(r, base, factors, config, estimates) {
  const prediction = A.predict(r, base, factors, Date.now(), config.workweek, estimates);
  return {
    key: r.key,
    project: r.project,
    summary: r.summary,
    status: r.status,
    type: r.type,
    points: r.points,
    assignee_id: r.assignee_id,
    assignee_name: r.assignee_name,
    design_days: r.design_days,
    impl_days: r.eff_impl_days ?? r.impl_days,
    ...estLevels(r, estimates, factors),
    ...taskGit(r),
    prediction,
    projected_done_at: prediction ? prediction.projected_done_at : null,
    pct_consumed: prediction ? prediction.pct_consumed : null,
  };
}

function overview(account, projectsParam, sinceMs = 0) {
  const scope = loadScope(account, projectsParam);
  if (!scope) return null;
  const { config, records, estimates, canonical } = scope;
  const { base, sigCounts, stats } = nowStats(scope, sinceMs);
  const factors = A.assigneeFactor(records, base);
  const completed = records.filter((r) => !r.feature && A.isDone(r) && inViewPeriod(r, sinceMs));
  const measurable = completed.filter((r) => !A.isExcluded(r) && A.isTimingSample(r) && (r.eff_cycle_days ?? r.cycle_days) !== null);
  const open = records.filter((r) => !r.feature && !A.isDone(r));
  const flags = {};
  for (const r of completed) for (const f of r.flags || []) flags[f] = (flags[f] || 0) + 1;
  const onTrack = measurable.filter((r) => {
    const hit = base.lookup(r.type, r.points);
    const v = hit ? A.verdict(r.manual_days ?? r.eff_cycle_days ?? r.cycle_days, hit.bucket.total.p50) : null;
    return v === 'fast' || v === 'on_track';
  }).length;

  const team = A.teamMedians(stats);
  const scopeValues = A.scopeMetrics(records, base, config.workweek, Date.now(), estimates, sinceMs);
  const scopeGoals = loadScopeGoals(account);
  const roleOf = (id) => {
    const cid = canonical(id);
    return (scope.flat_people[cid] && scope.flat_people[cid].role) || '';
  };
  const roleGoals = {};
  for (const [role, goals] of Object.entries(scopeGoals.roles || {})) {
    const roleRecords = records.filter((r) => r.assignee_id && roleOf(r.assignee_id) === role);
    roleGoals[role] = A.evalScopeGoals(goals, A.scopeMetrics(roleRecords, base, config.workweek, Date.now(), estimates, sinceMs));
  }

  // Per-dev goal chips for the team table.
  const devGoals = loadDevGoals(account);
  const withGoals = stats.map((s) => {
    const slot = devGoals.assignees[s.assignee_id] || { goals: [], snapshots: [] };
    const rows = devGoalRows(slot, s, team);
    return {
      ...s,
      role: roleOf(s.assignee_id),
      goals_met: rows.filter((g) => g.met).length,
      goals_total: rows.length,
    };
  });

  const routineTotals = withGoals.reduce(
    (acc, s) => ({ routine: acc.routine + (s.routine_done || 0), feature: acc.feature + (s.feature_done || 0) }),
    { routine: 0, feature: 0 },
  );

  // Unscoped fix work (ABC-0000-style commits): per-person commit counts within
  // the period, matched to people via the same author matcher.
  const featFile = store.readJson(store.featuresPath(DATA_DIR), null);
  const unscopedByPerson = {};
  for (const uAuthor of (featFile && featFile.unscoped) || []) {
    const pid = scope.matcher(uAuthor.name, uAuthor.email);
    if (!pid) continue;
    const cid = canonical(pid);
    let n = 0;
    for (const [ym, count] of Object.entries(uAuthor.monthly || {})) {
      const t = Date.parse(`${ym}-01T00:00:00Z`);
      if (!sinceMs || t >= sinceMs - 31 * 86400000) n += count;
    }
    if (n > 0) unscopedByPerson[cid] = (unscopedByPerson[cid] || 0) + n;
  }
  for (const s of withGoals) s.unscoped_commits = unscopedByPerson[s.assignee_id] || 0;

  return {
    scanned: scope.scanned,
    scanned_at: Object.values(scope.scanned).length ? Math.min(...Object.values(scope.scanned).filter(Boolean)) : null,
    projects: scope.projects,
    all_projects: scope.all_projects,
    capped: scope.capped,
    target_used: scope.target_used,
    completed: completed.length,
    excluded: completed.length - measurable.length,
    open: open.length,
    on_track: onTrack,
    measurable: measurable.length,
    baseline_n: base.completed_n,
    assignees: withGoals,
    team,
    scope: scopeValues,
    scope_goals: { team: A.evalScopeGoals(scopeGoals.team, scopeValues), roles: roleGoals },
    baseline: base.buckets.filter((b) => b.n >= 2),
    flags,
    routine: routineTotals,
    unmatched_authors: A.unmatchedAuthors(records, scope.matcher).slice(0, 12),
    roles: config.roles,
    since: sinceMs || null,
    open_tasks: open
      .map((r) => openTaskRow(r, base, factors, config, estimates))
      .sort((a, b) => (a.assignee_name || '').localeCompare(b.assignee_name || '') || a.key.localeCompare(b.key)),
    suspects: measurable.filter((r) => suspectOutlier(r, base)).length + completed.filter((r) => suspectOutlier(r, base) && A.isExcluded(r)).length,
  };
}

function assigneeView(account, projectsParam, assigneeId, sinceMs = 0) {
  const scope = loadScope(account, projectsParam);
  if (!scope) return null;
  const { config, records, estimates, canonical } = scope;
  const { base, sigCounts, stats } = nowStats(scope, sinceMs);
  const factors = A.assigneeFactor(records, base);
  const devStats = stats.find((s) => s.assignee_id === assigneeId);
  if (!devStats) return null;
  const team = A.teamMedians(stats);

  const goalsFile = loadDevGoals(account);
  const slot = goalsFile.assignees[assigneeId] || { goals: [], snapshots: [] };
  const goals = devGoalRows(slot, devStats, team);

  // Hierarchy context: parent/epic names + each task's sub-task children.
  const byKey = new Map(records.map((r) => [r.key, r]));
  const childrenOf = new Map();
  for (const r of records) {
    if (!r.parent_key) continue;
    if (!childrenOf.has(r.parent_key)) childrenOf.set(r.parent_key, []);
    childrenOf.get(r.parent_key).push(r);
  }
  const lineage = (r) => {
    const parent = r.parent_key ? byKey.get(r.parent_key) : null;
    const epic = parent && parent.parent_key ? byKey.get(parent.parent_key) : parent && parent.type === 'Epic' ? parent : null;
    return {
      parent_key: r.parent_key,
      parent_summary: parent ? parent.summary : null,
      parent_type: parent ? parent.type : null,
      epic_key: epic && epic !== parent ? epic.key : parent && parent.type === 'Epic' ? parent.key : null,
      epic_summary: epic && epic !== parent ? epic.summary : parent && parent.type === 'Epic' ? parent.summary : null,
      children: (childrenOf.get(r.key) || []).map((c) => ({
        key: c.key,
        type: c.type,
        summary: c.summary,
        assignee_name: c.assignee_name,
        rollup: c.rollup === true,
        actual_days: c.manual_days ?? c.eff_cycle_days ?? c.cycle_days,
      })),
    };
  };

  const mine = records.filter((r) => canonical(r.assignee_id) === assigneeId);
  const decorate = (r) => {
    const hit = base.lookup(r.type, r.points);
    const bucket = hit ? hit.bucket : null;
    const actual = r.manual_days ?? r.eff_cycle_days ?? r.cycle_days;
    return {
      ...r,
      ...estLevels(r, estimates, factors),
      ...taskGit(r),
      ...lineage(r),
      routine: A.isRoutine(r, sigCounts, estimates[r.key]),
      excluded: A.isExcluded(r),
      suspect_outlier: suspectOutlier(r, base),
      baseline: bucket
        ? { level: hit.level, n: bucket.n, design: bucket.design, impl: bucket.impl, total: bucket.total }
        : null,
      verdicts: bucket
        ? {
            design: A.verdict(r.design_days_eff ?? r.design_days, bucket.design.p50),
            impl: A.verdict(r.eff_impl_days ?? r.impl_days, bucket.impl.p50),
            total: A.verdict(actual, bucket.total.p50),
          }
        : { design: null, impl: null, total: null },
    };
  };
  const completed = mine
    .filter((r) => A.isDone(r) && inViewPeriod(r, sinceMs))
    .sort((a, b) => (b.eff_done_at ?? b.done_at) - (a.eff_done_at ?? a.done_at))
    .map(decorate);
  const open = mine
    .filter((r) => !A.isDone(r))
    .map((r) => ({ ...openTaskRow(r, base, factors, config, estimates), ...lineage(r), intervals: r.intervals }));

  // Tasks this dev contributed code to without being the assignee.
  const contributions = records
    .filter((r) => canonical(r.assignee_id) !== assigneeId && A.isDone(r) && inViewPeriod(r, sinceMs))
    .map((r) => ({ r, credit: A.contributorCredits(r, scope.matcher).map((c) => ({ ...c, person_id: canonical(c.person_id) })).find((c) => c.person_id === assigneeId) }))
    .filter((x) => x.credit)
    .sort((a, b) => (b.r.eff_done_at ?? 0) - (a.r.eff_done_at ?? 0))
    .slice(0, 50)
    .map(({ r, credit }) => ({
      key: r.key,
      summary: r.summary,
      assignee_name: r.assignee_name,
      share: Math.round(credit.share * 100) / 100,
      commits: credit.commits,
      actual_days: r.manual_days ?? r.eff_cycle_days ?? r.cycle_days,
    }));

  return { account, projects: scope.projects, since: sinceMs || null, stats: devStats, team, goals, completed, open, contributions };
}

// ---- git-only features view ---------------------------------------------------

function featuresView(account) {
  const file = store.readJson(store.featuresPath(DATA_DIR), null);
  if (!file || !Array.isArray(file.features)) return { scanned_at: null, features: [], unscoped: (file && file.unscoped) || [] };
  const config = loadConfig();
  const scope = loadScope(account, '*');
  const estimates = loadEstimates(account, '__features__');
  const matcher = scope ? scope.matcher : A.makeAuthorMatcher({});
  const factors = scope ? A.assigneeFactor(scope.records, A.baselines(scope.records)) : new Map();
  const people = scope ? scope.people : {};
  const rows = file.features
    .filter((f) => !(f.jira_keys || []).length)
    .map((f) => {
      const est = estimates[f.id];
      const credits = (f.authors || [])
        .map((a) => ({ id: matcher(a.name, a.email), a }))
        .filter((x) => x.id);
      const top = credits[0] ? credits[0].id : null;
      const factor = top && factors.has(top) ? factors.get(top).factor : 1.0;
      const actual = Math.round(A.businessDays(f.first_commit_at, f.merged_at, config.workweek) * 100) / 100;
      return {
        ...f,
        actual_days: actual,
        est_days_ai: est && est.days > 0 ? est.days : null,
        est_days_dev: est && est.days > 0 ? Math.round(est.days * factor * 100) / 100 : null,
        routine: Boolean(est && est.routine),
        people: credits.map((c) => (people[c.id] ? people[c.id].name : c.a.name)),
        deploy_wait_days:
          f.deployed_at && f.deployed_at > f.merged_at
            ? Math.round(A.businessDays(f.merged_at, f.deployed_at, config.workweek) * 100) / 100
            : null,
      };
    })
    .sort((a, b) => b.merged_at - a.merged_at);
  return { scanned_at: file.scanned_at || null, features: rows, unscoped: file.unscoped || [] };
}

// ---- per-dev summary reports (agent-written HTML, per quarter/year) -----------

const fs = require('fs');

/** UTC bounds of a report period. kind: 'year' | 'quarter'. */
function periodBounds(kind, year, quarter) {
  if (kind === 'quarter') {
    const q = Math.min(4, Math.max(1, quarter || 1));
    const start = Date.UTC(year, (q - 1) * 3, 1);
    const end = Date.UTC(year, q * 3, 1);
    return { start, end, label: `${year} Q${q}` };
  }
  return { start: Date.UTC(year, 0, 1), end: Date.UTC(year + 1, 0, 1), label: String(year) };
}

function loadReportsIndex(account) {
  const idx = store.readJson(store.reportsIndexPath(DATA_DIR, account), null);
  return idx && Array.isArray(idx.reports) ? idx : { reports: [] };
}

/** One dev's numbers for a window: stats row + task list (for the prompt). */
function periodSummary(scope, assigneeId, start, end) {
  const { base, stats } = nowStats(scope, start, end);
  const s = stats.find((x) => x.assignee_id === assigneeId) || null;
  const mine = scope.records.filter(
    (r) =>
      scope.canonical(r.assignee_id) === assigneeId &&
      A.isDone(r) &&
      (r.eff_done_at ?? r.done_at ?? 0) >= start &&
      (r.eff_done_at ?? r.done_at ?? 0) < end,
  );
  const tasks = mine
    .filter((r) => !r.rollup)
    .sort((a, b) => (b.manual_days ?? b.eff_cycle_days ?? b.cycle_days ?? 0) - (a.manual_days ?? a.eff_cycle_days ?? a.cycle_days ?? 0))
    .slice(0, 40)
    .map((r) => {
      const est = scope.estimates[r.key];
      return {
        key: r.key,
        type: r.type,
        summary: r.summary.slice(0, 110),
        est_ai: est && est.days > 0 ? est.days : null,
        actual: r.manual_days ?? r.eff_cycle_days ?? r.cycle_days,
        fixes: r.fix_count || 0,
        deployed: Boolean(r.deployed_at),
      };
    });
  void base;
  return { stats: s, tasks, task_count: mine.length };
}

/** Strip a saved HTML report to text for prompt context. */
function htmlToText(html, cap = 1600) {
  return String(html)
    .replace(/<style[\s\S]*?<\/style>/gi, ' ')
    .replace(/<script[\s\S]*?<\/script>/gi, ' ')
    .replace(/<[^>]+>/g, ' ')
    .replace(/\s+/g, ' ')
    .trim()
    .slice(0, cap);
}

const reportJobs = new Map(); // `${account}__${assignee}__${label}` -> job

async function runReport(account, assigneeId, kind, year, quarter) {
  const jobKey = `${account}__${assigneeId}__${kind}${year}${quarter || ''}`;
  const job = reportJobs.get(jobKey);
  try {
    const scope = loadScope(account, '*');
    if (!scope) throw new Error('no scan yet');
    const { start, end, label } = periodBounds(kind, year, quarter);
    const cur = periodSummary(scope, assigneeId, start, end);
    if (!cur.stats) throw new Error('no data for this developer in that period');
    const prevBounds = kind === 'quarter'
      ? periodBounds('quarter', year - 1, quarter)
      : periodBounds('year', year - 1);
    const prev = periodSummary(scope, assigneeId, prevBounds.start, prevBounds.end);
    const name = cur.stats.assignee_name || assigneeId;

    // Team reference for the same window + this dev's goals.
    const { stats: teamStats } = nowStats(scope, start, end);
    const team = A.teamMedians(teamStats);
    const goalsFile = loadDevGoals(account);
    const slot = goalsFile.assignees[assigneeId] || { goals: [], snapshots: [] };
    const goals = devGoalRows(slot, cur.stats, team).map((g) => ({ metric: g.metric, target: g.target, current: g.current, met: g.met }));

    // Attach previously saved reports for this dev (older periods) as context.
    const idx = loadReportsIndex(account);
    const priors = idx.reports
      .filter((r) => r.assignee_id === assigneeId && !(r.kind === kind && r.year === year && (r.quarter || null) === (quarter || null)))
      .sort((a, b) => b.year - a.year || (b.quarter || 0) - (a.quarter || 0))
      .slice(0, 2)
      .map((r) => {
        try {
          return { label: r.label, text: htmlToText(fs.readFileSync(store.reportFilePath(DATA_DIR, account, r.file_name), 'utf8')) };
        } catch {
          return null;
        }
      })
      .filter(Boolean);

    const fmt = (o) => JSON.stringify(o, (_, v) => (typeof v === 'number' ? Math.round(v * 100) / 100 : v));
    const prompt = `You are writing a performance summary report for an engineering team lead about developer "${name}" for ${label}. Write a COMPLETE, SELF-CONTAINED HTML document (inline CSS, dark theme, no external assets, no javascript) — respond with ONLY the HTML, starting with <!doctype html>.

Data (business days; "weighted_done" = dev-agnostic AI-estimated days of work delivered, credited by commit share — the fair volume measure; "pace_factor" >1 = slower than team on the same estimated volume; "efficiency" >1 = delivers faster than estimates):

THIS PERIOD (${label}): ${fmt(cur.stats)}
Largest tasks this period: ${fmt(cur.tasks.slice(0, 25))} (of ${cur.task_count} total)
SAME PERIOD LAST YEAR (${prevBounds.label}): ${prev.stats ? fmt(prev.stats) : 'no data'}
TEAM medians this period: ${fmt(team)}
Current goals: ${fmt(goals)}
${priors.length ? priors.map((p) => `PRIOR SAVED REPORT (${p.label}): ${p.text}`).join('\n') : ''}

The report must include: an executive summary; delivery volume & quality vs ${prevBounds.label} (call out concrete changes with numbers); strengths; areas to improve (be specific, use the task data — e.g. estimate misses, fix rates, WIP habits); notable tasks; and proposed goals for the NEXT period (concrete, measurable targets derived from the data). Be honest and specific — this is an internal management document, not a celebration page.`;

    const r = await hostPost('/agents/run', { prompt });
    let html = (r && r.text) || '';
    const lo = html.search(/<!doctype|<html/i);
    if (lo > 0) html = html.slice(lo);
    const endTag = html.toLowerCase().lastIndexOf('</html>');
    if (endTag > 0) html = html.slice(0, endTag + 7);
    if (!/<html|<!doctype/i.test(html)) {
      html = `<!doctype html><html><body><pre style="white-space:pre-wrap;font-family:system-ui">${html.replace(/</g, '&lt;')}</pre></body></html>`;
    }

    const id = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`;
    const fileName = `${assigneeId}__${kind}-${year}${quarter ? `-Q${quarter}` : ''}__${id}`;
    fs.mkdirSync(store.reportsDir(DATA_DIR, account), { recursive: true });
    fs.writeFileSync(store.reportFilePath(DATA_DIR, account, fileName), html);
    const entry = {
      id,
      assignee_id: assigneeId,
      assignee_name: name,
      kind,
      year,
      quarter: quarter || null,
      label,
      created_at: Date.now(),
      file_name: fileName,
    };
    idx.reports.push(entry);
    store.writeJsonAtomic(store.reportsIndexPath(DATA_DIR, account), idx);
    job.state = 'done';
    job.report = entry;
  } catch (e) {
    console.error('report failed:', e);
    job.state = 'error';
    job.error = String(e.message || 'report failed');
  }
  job.finished_at = Date.now();
}

// ---- AI coach ---------------------------------------------------------------

async function coach(account, projectsParam, assigneeId) {
  const scope = loadScope(account, projectsParam);
  if (!scope) throw new Error('no scan yet');
  const { stats } = nowStats(scope);
  const team = A.teamMedians(stats);
  const list = assigneeId ? stats.filter((s) => s.assignee_id === assigneeId) : stats;
  const lines = list.map(
    (s) =>
      `- ${s.assignee_name}${s.role ? ` (${s.role})` : ''}: completed=${s.completed} wip=${s.wip} weighted_done=${s.weighted_done}est-days efficiency=${s.efficiency ?? 'n/a'} routine_share=${s.routine_done}/${s.routine_done + s.feature_done} median_impl=${s.median_impl}d median_cycle=${s.median_cycle}d vs_team_factor=${s.factor ?? 'n/a'} estimate_error=${s.mape ?? 'n/a'} avg_wip=${s.avg_wip ?? 'n/a'} trend=${s.trend ?? 'n/a'} flags=${JSON.stringify(s.flags)}`,
  );
  const prompt = `You are an engineering-delivery coach for a team lead. Data below comes from git-primary delivery analysis (first commit → merge to develop/release, fixes, deploy tags; business days) blended with Jira changelogs, plus AI dev-agnostic scope estimates (weighted_done = estimated days of work delivered — the fair throughput measure; task counts are misleading because routine work like version bumps inflates them). Projects: ${scope.projects.join(', ')}. Team medians: ${JSON.stringify(team)}.\n\nPer-developer stats:\n${lines.join('\n')}\n\nGive concise, concrete coaching: for each developer, 2-3 specific observations (scope-weighted output vs raw counts, efficiency, phase imbalance, estimation accuracy, WIP habits, trend) and one actionable goal. Avoid generic advice.`;
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
const SCOPE_METRICS = new Set(A.SCOPE_METRICS);

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
      const config = loadConfig();
      const live = await makeClient(creds, { paceMs: config.pace_ms }).searchProjects(q.get('query') || '');
      const scanned = new Set(store.listProjects(DATA_DIR, q.get('account') || '').filter((p) => p !== '__features__'));
      return send(res, 200, live.map((p) => ({ ...p, scanned: scanned.has(p.key) })));
    }

    if (u.pathname === '/users' && req.method === 'GET') {
      const creds = await hostGet(`/jira/credentials?account=${encodeURIComponent(q.get('account') || '')}`);
      const config = loadConfig();
      return send(res, 200, await makeClient(creds, { paceMs: config.pace_ms }).assignableUsers(q.get('project') || ''));
    }

    if (u.pathname === '/repos' && req.method === 'GET') {
      return send(res, 200, (await hostRepos()).map((r) => ({ name: r.name, path: r.path })));
    }

    if (u.pathname === '/statuses' && req.method === 'GET') {
      const project = q.get('project') || '';
      const creds = await hostGet(`/jira/credentials?account=${encodeURIComponent(q.get('account') || '')}`);
      const config = loadConfig();
      const statuses = await makeClient(creds, { paceMs: config.pace_ms }).projectStatuses(project);
      const map = config.status_map[project] || {};
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

    if (u.pathname === '/people' && req.method === 'GET') {
      const reg = loadPeople();
      const scope = loadScope(q.get('account') || '', '*');
      const unmatched = scope ? A.unmatchedAuthors(scope.all_records, scope.matcher).slice(0, 30) : [];
      return send(res, 200, { people: reg.people, unmatched_authors: unmatched, roles: loadConfig().roles });
    }
    if (u.pathname === '/people' && req.method === 'PUT') {
      const body = await readBody(req);
      if (!body.people || typeof body.people !== 'object') return send(res, 400, { error: 'people object required' });
      const reg = loadPeople();
      for (const [id, p] of Object.entries(body.people)) {
        if (!p || typeof p !== 'object') continue;
        const prev = reg.people[id] || { name: id, role: '', included: true, aliases: [] };
        reg.people[id] = {
          name: typeof p.name === 'string' && p.name.trim() ? p.name.trim() : prev.name,
          role: typeof p.role === 'string' ? p.role.trim() : prev.role,
          included: p.included !== undefined ? Boolean(p.included) : prev.included,
          aliases: Array.isArray(p.aliases) ? p.aliases.map(String).map((a) => a.trim()).filter(Boolean).slice(0, 20) : prev.aliases,
          // Merge duplicate Jira accounts of the same human: this account's
          // history folds into `merged_into` everywhere (no self-merge).
          merged_into:
            p.merged_into !== undefined
              ? p.merged_into && p.merged_into !== id
                ? String(p.merged_into)
                : null
              : prev.merged_into || null,
        };
      }
      savePeople(reg);
      return send(res, 200, { people: reg.people });
    }
    if (u.pathname === '/people/seed' && req.method === 'POST') {
      const body = await readBody(req);
      const creds = await hostGet(`/jira/credentials?account=${encodeURIComponent(body.account || '')}`);
      const config = loadConfig();
      const users = await makeClient(creds, { paceMs: config.pace_ms }).assignableUsers(body.project || '');
      const reg = loadPeople();
      let added = 0;
      for (const uOne of users) {
        if (reg.people[uOne.id]) continue;
        reg.people[uOne.id] = { name: uOne.name, role: '', included: true, aliases: [] };
        added++;
      }
      savePeople(reg);
      return send(res, 200, { people: reg.people, added });
    }

    if (u.pathname === '/scan' && req.method === 'POST') {
      const body = await readBody(req);
      const account = body.account;
      const projects = Array.isArray(body.projects) && body.projects.length
        ? body.projects.map(String)
        : body.project
          ? [String(body.project)]
          : [];
      if (!account || !projects.length) return send(res, 400, { error: 'account and projects[] are required' });
      const assignees = Array.isArray(body.assignees) ? body.assignees.map(String).filter(Boolean) : null;
      const existing = jobs.get(account);
      if (existing && existing.state === 'running') return send(res, 409, { error: 'scan already running' });
      const job = {
        state: 'running',
        step: 'starting',
        project: projects[0],
        project_i: 0,
        project_n: projects.length,
        fetched: 0,
        total: null,
        retries: 0,
        pace_ms: null,
        errors: 0,
        estimate_remaining: 0,
        started_at: Date.now(),
        finished_at: null,
        error: null,
        full: Boolean(body.full),
        scoped_people: assignees ? assignees.length : 0,
        last_scan: existing ? existing.last_scan : null,
      };
      jobs.set(account, job);
      rememberScanParams(account, projects, assignees); // the auto-scan cron repeats these
      runScan(account, projects, Boolean(body.full), assignees); // fire and forget; job records progress
      return send(res, 200, { started: true, projects });
    }

    if (u.pathname === '/scan/status' && req.method === 'GET') {
      const account = q.get('account') || '';
      const job = jobs.get(account);
      if (job) return send(res, 200, job);
      const scanned = store.listProjects(DATA_DIR, account).filter((p) => p !== '__features__');
      let last = null;
      for (const p of scanned) {
        const c = store.readJson(store.corpusPath(DATA_DIR, account, p), null);
        if (c && c.scanned_at && (!last || c.scanned_at > last)) last = c.scanned_at;
      }
      return send(res, 200, { state: 'idle', last_scan: last });
    }

    const sinceParam = () => {
      const s = q.get('since');
      if (!s) return 0;
      const n = /^\d+$/.test(s) ? Number(s) : Date.parse(s);
      return Number.isFinite(n) && n > 0 ? n : 0;
    };

    if (u.pathname === '/overview' && req.method === 'GET') {
      const o = overview(q.get('account') || '', q.get('projects') || q.get('project') || '', sinceParam());
      return o ? send(res, 200, o) : send(res, 404, { error: 'no scan for this scope yet' });
    }

    if (u.pathname === '/assignee' && req.method === 'GET') {
      const v = assigneeView(q.get('account') || '', q.get('projects') || q.get('project') || '', q.get('assignee') || '', sinceParam());
      return v ? send(res, 200, v) : send(res, 404, { error: 'unknown assignee or no scan yet' });
    }

    if (u.pathname === '/report' && req.method === 'POST') {
      const body = await readBody(req);
      const { account, assignee, kind, year } = body;
      const quarter = body.quarter ? Number(body.quarter) : null;
      if (!account || !assignee || !['year', 'quarter'].includes(kind) || !Number.isInteger(Number(year))) {
        return send(res, 400, { error: 'account, assignee, kind (year|quarter), year required' });
      }
      if (kind === 'quarter' && !(quarter >= 1 && quarter <= 4)) return send(res, 400, { error: 'quarter must be 1..4' });
      const jobKey = `${account}__${assignee}__${kind}${year}${quarter || ''}`;
      const existing = reportJobs.get(jobKey);
      if (existing && existing.state === 'running') return send(res, 409, { error: 'report already generating' });
      const job = { state: 'running', started_at: Date.now(), finished_at: null, error: null, report: null };
      reportJobs.set(jobKey, job);
      runReport(account, assignee, kind, Number(year), quarter); // fire and forget
      return send(res, 200, { started: true, job: jobKey });
    }

    if (u.pathname === '/report/status' && req.method === 'GET') {
      const job = reportJobs.get(q.get('job') || '');
      return send(res, 200, job || { state: 'idle' });
    }

    if (u.pathname === '/reports' && req.method === 'GET') {
      const idx = loadReportsIndex(q.get('account') || '');
      const assignee = q.get('assignee');
      const list = idx.reports
        .filter((r) => !assignee || r.assignee_id === assignee)
        .sort((a, b) => b.created_at - a.created_at);
      return send(res, 200, { reports: list });
    }

    if (u.pathname === '/report/html' && req.method === 'GET') {
      const idx = loadReportsIndex(q.get('account') || '');
      const entry = idx.reports.find((r) => r.id === q.get('id'));
      if (!entry) return send(res, 404, { error: 'unknown report' });
      try {
        const html = fs.readFileSync(store.reportFilePath(DATA_DIR, q.get('account') || '', entry.file_name), 'utf8');
        return send(res, 200, { ...entry, html });
      } catch {
        return send(res, 404, { error: 'report file missing' });
      }
    }

    if (u.pathname === '/features' && req.method === 'GET') {
      return send(res, 200, featuresView(q.get('account') || ''));
    }

    if (u.pathname === '/override' && req.method === 'PUT') {
      const body = await readBody(req);
      const { account, project, key } = body;
      if (!account || !project || !key) return send(res, 400, { error: 'account, project, key required' });
      const file = store.overridesPath(DATA_DIR, account, project);
      const cur = store.readJson(file, null) || { issues: {} };
      if (!cur.issues) cur.issues = {};
      const o = cur.issues[key] || {};
      if (body.outlier !== undefined) o.outlier = Boolean(body.outlier);
      if (body.excluded !== undefined) o.excluded = Boolean(body.excluded);
      if (body.manual_days !== undefined) {
        const n = body.manual_days === null ? null : Number(body.manual_days);
        if (n !== null && (!Number.isFinite(n) || n <= 0 || n > 365)) return send(res, 400, { error: 'manual_days must be in 0..365 or null' });
        o.manual_days = n;
      }
      o.updated_at = Date.now();
      cur.issues[key] = o;
      store.writeJsonAtomic(file, cur);
      return send(res, 200, { key, ...o });
    }

    if (u.pathname === '/goals' && req.method === 'PUT') {
      const body = await readBody(req);
      const { account, assignee, goals } = body;
      if (!account || !assignee || !Array.isArray(goals)) {
        return send(res, 400, { error: 'account, assignee, goals[] required' });
      }
      for (const g of goals) {
        if (!GOAL_METRICS.has(g.metric)) return send(res, 400, { error: `unknown metric ${g.metric}` });
        const t = Number(g.target);
        if (!Number.isFinite(t) || t <= 0) return send(res, 400, { error: 'target must be a positive number' });
      }
      const file = loadDevGoals(account);
      const slot = (file.assignees[assignee] ||= { goals: [], snapshots: [] });
      for (const g of goals) {
        slot.goals = slot.goals.filter((x) => x.metric !== g.metric);
        slot.goals.push({ metric: g.metric, target: Number(g.target), set_at: Date.now() });
      }
      saveDevGoals(account, file);
      return send(res, 200, { saved: slot.goals });
    }

    if (u.pathname === '/goals/scope' && req.method === 'GET') {
      return send(res, 200, loadScopeGoals(q.get('account') || ''));
    }
    if (u.pathname === '/goals/scope' && req.method === 'PUT') {
      const body = await readBody(req);
      if (!body.account) return send(res, 400, { error: 'account required' });
      const check = (arr) => {
        if (!Array.isArray(arr)) throw new Error('goals must be arrays');
        for (const g of arr) {
          if (!SCOPE_METRICS.has(g.metric)) throw new Error(`unknown scope metric ${g.metric}`);
          const t = Number(g.target);
          if (!Number.isFinite(t) || t < 0) throw new Error('target must be a non-negative number');
          g.target = t;
        }
        return arr.map((g) => ({ metric: g.metric, target: g.target }));
      };
      try {
        const team = check(body.team || []);
        const roles = {};
        for (const [role, arr] of Object.entries(body.roles || {})) roles[role] = check(arr);
        store.writeJsonAtomic(store.scopeGoalsPath(DATA_DIR, body.account), { team, roles });
        return send(res, 200, { team, roles });
      } catch (e) {
        return send(res, 400, { error: e.message });
      }
    }

    if (u.pathname === '/analyze' && req.method === 'POST') {
      const body = await readBody(req);
      return send(res, 200, await coach(body.account, body.projects || body.project || '', body.assignee || null));
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
