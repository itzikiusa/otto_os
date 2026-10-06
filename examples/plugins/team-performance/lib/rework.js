// Git rework analysis — a worker process: JSON on stdin → JSON on stdout.
//
// Input: { repos: [path], since?: 'YYYY-MM-DD', from?: 'YYYY-MM-DD',
//          recent_days?: number, cache_path?: string,
//          config?: { rework: { since, recent_days } } }
// argv overrides: --since=YYYY-MM-DD --recent-days=N --cache=PATH
//
// For every ticket-keyed commit (ABC-123 style key in the subject) on the
// repo's MAINLINE (origin/develop, else origin/main, plus release/* and
// hotfix/* refs — never --all, which drags in abandoned feature branches),
// blame the lines it deleted/changed on its parent and classify them:
//   self        — the line was written by the same ticket (iteration, not rework)
//   reworkOther — written by ANOTHER ticket within recent_days → a pair B>A
//   recentNoKey — recent but un-keyed code
//   older       — older than recent_days (maintenance, not rework)
// Cherry-picks across mainline/release refs are deduplicated by
// `git patch-id --stable`. Each commit's blame result is cached per SHA in
// rework-cache.json (independent of recent_days, so a window change needs no
// re-blame); re-runs only blame new SHAs. Read-only on every repo.
//
// Renames: the diff uses rename detection (-M40%) so a `git mv` + edit blames
// the OLD path on the parent, and blame runs with -C (lines moved/copied from
// other files in the same commit keep their origin) and follows the file's
// history across earlier renames — the charge survives a file move.
//
// Output also carries, for reports:
//   perAuthor  email → { rework_out_lines (others' recent code this person
//              rewrote), rework_in_lines (this person's recent code others
//              rewrote), self_lines }
//   byKey      key → { reworked: [{key, lines}] (tickets this one rewrote),
//              reworked_by: [{key, lines}] (tickets that rewrote this one) }
'use strict';
const fs = require('fs');
const path = require('path');
const { execFile, spawn } = require('child_process');

const CACHE_VERSION = 3; // v3: author on commits/origins, rename-aware blame
const DEFAULT_RECENT_DAYS = 120;
const DEFAULT_SINCE_DAYS = 365;
const BLAME_CONCURRENCY = 4;
const KEY_RE = /\b([A-Z][A-Z0-9]+)-(\d+)\b/g;
const SKIP = /(^|\/)(vendor|node_modules|dist|build|\.idea|generated|mocks?)\/|_mock\.go$|\.pb\.go$|go\.sum$|package-lock\.json$|yarn\.lock$|\.min\.(js|css)$|\.(png|jpg|jpeg|gif|svg|ico|pdf|zip|jar)$/i;
const MAX_BLAME_LINES = 2500;
const PATCH_ID_BATCH = 100;

const git = (cwd, args) => new Promise((res) => execFile('git', args, { cwd, maxBuffer: 1 << 28, timeout: 120000 }, (err, out) => res(err ? null : out)));
const keysOf = (s) => [...new Set([...String(s).matchAll(KEY_RE)].map((m) => `${m[1]}-${m[2]}`))].filter((k) => !/-0+$/.test(k));

/** Tiny counting semaphore: bounds concurrent `git blame` processes. */
function limiter(n) {
  let active = 0;
  const queue = [];
  const next = () => {
    if (active >= n || !queue.length) return;
    active++;
    const { fn, res } = queue.shift();
    fn().then(res, () => res(null)).finally(() => { active--; next(); });
  };
  return (fn) => new Promise((res) => { queue.push({ fn, res }); next(); });
}

/** Mainline + release/hotfix refs to scan (remote-tracking preferred over stale locals). */
async function scanRefs(repo) {
  const out = await git(repo, ['for-each-ref', '--format=%(refname)', 'refs/remotes/origin/', 'refs/heads/']);
  if (!out) return [];
  const all = out.split('\n').filter(Boolean);
  const has = (r) => all.includes(r);
  const main = ['refs/remotes/origin/develop', 'refs/remotes/origin/main', 'refs/heads/develop', 'refs/heads/main', 'refs/heads/master'].find(has);
  const extra = all.filter((r) => /^refs\/remotes\/origin\/(release|hotfix)\//i.test(r));
  return [...new Set([main, ...extra].filter(Boolean))];
}

/** sha → patch-id for the given commits (batched `git show | git patch-id --stable`). */
async function patchIds(repo, shas) {
  const out = {};
  for (let i = 0; i < shas.length; i += PATCH_ID_BATCH) {
    const batch = shas.slice(i, i + PATCH_ID_BATCH);
    const text = await new Promise((res) => {
      const show = spawn('git', ['show', '--no-color', '--format=commit %H', ...batch], { cwd: repo, stdio: ['ignore', 'pipe', 'ignore'] });
      const pid = spawn('git', ['patch-id', '--stable'], { cwd: repo, stdio: ['pipe', 'pipe', 'ignore'] });
      let buf = '';
      show.stdout.pipe(pid.stdin);
      show.on('error', () => res(''));
      pid.on('error', () => res(''));
      pid.stdout.on('data', (d) => (buf += d));
      pid.on('close', () => res(buf));
    });
    for (const l of text.split('\n')) {
      const [p, sha] = l.split(' ');
      if (p && sha) out[sha] = p;
    }
  }
  return out;
}

/** Blame one commit's rewritten lines → cached per-SHA result (window-independent). */
async function analyzeCommit(repo, c, blame) {
  const res = { at: c.at, ks: c.ks, author: c.author || null, added: 0, deleted: 0, bulk: 0, origins: [] };
  const diff = await git(repo, ['diff', '-U0', '--no-color', '-M40%', `${c.h}^`, c.h]);
  if (diff == null) return res; // root commit / unreadable: nothing rewritten
  let oldPath = null;
  let newPath = null;
  const ranges = new Map(); // oldPath -> [[a,b]]
  for (const l of diff.split('\n')) {
    if (l.startsWith('diff --git ')) { oldPath = null; newPath = null; continue; }
    if (l.startsWith('--- ')) { oldPath = l.slice(4) === '/dev/null' ? null : l.slice(6); continue; }
    if (l.startsWith('+++ ')) { newPath = l.slice(4) === '/dev/null' ? null : l.slice(6); continue; }
    const m = l.match(/^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/);
    if (!m) continue;
    const p = newPath || oldPath;
    if (!p || SKIP.test(p)) continue;
    const a = +m[1];
    const b = m[2] === undefined ? 1 : +m[2];
    res.added += m[4] === undefined ? 1 : +m[4];
    if (b > 0 && oldPath) {
      res.deleted += b;
      if (!ranges.has(oldPath)) ranges.set(oldPath, []);
      ranges.get(oldPath).push([a, a + b - 1]);
    }
  }
  const byOrigin = new Map(); // origin sha -> {at, keys, n}
  await Promise.all([...ranges].map(([file, rs]) => {
    const total = rs.reduce((x, [a, b]) => x + b - a + 1, 0);
    if (total > MAX_BLAME_LINES) { res.bulk += total; return null; }
    const args = ['blame', '--porcelain', '-C'];
    for (const [a, b] of rs) args.push('-L', `${a},${b}`);
    args.push(`${c.h}^`, '--', file);
    return blame(() => git(repo, args)).then((out) => {
      if (!out) return;
      const info = {};
      let cur = null;
      for (const l of out.split('\n')) {
        const hm = l.match(/^([0-9a-f]{40}) \d+ \d+/);
        if (hm) {
          cur = hm[1];
          info[cur] ||= { at: null, keys: [], n: 0, author: null };
          info[cur].n++;
          continue;
        }
        if (!cur) continue;
        if (l.startsWith('author-time ')) info[cur].at = +l.slice(12);
        else if (l.startsWith('summary ')) info[cur].keys = keysOf(l.slice(8));
        else if (l.startsWith('author-mail ')) info[cur].author = l.slice(12).replace(/^<|>$/g, '').toLowerCase() || null;
      }
      for (const [sha, o] of Object.entries(info)) {
        const e = byOrigin.get(sha) || { at: o.at, keys: o.keys, n: 0, author: o.author };
        e.at ??= o.at;
        e.author ??= o.author;
        if (!e.keys.length) e.keys = o.keys;
        e.n += o.n;
        byOrigin.set(sha, e);
      }
    });
  }));
  res.origins = [...byOrigin.values()];
  return res;
}

/** Aggregate cached commit results under the current recent-days window. */
function aggregate(results, recentS) {
  const perKey = {};
  const pairs = {};
  const perAuthor = {};
  const P = (a) => (perAuthor[a] ||= { rework_out_lines: 0, rework_in_lines: 0, self_lines: 0 });
  const K = (k) => (perKey[k] ||= { commits: 0, added: 0, deleted: 0, self: 0, reworkOther: 0, recentNoKey: 0, older: 0, bulk: 0, reworkedBy: 0, reworkedBy90: 0 });
  for (const c of results) {
    const w = 1 / c.ks.length;
    for (const k of c.ks) {
      const s = K(k);
      s.commits++;
      s.added += c.added * w;
      s.deleted += c.deleted * w;
      s.bulk += c.bulk * w;
    }
    for (const o of c.origins) {
      const recent = o.at != null && c.at - o.at <= recentS;
      for (const k of c.ks) {
        const s = K(k);
        const lw = o.n * w;
        if (o.keys.some((x) => c.ks.includes(x))) {
          s.self += lw;
          if (c.author) P(c.author).self_lines += lw;
        } else if (recent && o.keys.length) {
          s.reworkOther += lw;
          if (c.author) P(c.author).rework_out_lines += lw;
          if (o.author) P(o.author).rework_in_lines += lw;
          for (const a of o.keys) {
            const share = lw / o.keys.length;
            const pr = (pairs[`${k}>${a}`] ||= { lines: 0, at: c.at, originAt: o.at });
            pr.lines += share;
            pr.at = Math.min(pr.at, c.at);
            pr.originAt = Math.min(pr.originAt, o.at);
            const A = K(a);
            A.reworkedBy += share;
            if (c.at - o.at <= 90 * 86400) A.reworkedBy90 += share;
          }
        } else if (recent) s.recentNoKey += lw;
        else s.older += lw;
      }
    }
  }
  const r2 = (v) => Math.round(v * 10) / 10;
  for (const st of Object.values(perKey)) for (const f of Object.keys(st)) if (f !== 'commits') st[f] = r2(st[f]);
  for (const p of Object.values(pairs)) p.lines = r2(p.lines);
  for (const st of Object.values(perAuthor)) for (const f of Object.keys(st)) st[f] = r2(st[f]);
  return { perKey, pairs, perAuthor, byKey: byKeyLists(pairs) };
}

/** key → {reworked, reworked_by} pair lists (both directions), largest first. */
function byKeyLists(pairs) {
  const out = {};
  const E = (k) => (out[k] ||= { reworked: [], reworked_by: [] });
  for (const [pk, p] of Object.entries(pairs)) {
    const i = pk.indexOf('>');
    const b = pk.slice(0, i);
    const a = pk.slice(i + 1);
    E(b).reworked.push({ key: a, lines: p.lines });
    E(a).reworked_by.push({ key: b, lines: p.lines });
  }
  const sort = (xs) => xs.sort((x, y) => y.lines - x.lines || x.key.localeCompare(y.key));
  for (const e of Object.values(out)) { sort(e.reworked); sort(e.reworked_by); }
  return out;
}

function readCache(p) {
  if (!p) return { version: CACHE_VERSION, repos: {} };
  try {
    const c = JSON.parse(fs.readFileSync(p, 'utf8'));
    if (c && c.version === CACHE_VERSION && c.repos) return c;
  } catch { /* missing or corrupt → rebuild */ }
  return { version: CACHE_VERSION, repos: {} };
}

function writeCache(p, cache) {
  if (!p) return;
  try {
    const tmp = `${p}.tmp-${process.pid}`;
    fs.writeFileSync(tmp, JSON.stringify(cache));
    fs.renameSync(tmp, p);
  } catch { /* cache is an optimisation only */ }
}

function parseArgv(argv) {
  const o = {};
  for (const a of argv) {
    const m = a.match(/^--(since|recent-days|cache)=(.*)$/);
    if (m) o[m[1]] = m[2];
  }
  return o;
}

/** Resolve the effective options: argv > input > input.config.rework > defaults. */
function resolveOptions(input = {}, argv = []) {
  const a = parseArgv(argv);
  const cfg = (input.config && input.config.rework) || {};
  const recent = Number(a['recent-days'] ?? input.recent_days ?? cfg.recent_days ?? DEFAULT_RECENT_DAYS);
  const since = a.since || input.since || cfg.since || new Date(Date.now() - DEFAULT_SINCE_DAYS * 86400000).toISOString().slice(0, 10);
  return {
    repos: input.repos || [],
    since,
    from: Date.parse(input.from || since) / 1000 || 0,
    recentDays: recent > 0 ? recent : DEFAULT_RECENT_DAYS,
    cachePath: a.cache || input.cache_path || null,
  };
}

async function scanRepo(repo, opts, cache, blame, stats) {
  if (!fs.existsSync(path.join(repo, '.git'))) return [];
  const refs = await scanRefs(repo);
  if (!refs.length) return [];
  const log = await git(repo, ['log', '--no-merges', `--since=${opts.since}`, '--format=%H%x1f%at%x1f%ae%x1f%s', ...refs]);
  if (!log) return [];
  const commits = [];
  for (const line of log.split('\n')) {
    if (!line) continue;
    const [h, at, ae, subj] = line.split('\x1f');
    const ks = keysOf(subj);
    if (!ks.length || +at < opts.from) continue;
    commits.push({ h, at: +at, ks, author: String(ae || '').toLowerCase() || null });
  }
  const prev = cache.repos[repo] || {};
  const next = {};
  const fresh = commits.filter((c) => !prev[c.h]);
  const pids = await patchIds(repo, fresh.map((c) => c.h));
  for (const c of fresh) {
    next[c.h] = { pid: pids[c.h] || null, ...(await analyzeCommit(repo, c, blame)) };
    stats.processed++;
  }
  for (const c of commits) if (prev[c.h]) { next[c.h] = prev[c.h]; stats.cached++; }
  cache.repos[repo] = next; // drops SHAs no longer on the scanned refs
  // Patch-id dedup: the same change cherry-picked onto release/hotfix refs
  // counts once (earliest authored copy wins).
  const seen = new Set();
  const out = [];
  for (const r of Object.values(next).sort((x, y) => x.at - y.at)) {
    if (r.pid) {
      if (seen.has(r.pid)) { stats.duplicates++; continue; }
      seen.add(r.pid);
    }
    out.push(r);
  }
  return out;
}

async function run(input = {}, argv = []) {
  const opts = resolveOptions(input, argv);
  const cache = readCache(opts.cachePath);
  const blame = limiter(BLAME_CONCURRENCY);
  const stats = { processed: 0, cached: 0, duplicates: 0 };
  const results = [];
  for (const k of Object.keys(cache.repos)) if (!opts.repos.includes(k)) delete cache.repos[k];
  for (const repo of opts.repos) {
    try { results.push(...(await scanRepo(repo, opts, cache, blame, stats))); } catch { /* skip repo */ }
  }
  writeCache(opts.cachePath, cache);
  const { perKey, pairs, perAuthor, byKey } = aggregate(results, opts.recentDays * 86400);
  return { generated_at: new Date().toISOString(), since: opts.since, recent_days: opts.recentDays, stats, perKey, pairs, perAuthor, byKey };
}

module.exports = { run, resolveOptions, aggregate, byKeyLists, keysOf };
if (require.main === module) {
  let buf = '';
  process.stdin.on('data', (d) => (buf += d));
  process.stdin.on('end', async () => process.stdout.write(JSON.stringify(await run(JSON.parse(buf || '{}'), process.argv.slice(2)))));
}
