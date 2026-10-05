// Git rework analysis (worker: JSON {repos:[path], since:'YYYY-MM-DD'} on stdin → JSON on stdout): for every commit tied to a ticket (GS-/GRV- key in the
// subject) since 2025-10-01, blame the lines it deleted/changed on the parent
// and classify them: rework of ANOTHER recent ticket (< 120 days old), self
// churn (same ticket), recent un-keyed code, or older code. Read-only on every
// repo (git log / diff / blame). Writes rework.json.
'use strict';
const fs = require('fs');
const path = require('path');
const { execFile } = require('child_process');

let REPOS = [];
let SINCE = '2025-01-01';
let FROM = 0;
const RECENT_S = 120 * 86400;
const KEY_RE = /\b([A-Z][A-Z0-9]+)-(\d+)\b/g;
const SKIP = /(^|\/)(vendor|node_modules|dist|build|\.idea|generated|mocks?)\/|_mock\.go$|\.pb\.go$|go\.sum$|package-lock\.json$|yarn\.lock$|\.min\.(js|css)$|\.(png|jpg|jpeg|gif|svg|ico|pdf|zip|jar)$/i;
const MAX_BLAME_LINES = 2500;

const git = (cwd, args) => new Promise((res) => execFile('git', args, { cwd, maxBuffer: 1 << 28, timeout: 120000 }, (err, out) => res(err ? null : out)));
const keysOf = (s) => [...new Set([...String(s).matchAll(KEY_RE)].map((m) => `${m[1]}-${m[2]}`))].filter((k) => !/-0+$/.test(k));

const perKey = {}; // key -> stats
const pairs = {}; // "B>A" -> {lines, firstAt}
const K = (k) => (perKey[k] ||= { commits: 0, added: 0, deleted: 0, self: 0, reworkOther: 0, recentNoKey: 0, older: 0, bulk: 0, reworkedBy: 0, reworkedBy90: 0, repos: {} });

async function scanRepo(repo) {
  if (!fs.existsSync(path.join(repo, '.git'))) return;
  const log = await git(repo, ['log', '--all', '--no-merges', `--since=${SINCE}`, '--format=%H%x1f%at%x1f%s']);
  if (!log) return;
  const seen = new Set();
  const commits = [];
  for (const line of log.split('\n')) {
    if (!line) continue;
    const [h, at, subj] = line.split('\x1f');
    const ks = keysOf(subj);
    if (!ks.length || +at < FROM) continue;
    const sig = `${at}|${subj}`; // cherry-picks across release branches
    if (seen.has(sig)) continue;
    seen.add(sig);
    commits.push({ h, at: +at, ks });
  }
  const name = path.basename(repo);
  for (const c of commits) {
    const diff = await git(repo, ['diff', '-U0', '--no-color', '-M', `${c.h}^`, c.h]);
    if (diff == null) continue;
    let oldPath = null;
    let newPath = null;
    const ranges = new Map(); // oldPath -> [[a,b]]
    let added = 0;
    let deleted = 0;
    for (const l of diff.split('\n')) {
      if (l.startsWith('--- ')) { oldPath = l.slice(4) === '/dev/null' ? null : l.slice(6); continue; }
      if (l.startsWith('+++ ')) { newPath = l.slice(4) === '/dev/null' ? null : l.slice(6); continue; }
      const m = l.match(/^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/);
      if (!m) continue;
      const p = newPath || oldPath;
      if (!p || SKIP.test(p)) continue;
      const a = +m[1];
      const b = m[2] === undefined ? 1 : +m[2];
      const d = m[4] === undefined ? 1 : +m[4];
      added += d;
      if (b > 0 && oldPath) {
        deleted += b;
        if (!ranges.has(oldPath)) ranges.set(oldPath, []);
        ranges.get(oldPath).push([a, a + b - 1]);
      }
    }
    for (const k of c.ks) {
      const s = K(k);
      s.commits++;
      s.added += added / c.ks.length;
      s.deleted += deleted / c.ks.length;
      s.repos[name] = (s.repos[name] || 0) + 1;
    }
    for (const [file, rs] of ranges) {
      const total = rs.reduce((x, [a, b]) => x + b - a + 1, 0);
      if (total > MAX_BLAME_LINES) { for (const k of c.ks) K(k).bulk += total / c.ks.length; continue; }
      const args = ['blame', '--porcelain'];
      for (const [a, b] of rs) args.push('-L', `${a},${b}`);
      args.push(`${c.h}^`, '--', file);
      const out = await git(repo, args);
      if (!out) continue;
      const info = {}; // sha -> {at, keys}
      let cur = null;
      const lineShas = [];
      for (const l of out.split('\n')) {
        const hm = l.match(/^([0-9a-f]{40}) \d+ \d+/);
        if (hm) { cur = hm[1]; info[cur] ||= {}; lineShas.push(cur); continue; }
        if (!cur) continue;
        if (l.startsWith('author-time ')) info[cur].at = +l.slice(12);
        else if (l.startsWith('summary ')) info[cur].keys = keysOf(l.slice(8));
      }
      for (const sha of lineShas) {
        const o = info[sha] || {};
        const ok = o.keys || [];
        const recent = o.at && c.at - o.at <= RECENT_S;
        for (const k of c.ks) {
          const s = K(k);
          const w = 1 / c.ks.length;
          if (ok.some((x) => c.ks.includes(x))) s.self += w;
          else if (recent && ok.length) {
            s.reworkOther += w;
            for (const a of ok) {
              const pk = `${k}>${a}`;
              const pr = (pairs[pk] ||= { lines: 0, at: c.at, originAt: o.at });
              pr.lines += w / ok.length;
              const A = K(a);
              A.reworkedBy += w / ok.length;
              if (c.at - o.at <= 90 * 86400) A.reworkedBy90 += w / ok.length;
            }
          } else if (recent) s.recentNoKey += w;
          else s.older += w;
        }
      }
    }
  }
}

async function run(input) {
  REPOS = input.repos || [];
  SINCE = input.since || SINCE;
  FROM = Date.parse(input.from || SINCE) / 1000;
  let i = 0;
  const lane = async () => { while (i < REPOS.length) { const r = REPOS[i++]; try { await scanRepo(r); } catch { /* skip repo */ } } };
  await Promise.all(Array.from({ length: 6 }, lane));
  const r2 = (v) => Math.round(v * 10) / 10;
  for (const st of Object.values(perKey)) { for (const f of ['added', 'deleted', 'self', 'reworkOther', 'recentNoKey', 'older', 'bulk', 'reworkedBy', 'reworkedBy90']) st[f] = r2(st[f]); delete st.repos; }
  for (const p of Object.values(pairs)) p.lines = r2(p.lines);
  return { generated_at: new Date().toISOString(), perKey, pairs };
}
module.exports = { run };
if (require.main === module) {
  let buf = '';
  process.stdin.on('data', (d) => (buf += d));
  process.stdin.on('end', async () => process.stdout.write(JSON.stringify(await run(JSON.parse(buf || '{}')))));
}
