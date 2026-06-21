// team-performance — Otto runtime plugin (Node sidecar, zero dependencies).
//
// Otto spawns this with: OTTO_PLUGIN_PORT (bind here), OTTO_PLUGIN_TOKEN +
// OTTO_HOST_API (call back for repos/jira/agents), OTTO_PLUGIN_DATA_DIR (config).
// Otto reverse-proxies /api/v1/plugins/team-performance/* to these routes.
//
// "Done" = a story's LAST merge into `develop` (git merge-base --is-ancestor),
// correlated by the JIRA key in commit subjects. Each completed story gets an
// AI-adjusted target (estimate × (1 − target), default 10%) and a met/missed
// verdict; overlapping in-progress windows are flagged as concurrent work.

const http = require('http');
const https = require('https');
const { execFileSync } = require('child_process');
const fs = require('fs');
const path = require('path');
const { URL } = require('url');

const PORT = parseInt(process.env.OTTO_PLUGIN_PORT || '0', 10);
const HOST_API = process.env.OTTO_HOST_API || '';
const TOKEN = process.env.OTTO_PLUGIN_TOKEN || '';
const DATA_DIR = process.env.OTTO_PLUGIN_DATA_DIR || '.';
const CONFIG = path.join(DATA_DIR, 'config.json');
const MAX_STORIES = 50;

// ---- helpers --------------------------------------------------------------

function httpJson(method, urlStr, headers, body) {
  return new Promise((resolve, reject) => {
    const u = new URL(urlStr);
    const mod = u.protocol === 'https:' ? https : http;
    const data = body ? JSON.stringify(body) : null;
    const opts = {
      method,
      hostname: u.hostname,
      port: u.port || (u.protocol === 'https:' ? 443 : 80),
      path: u.pathname + u.search,
      headers: { Accept: 'application/json', ...(headers || {}) },
    };
    if (data) opts.headers['Content-Type'] = 'application/json';
    const req = mod.request(opts, (res) => {
      let buf = '';
      res.on('data', (c) => (buf += c));
      res.on('end', () => {
        if (res.statusCode >= 200 && res.statusCode < 300) {
          try {
            resolve(buf ? JSON.parse(buf) : null);
          } catch (e) {
            reject(new Error('bad JSON from ' + urlStr));
          }
        } else {
          reject(new Error(`${res.statusCode} from ${urlStr}: ${buf.slice(0, 200)}`));
        }
      });
    });
    req.on('error', reject);
    if (data) req.write(data);
    req.end();
  });
}

const hostGet = (p) => httpJson('GET', HOST_API + p, { Authorization: `Bearer ${TOKEN}` });
const hostPost = (p, b) => httpJson('POST', HOST_API + p, { Authorization: `Bearer ${TOKEN}` }, b);

function jiraGet(creds, p) {
  const auth = Buffer.from(`${creds.email}:${creds.token}`).toString('base64');
  return httpJson('GET', creds.base_url.replace(/\/$/, '') + p, { Authorization: `Basic ${auth}` });
}

function loadConfig() {
  try {
    return JSON.parse(fs.readFileSync(CONFIG, 'utf8'));
  } catch {
    return { target: 0.1 };
  }
}
function saveConfig(c) {
  fs.mkdirSync(DATA_DIR, { recursive: true });
  fs.writeFileSync(CONFIG, JSON.stringify(c));
}

function git(repoPath, args) {
  try {
    return execFileSync('git', ['-C', repoPath, ...args], {
      encoding: 'utf8',
      maxBuffer: 32 * 1024 * 1024,
    });
  } catch {
    return '';
  }
}

const KEY_RE = /\b[A-Z][A-Z0-9]+-\d+\b/g;
function parseEstimate(s) {
  if (!s) return null;
  const m = String(s).match(/[\d.]+/);
  return m ? parseFloat(m[0]) : null;
}
function jiraTs(s) {
  const t = Date.parse(s);
  return Number.isNaN(t) ? null : t;
}

// Last commit mentioning `key` that is merged into develop, across all repos.
function completedAt(repos, key) {
  let best = null;
  for (const r of repos) {
    const log = git(r.path, ['log', '--all', '-n', '2000', '--pretty=%H%x1f%cI%x1f%s']);
    const lines = log.split('\n').filter(Boolean);
    const matches = [];
    for (const line of lines) {
      const [sha, date, subj] = line.split('\x1f');
      if (subj && subj.match(KEY_RE)?.includes(key)) matches.push({ sha, date });
    }
    matches.sort((a, b) => Date.parse(b.date) - Date.parse(a.date)); // newest first
    for (const m of matches) {
      try {
        execFileSync('git', ['-C', r.path, 'merge-base', '--is-ancestor', m.sha, 'develop']);
        const t = Date.parse(m.date);
        if (best === null || t > best) best = t;
        break; // newest ancestor in this repo
      } catch {
        /* not an ancestor — keep looking older */
      }
    }
  }
  return best;
}

function inProgressAt(changelog) {
  let earliest = null;
  for (const h of changelog?.histories || []) {
    for (const it of h.items || []) {
      if (it.field === 'status' && String(it.toString || '').toLowerCase().includes('progress')) {
        const t = jiraTs(h.created);
        if (t !== null && (earliest === null || t < earliest)) earliest = t;
      }
    }
  }
  return earliest;
}

const DAY = 86400000;

async function scorecard(account, project, assignee) {
  const creds = await hostGet('/jira/credentials?account=' + encodeURIComponent(account));
  const repos = await hostGet('/repos');
  const cfg = loadConfig();
  const target = cfg.target ?? 0.1;

  const jql = `project = "${project}" AND issuetype = Story AND assignee = "${assignee}" ORDER BY updated DESC`;
  const search = await jiraGet(
    creds,
    `/rest/api/3/search/jql?maxResults=${MAX_STORIES}&fields=summary&jql=${encodeURIComponent(jql)}`,
  ).catch(() => ({ issues: [] }));
  const keys = (search.issues || []).map((i) => i.key).slice(0, MAX_STORIES);

  const stories = [];
  for (const key of keys) {
    let full;
    try {
      full = await jiraGet(
        creds,
        `/rest/api/3/issue/${key}?expand=changelog&fields=summary,status,assignee,timeoriginalestimate,customfield_10016`,
      );
    } catch {
      continue;
    }
    const f = full.fields || {};
    const est =
      parseEstimate(f.customfield_10016) ??
      (f.timeoriginalestimate ? f.timeoriginalestimate / (8 * 3600) : null); // seconds → days (8h/day)
    const ip = inProgressAt(full.changelog);
    const done = completedAt(repos, key);
    const actual = ip && done ? Math.max(0, (done - ip) / DAY) : null;
    const suggested = est != null ? est * (1 - target) : null;
    const met = actual != null && suggested != null ? actual <= suggested : null;
    stories.push({
      key,
      summary: f.summary || '',
      status: f.status?.name || '',
      url: `${creds.base_url.replace(/\/$/, '')}/browse/${key}`,
      estimate_days: est,
      in_progress_at: ip,
      completed_at: done,
      actual_days: actual,
      suggested_days: suggested,
      met,
    });
  }

  // Concurrency: overlapping [in_progress, completed||now] windows.
  const now = Date.now();
  const concurrent = [];
  for (let i = 0; i < stories.length; i++) {
    for (let j = i + 1; j < stories.length; j++) {
      const a = stories[i],
        b = stories[j];
      if (!a.in_progress_at || !b.in_progress_at) continue;
      const a1 = a.completed_at || now,
        b1 = b.completed_at || now;
      if (a.in_progress_at < b1 && b.in_progress_at < a1) concurrent.push([a.key, b.key]);
    }
  }

  return {
    account,
    project,
    assignee,
    target,
    stories,
    completed: stories.filter((s) => s.completed_at).length,
    met_count: stories.filter((s) => s.met === true).length,
    missed_count: stories.filter((s) => s.met === false).length,
    concurrent,
  };
}

// ---- routing --------------------------------------------------------------

function send(res, code, obj) {
  const body = JSON.stringify(obj);
  res.writeHead(code, { 'Content-Type': 'application/json' });
  res.end(body);
}

const server = http.createServer(async (req, res) => {
  const u = new URL(req.url, 'http://localhost');
  const q = u.searchParams;
  try {
    if (u.pathname === '/health') return send(res, 200, { ok: true });

    if (u.pathname === '/accounts' && req.method === 'GET') {
      return send(res, 200, await hostGet('/jira/accounts'));
    }
    if (u.pathname === '/config' && req.method === 'GET') {
      return send(res, 200, { target: loadConfig().target ?? 0.1 });
    }
    if (u.pathname === '/config' && req.method === 'PUT') {
      const body = await readBody(req);
      const target = Math.min(1, Math.max(0, parseFloat(body.target)));
      saveConfig({ target });
      return send(res, 200, { target });
    }
    if (u.pathname === '/scorecard' && req.method === 'GET') {
      const card = await scorecard(q.get('account'), q.get('project'), q.get('assignee'));
      return send(res, 200, card);
    }
    if (u.pathname === '/analyze' && req.method === 'POST') {
      const body = await readBody(req);
      const card = await scorecard(body.account, body.project, body.assignee);
      const lines = card.stories
        .map(
          (s) =>
            `  - ${s.key} [${s.status}] est=${s.estimate_days ?? '?'}d actual=${s.actual_days != null ? s.actual_days.toFixed(1) + 'd' : 'open'} met=${s.met === null ? 'n/a' : s.met}`,
        )
        .join('\n');
      const prompt = `You are an engineering-delivery coach. The team adopted AI tooling with a ${(card.target * 100).toFixed(0)}% estimation-improvement target. For "${card.assignee}" on ${card.project}: ${card.completed} completed, ${card.met_count} met, ${card.missed_count} missed, ${card.concurrent.length} concurrent overlaps. Assess the trend and give concrete coaching. Stories:\n${lines}`;
      const r = await hostPost('/agents/run', { prompt });
      return send(res, 200, { summary: r.text });
    }
    send(res, 404, { error: 'not found' });
  } catch (e) {
    send(res, 500, { error: String(e && e.message ? e.message : e) });
  }
});

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

server.listen(PORT, '127.0.0.1', () => {
  console.log(`team-performance sidecar on :${PORT}`);
});
