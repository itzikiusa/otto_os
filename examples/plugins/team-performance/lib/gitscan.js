// One-pass git delivery index (v2 — git is the primary timing signal).
//
// Per repo, a bounded number of `git log` / `for-each-ref` calls total — never
// a per-issue subprocess:
//   (0) optional `git fetch --prune` (moderated: once per scan, 30s timeout,
//       failures swallowed — offline scans still work on the local clone)
//   (1) target branch log (develop → main → master) with parent info —
//       merge-events vs direct commits per key
//   (2) each release/* branch, `--not target` (release-only commits are few)
//   (3) `--all` with authors — first_commit_at + per-key non-merge authors
//       (multi-dev credit)
//   (4) *-DEPLOYED* tags ascending by creatordate, each `--not prevTags` so
//       every commit is visited once — earliest prod deployment per key
//
// Delivery model (matches the team's Bitbucket flow):
//   done_git_at   = first merge-event on develop|release mentioning the key
//                   (fallback: first direct on-target commit — single-commit
//                   flows like version bumps)
//   fix_*         = key commits/merges landing after done_git_at
//   deployed_at   = creatordate of the earliest deploy tag reaching the key
// Depth 0 = unlimited.
'use strict';

const { execFileSync } = require('node:child_process');

const KEY_RE = /\b[A-Z][A-Z0-9]+-\d+\b/g;
// Catch-all keys devs use for unscoped fixes / prod issues (ABC-0000, ABC-000 …)
// — real work that belongs to no story. Tracked separately per author.
const PLACEHOLDER_RE = /^[A-Z][A-Z0-9]*-0+$/;
const US = '\x1f';
const RS = '\x1e';

function git(repoPath, args, opts = {}) {
  try {
    return execFileSync('git', ['-C', repoPath, ...args], {
      encoding: 'utf8',
      maxBuffer: 256 * 1024 * 1024,
      timeout: opts.timeout || 0,
    });
  } catch {
    return null;
  }
}

/** First existing target branch on the repo (develop → main → master). */
function resolveTarget(repoPath, targets) {
  for (const t of targets) {
    if (git(repoPath, ['rev-parse', '--verify', '--quiet', `refs/heads/${t}`]) !== null) return t;
    // Fall back to a remote-tracking ref when the local branch doesn't exist.
    if (git(repoPath, ['rev-parse', '--verify', '--quiet', `refs/remotes/origin/${t}`]) !== null) return `origin/${t}`;
  }
  return null;
}

/** All release/* + hotfix/* refs (local + origin), deduped by short name. */
function releaseBranches(repoPath) {
  const out = git(repoPath, [
    'for-each-ref',
    'refs/heads/release', 'refs/remotes/origin/release',
    'refs/heads/hotfix', 'refs/remotes/origin/hotfix',
    '--format=%(refname:short)',
  ]);
  if (!out) return [];
  const seen = new Set();
  const refs = [];
  for (const line of out.split('\n')) {
    const ref = line.trim();
    if (!ref) continue;
    const short = ref.replace(/^origin\//, '');
    if (seen.has(short)) continue;
    seen.add(short);
    refs.push(ref);
  }
  return refs;
}

/** Deploy tags (name matches `pattern`, case-insensitive substring), ascending by creatordate. */
function deployTags(repoPath, pattern) {
  const out = git(repoPath, ['for-each-ref', 'refs/tags', '--format=%(creatordate:unix)\x1f%(refname:short)']);
  if (!out) return [];
  const needle = String(pattern || 'deployed').toLowerCase();
  const tags = [];
  for (const line of out.split('\n')) {
    const idx = line.indexOf(US);
    if (idx <= 0) continue;
    const ts = parseInt(line.slice(0, idx), 10) * 1000;
    const name = line.slice(idx + 1).trim();
    if (Number.isNaN(ts) || !name) continue;
    if (name.toLowerCase().includes(needle)) tags.push({ name, ts });
  }
  tags.sort((a, b) => a.ts - b.ts);
  return tags;
}

/** Parse `git log` output where each line is US-joined fields, last field = subject. */
function scanLog(out, nFields, onLine) {
  if (!out) return;
  for (const line of out.split('\n')) {
    const parts = line.split(US);
    if (parts.length < nFields) continue;
    const ts = parseInt(parts[0], 10) * 1000;
    if (Number.isNaN(ts)) continue;
    const subject = parts.slice(nFields - 1).join(US); // subjects may contain the separator
    const keys = subject.match(KEY_RE);
    if (keys) onLine(parts, ts, [...new Set(keys)]);
  }
}

function depthArgs(depth) {
  const n = Number(depth) || 0;
  return n > 0 ? ['-n', String(n)] : [];
}

// ---------------------------------------------------------------------------
// Git-only feature extraction (work that never had a Jira story — e.g.
// automation repos): every merge into the target is a "feature"; its commits
// are recovered from the parent graph (one full-graph `git log`, no per-merge
// subprocess), so timing = first branch commit → merge.
// ---------------------------------------------------------------------------

const RE_MERGE_BRANCH = [
  /Merged in ([^\s']+) \(pull request/i, // Bitbucket
  /Merge pull request #\d+ (?:in [^\s]+ )?from [^\s/]+\/([^\s]+)/i, // GitHub
  /Merge branch '([^']+)'/i,
  /Merge remote-tracking branch '(?:origin\/)?([^']+)'/i,
];

function branchOfMerge(subject) {
  for (const re of RE_MERGE_BRANCH) {
    const m = re.exec(subject);
    if (m) {
      const b = m[1].replace(/^origin\//, '');
      // Merge-backs between long-lived branches are not features.
      if (/^(develop|main|master|release\/|hotfix\/|feature\/release)/i.test(b) && !/^(feature|bugfix|fix|task|chore)\//i.test(b)) return null;
      return b;
    }
  }
  return null;
}

/**
 * Extract merged features of one repo from its target-branch graph.
 * → [{id, repo, branch, summary, merge_hash, merged_at, first_commit_at,
 *     commit_count, subjects: [..≤12], authors: [{name,email,commits}],
 *     jira_keys: [..]}]
 */
function featureIndex(repoName, repoPath, target, depth) {
  const out = git(repoPath, ['log', target, ...depth, `--pretty=%H${US}%P${US}%ct${US}%an${US}%ae${US}%s`]);
  if (!out) return [];
  const nodes = new Map(); // hash -> {parents, ts, an, ae, subject}
  let tip = null;
  for (const line of out.split('\n')) {
    const p = line.split(US);
    if (p.length < 6) continue;
    const ts = parseInt(p[2], 10) * 1000;
    if (Number.isNaN(ts)) continue;
    const hash = p[0];
    if (tip === null) tip = hash; // `git log` emits the tip first
    nodes.set(hash, { parents: p[1] ? p[1].split(' ') : [], ts, an: p[3], ae: p[4], subject: p.slice(5).join(US) });
  }
  // Mainline = the first-parent chain from the tip.
  const mainline = new Set();
  for (let h = tip; h && nodes.has(h) && !mainline.has(h); h = nodes.get(h).parents[0]) mainline.add(h);

  const claimed = new Set(); // commits already attributed to a feature
  const features = [];
  for (const h of mainline) {
    const n = nodes.get(h);
    if (n.parents.length < 2) continue;
    const branch = branchOfMerge(n.subject);
    if (!branch) continue;
    // BFS from the merged head; stop at mainline / other features / truncation.
    const commits = [];
    const q = n.parents.slice(1);
    while (q.length) {
      const c = q.pop();
      if (!nodes.has(c) || mainline.has(c) || claimed.has(c)) continue;
      claimed.add(c);
      const cn = nodes.get(c);
      if (cn.parents.length < 2) commits.push({ ts: cn.ts, an: cn.an, ae: cn.ae, subject: cn.subject });
      q.push(...cn.parents);
    }
    if (!commits.length) continue;
    commits.sort((a, b) => a.ts - b.ts);
    const authors = new Map();
    const keys = new Set((n.subject.match(KEY_RE) || []).filter((k) => !PLACEHOLDER_RE.test(k)));
    for (const c of commits) {
      const who = `${c.an}${US}${c.ae}`;
      authors.set(who, (authors.get(who) || 0) + 1);
      for (const k of c.subject.match(KEY_RE) || []) if (!PLACEHOLDER_RE.test(k)) keys.add(k);
    }
    features.push({
      id: `${repoName}:${branch}@${h.slice(0, 7)}`,
      repo: repoName,
      branch,
      summary: branch.replace(/^[a-z]+\//i, '').replace(/[-_]+/g, ' ').trim(),
      merge_hash: h,
      merged_at: n.ts,
      first_commit_at: commits[0].ts,
      commit_count: commits.length,
      subjects: commits.slice(0, 12).map((c) => c.subject.slice(0, 120)),
      authors: [...authors.entries()]
        .map(([who, count]) => {
          const [name, email] = who.split(US);
          return { name, email, commits: count };
        })
        .sort((a, b) => b.commits - a.commits),
      jira_keys: [...keys],
    });
  }
  return features;
}

/**
 * Build the delivery index across registered repos.
 * repos: [{name, path}]
 * config: {target_branches?, git_depth?, git_fetch?, deploy_tag_pattern?,
 *          feature_repos?: [name]} — repos listed in `feature_repos` also get
 *          git-only feature extraction (work without Jira stories).
 * → {byKey: Map<key, {first_commit_at, done_git_at, delivered_at, last_fix_at,
 *      fix_count, deployed_at, authors: [{name, email, commits}]}>,
 *    features: [...], target_used: {repo: branch}, fetched: {repo: bool},
 *    hasRepos}
 */
function buildIndex(repos, config = {}) {
  const targets = config.target_branches || ['develop', 'main', 'master'];
  const depth = depthArgs(config.git_depth);
  const byKey = new Map();
  const targetUsed = {};
  const fetched = {};
  const features = [];
  const unscoped = new Map(); // "name\x1femail" -> {name,email,commits,monthly,last_at}
  const repoActivity = new Map(); // feature-repo raw commit activity per author
  const changeByKey = new Map(); // key -> diff evidence for the AI estimator
  const change = (k) => {
    let c = changeByKey.get(k);
    if (!c) {
      c = { commits: 0, fileCount: 0, ins: 0, del: 0, files: new Set(), subjects: new Set(), repos: {} };
      changeByKey.set(k, c);
    }
    return c;
  };
  const evidenceSinceArg = config.evidence_since ? [`--since=${config.evidence_since}`] : [];
  const featureRepos = new Set(config.feature_repos || []);
  let hasRepos = false;

  const entry = (k) => {
    let e = byKey.get(k);
    if (!e) {
      e = {
        first_commit_at: null,
        done_git_at: null,
        delivered_at: null, // kept for back-compat: last on-target key commit
        last_fix_at: null,
        fix_count: 0,
        deployed_at: null,
        commit_ts: [], // sampled commit timestamps (capped) — the QA-rework check
        authors: new Map(), // "name\x1femail" -> count (converted to array at the end)
      };
      byKey.set(k, e);
    }
    return e;
  };

  // Per-key on-target events across ALL repos/branches; resolved into
  // done/fixes after every repo is scanned (a key may span repos).
  const events = new Map(); // key -> [{ts, merge: bool}]
  const addEvent = (k, ts, merge) => {
    if (!events.has(k)) events.set(k, []);
    events.get(k).push({ ts, merge });
  };

  for (const r of repos || []) {
    if (config.git_fetch !== false) {
      // Moderated: one fetch per repo per scan; network failures are fine.
      fetched[r.name] = git(r.path, ['fetch', '--prune', '--quiet'], { timeout: 30000 }) !== null;
    }
    const target = resolveTarget(r.path, targets);
    if (target === null) continue; // unreadable / not a repo
    hasRepos = true;
    targetUsed[r.name] = target.replace(/^origin\//, '');

    // (1) target history: ts, parents, subject — merge = multiple parents.
    scanLog(git(r.path, ['log', target, ...depth, `--pretty=%ct${US}%P${US}%s`]), 3, (parts, ts, keys) => {
      const merge = parts[1].trim().includes(' ');
      for (const k of keys) {
        if (PLACEHOLDER_RE.test(k)) continue;
        addEvent(k, ts, merge);
        const e = entry(k);
        if (e.delivered_at === null || ts > e.delivered_at) e.delivered_at = ts;
      }
    });

    // (2) release-only commits (fixes waiting for prod, or done-without-develop).
    for (const ref of releaseBranches(r.path)) {
      scanLog(git(r.path, ['log', ref, '--not', target, ...depth, `--pretty=%ct${US}%P${US}%s`]), 3, (parts, ts, keys) => {
        const merge = parts[1].trim().includes(' ');
        for (const k of keys) if (!PLACEHOLDER_RE.test(k)) addEvent(k, ts, merge);
      });
    }

    // (3) everything: first commit + authorship (non-merge commits only — the
    // merger of a PR is not the author of the work). Placeholder keys
    // (ABC-0000 …) don't index as issues; they accumulate per-author unscoped
    // fix work instead.
    scanLog(git(r.path, ['log', '--all', ...depth, `--pretty=%ct${US}%P${US}%an${US}%ae${US}%s`]), 5, (parts, ts, keys) => {
      const merge = parts[1].trim().includes(' ');
      for (const k of keys) {
        if (PLACEHOLDER_RE.test(k)) {
          if (merge) continue;
          const who = `${parts[2]}${US}${parts[3]}`;
          let u = unscoped.get(who);
          if (!u) {
            u = { name: parts[2], email: parts[3], commits: 0, monthly: {}, last_at: null };
            unscoped.set(who, u);
          }
          u.commits++;
          const d = new Date(ts);
          const ym = `${d.getUTCFullYear()}-${String(d.getUTCMonth() + 1).padStart(2, '0')}`;
          u.monthly[ym] = (u.monthly[ym] || 0) + 1;
          if (u.last_at === null || ts > u.last_at) u.last_at = ts;
          continue;
        }
        const e = entry(k);
        if (e.first_commit_at === null || ts < e.first_commit_at) e.first_commit_at = ts;
        if (e.commit_ts.length < 200) e.commit_ts.push(ts);
        if (!merge) {
          const who = `${parts[2]}${US}${parts[3]}`;
          e.authors.set(who, (e.authors.get(who) || 0) + 1);
        }
      }
    });

    // Git-only features for opted-in repos (work without Jira stories).
    const isFeatureRepo = featureRepos.has(r.name);
    const repoFeatures = isFeatureRepo ? featureIndex(r.name, r.path, target, depth) : [];
    const featureByMergeHash = new Map(repoFeatures.map((f) => [f.merge_hash, f]));
    features.push(...repoFeatures);

    // Raw per-author commit activity for feature repos: automation/test people
    // commit DIRECTLY to master with no story key and no PR branch, so neither
    // the key index nor the merged-feature path sees their work. Tally every
    // non-merge commit by author (per repo, per month) so their output is
    // visible. Display-only — not folded into weighted throughput (the merged
    // features already credit that), so nothing is double-counted.
    if (isFeatureRepo) {
      // Raw walk (NOT scanLog — that only fires on key-bearing subjects; these
      // commits have no keys). One line per non-merge commit: ts, author.
      const raw = git(r.path, ['log', target, '--no-merges', ...depth, `--pretty=%ct${US}%an${US}%ae`]);
      if (raw) {
        for (const line of raw.split('\n')) {
          const p = line.split(US);
          if (p.length < 3) continue;
          const ts = parseInt(p[0], 10) * 1000;
          if (Number.isNaN(ts)) continue;
          const who = `${p[1]}${US}${p[2]}`;
          let a = repoActivity.get(who);
          if (!a) {
            a = { name: p[1], email: p[2], commits: 0, monthly: {}, repos: {}, last_at: null };
            repoActivity.set(who, a);
          }
          a.commits++;
          a.repos[r.name] = (a.repos[r.name] || 0) + 1;
          const ym = `${new Date(ts).getUTCFullYear()}-${String(new Date(ts).getUTCMonth() + 1).padStart(2, '0')}`;
          a.monthly[ym] = (a.monthly[ym] || 0) + 1;
          if (a.last_at === null || ts > a.last_at) a.last_at = ts;
        }
      }
    }

    // (3b) change evidence: per-key diff stats (files, +/- lines, commit
    // subjects, per-repo split) so the AI estimator sizes tasks from the ACTUAL
    // code change, not the ticket prose. Bounded by `evidence_since` (numstat
    // is verbose) — everything the estimation window covers.
    if (evidenceSinceArg.length) {
      const raw = git(r.path, ['log', target, '--no-merges', '--numstat', ...evidenceSinceArg, `--pretty=${RS}%s`], {
        maxBuffer: 512 * 1024 * 1024,
      });
      if (raw) {
        let curKeys = [];
        for (const line of raw.split('\n')) {
          if (line.charCodeAt(0) === 0x1e) {
            const subject = line.slice(1);
            curKeys = [...new Set((subject.match(KEY_RE) || []).filter((k) => !PLACEHOLDER_RE.test(k)))];
            for (const k of curKeys) {
              const c = change(k);
              c.commits++;
              if (c.subjects.size < 10) c.subjects.add(subject.slice(0, 120));
              c.repos[r.name] = c.repos[r.name] || { commits: 0, ins: 0, del: 0, files: 0 };
              c.repos[r.name].commits++;
            }
            continue;
          }
          if (!curKeys.length) continue;
          const tab1 = line.indexOf('\t');
          if (tab1 <= 0) continue;
          const tab2 = line.indexOf('\t', tab1 + 1);
          if (tab2 < 0) continue;
          const addS = line.slice(0, tab1);
          const delS = line.slice(tab1 + 1, tab2);
          const path = line.slice(tab2 + 1);
          const add = addS === '-' ? 0 : parseInt(addS, 10) || 0;
          const del = delS === '-' ? 0 : parseInt(delS, 10) || 0;
          for (const k of curKeys) {
            const c = change(k);
            c.ins += add;
            c.del += del;
            c.fileCount++;
            if (c.files.size < 40) c.files.add(path);
            const rr = c.repos[r.name];
            if (rr) {
              rr.ins += add;
              rr.del += del;
              rr.files++;
            }
          }
        }
      }
    }

    // (4) deploy tags ascending; `--not prev` visits each commit once, so the
    // first tag that reaches a key (or a feature's merge commit) is its
    // earliest prod deployment.
    const tags = deployTags(r.path, config.deploy_tag_pattern);
    const prev = [];
    for (const tag of tags) {
      const args = ['log', tag.name, ...prev.flatMap((p) => ['--not', p]), `--pretty=%ct${US}%H${US}%s`];
      const out = git(r.path, args);
      if (out !== null) {
        for (const line of out.split('\n')) {
          const parts = line.split(US);
          if (parts.length < 3) continue;
          const hash = parts[1];
          const feat = featureByMergeHash.get(hash);
          if (feat && (feat.deployed_at === undefined || feat.deployed_at === null)) feat.deployed_at = tag.ts;
          const keys = parts.slice(2).join(US).match(KEY_RE);
          if (!keys) continue;
          for (const k of new Set(keys)) {
            if (PLACEHOLDER_RE.test(k)) continue;
            const e = entry(k);
            if (e.deployed_at === null || tag.ts < e.deployed_at) e.deployed_at = tag.ts;
          }
        }
      }
      prev.push(tag.name);
    }
  }

  // Resolve on-target events into done/fix per key. Merge-events win when any
  // exist (PR flow); otherwise the first direct commit is the delivery.
  // Fixes only count within `fix_window_days` of delivery — a stray commit
  // that mentions the key months later is unrelated re-touch, not fixing THIS
  // delivery, and must not show up as (e.g.) "60 days of fixes".
  const fixWindowMs = (Number(config.fix_window_days) > 0 ? config.fix_window_days : 30) * 86400000;
  for (const [k, evs] of events) {
    const e = entry(k);
    evs.sort((a, b) => a.ts - b.ts);
    const merges = evs.filter((ev) => ev.merge);
    const done = merges.length ? merges[0].ts : evs[0].ts;
    e.done_git_at = done;
    const fixes = evs.filter((ev) => ev.ts > done && ev.ts <= done + fixWindowMs);
    e.fix_count = fixes.length;
    e.last_fix_at = fixes.length ? fixes[fixes.length - 1].ts : null;
    e.late_touches = evs.filter((ev) => ev.ts > done + fixWindowMs).length;
  }

  // Freeze author maps into plain arrays (records are JSON-persisted).
  for (const e of byKey.values()) {
    e.commit_ts = [...new Set(e.commit_ts)].sort((a, b) => a - b);
    e.authors = [...e.authors.entries()]
      .map(([who, commits]) => {
        const [name, email] = who.split(US);
        return { name, email, commits };
      })
      .sort((a, b) => b.commits - a.commits);
  }

  for (const f of features) if (f.deployed_at === undefined) f.deployed_at = null;

  // Fold diff evidence into each key's entry (Sets → capped arrays for JSON).
  for (const [k, c] of changeByKey) {
    entry(k).change = {
      commits: c.commits,
      files: c.fileCount,
      insertions: c.ins,
      deletions: c.del,
      sample_files: [...c.files],
      subjects: [...c.subjects],
      repos: Object.entries(c.repos)
        .map(([name, s]) => ({ name, ...s }))
        .sort((a, b) => b.ins + b.del - (a.ins + a.del)),
    };
  }

  return {
    byKey,
    features,
    unscoped: [...unscoped.values()].sort((a, b) => b.commits - a.commits),
    repo_activity: [...repoActivity.values()].sort((a, b) => b.commits - a.commits),
    target_used: targetUsed,
    fetched,
    hasRepos,
  };
}

module.exports = { buildIndex, featureIndex, branchOfMerge, KEY_RE, PLACEHOLDER_RE, deployTags, releaseBranches, resolveTarget };

// Worker mode: `node lib/gitscan.js` with {repos, config} JSON on stdin prints
// the serialized index on stdout. The git walk is all blocking execFileSync —
// running it in a child keeps the sidecar's event loop (scan status, views)
// responsive through multi-minute fetch+log passes.
//
// Fetch runs FIRST as a parallel pool (the walk then runs with fetch off):
// with a large repo fleet, serial 30s-timeout fetches alone could take the
// better part of an hour.
async function prefetch(repos, concurrency = 8) {
  const { spawn } = require('node:child_process');
  const queue = [...repos];
  const fetched = {};
  const one = (r) =>
    new Promise((resolve) => {
      const child = spawn('git', ['-C', r.path, 'fetch', '--prune', '--quiet'], { stdio: 'ignore' });
      const timer = setTimeout(() => child.kill('SIGKILL'), 30000);
      child.on('close', (code) => {
        clearTimeout(timer);
        fetched[r.name] = code === 0;
        resolve();
      });
      child.on('error', () => {
        clearTimeout(timer);
        fetched[r.name] = false;
        resolve();
      });
    });
  const lanes = Array.from({ length: concurrency }, async () => {
    while (queue.length) await one(queue.shift());
  });
  await Promise.all(lanes);
  return fetched;
}

if (require.main === module) {
  let buf = '';
  process.stdin.on('data', (c) => (buf += c));
  process.stdin.on('end', async () => {
    try {
      const { repos, config } = JSON.parse(buf || '{}');
      const cfg = { ...(config || {}) };
      let fetched = {};
      if (cfg.git_fetch !== false) fetched = await prefetch(repos || []);
      cfg.git_fetch = false;
      const idx = buildIndex(repos || [], cfg);
      idx.fetched = fetched;
      process.stdout.write(
        JSON.stringify({
          by_key: Object.fromEntries(idx.byKey),
          features: idx.features,
          unscoped: idx.unscoped,
          repo_activity: idx.repo_activity,
          target_used: idx.target_used,
          fetched: idx.fetched,
          hasRepos: idx.hasRepos,
        }),
      );
    } catch (e) {
      console.error('gitscan worker failed:', e);
      process.exit(1);
    }
  });
}
