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
//   (4) deploy tags (name contains deployed / hf / hotfix — see isDeployTag)
//       sorted by creatordate, each `--not <every earlier tag>` (tag.contains)
//       — a commit's first deploy; a key ships with its LAST commit
//
// Delivery model (matches the team's Bitbucket flow):
//   done_git_at   = first merge-event on develop|release mentioning the key
//                   (fallback: first direct on-target commit — single-commit
//                   flows like version bumps)
//   fix_*         = key commits/merges landing after done_git_at
//   deployed_at   = max over the key's commits of each commit's first deploy
//                   tag creatordate (first_deployed_at = the min)
// Depth 0 = unlimited.
'use strict';

const { execFileSync } = require('node:child_process');
const { createHash } = require('node:crypto');
const { TagRangeCache } = require('./tag-range-cache.js');
const digest = (value) => createHash('sha256').update(value).digest('hex');

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

// Target preference: the remote develop is the team's real integration branch
// (local clones often sit on a feature branch with a STALE local develop that
// silently hides every newer commit). Order: origin/develop → develop →
// origin/main → main → (any further configured targets, remote first).
const TARGET_ORDER = ['develop', 'main', 'master'];

/** First existing target ref (remote ref preferred per name). → 'origin/develop' | 'develop' | … | null */
function resolveTarget(repoPath, targets) {
  const names = [...new Set([...(targets || TARGET_ORDER)])];
  for (const t of names) {
    if (git(repoPath, ['rev-parse', '--verify', '--quiet', `refs/remotes/origin/${t}`]) !== null) return `origin/${t}`;
    if (git(repoPath, ['rev-parse', '--verify', '--quiet', `refs/heads/${t}`]) !== null) return t;
  }
  return null;
}

/** Days since the target ref's tip commit (committer date) — flags stale clones. */
function targetRefAgeDays(repoPath, ref, nowMs = Date.now()) {
  const out = git(repoPath, ['log', '-1', '--format=%ct', ref]);
  const ts = out ? parseInt(out.trim(), 10) * 1000 : NaN;
  if (Number.isNaN(ts)) return null;
  return Math.max(0, Math.round(((nowMs - ts) / 86400000) * 10) / 10);
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

// Deploy-tag rule (the ONE rule — phases.js and dora.js import it): a tag is a
// production deployment when its name CONTAINS (case-insensitive substring) one
// of the patterns — by default 'deployed', 'hf' or 'hotfix'. Hotfix tags are
// deployments too. Substring on purpose (the lead's rule is "contains"), so
// 'hotfixes-2026', 'prodHotfix1' and 'v2hf' all count. `config.deploy_tag_patterns`
// (or the legacy single `deploy_tag_pattern`) replaces the default list.
// tagKind: 'hotfix' when the name contains a hotfix pattern (config.hotfix_tag_patterns,
// default hf/hotfix; `hf_word_boundary` → /(^|[^a-z])hf/i), else 'deploy'.
const DEFAULT_DEPLOY_TAG_PATTERNS = ['deployed', 'hf', 'hotfix'];
const DEFAULT_HOTFIX_TAG_PATTERNS = ['hf', 'hotfix'];
// Optional `hf_word_boundary`: 'hf' only counts when NOT preceded by a letter
// (so 'shfoo' / 'pdf-hf' style accidents can be ruled out; 'rel-HF2' still hits).
const RE_HF_BOUNDARY = /(^|[^a-z])hf/i;

const normList = (list) => [...new Set(list.map((p) => String(p).trim().toLowerCase()).filter(Boolean))];

/** Hotfix pattern list: `config.hotfix_tag_patterns` or ['hf','hotfix']. (pure) */
function hotfixTagPatterns(config = {}) {
  const c = config && typeof config === 'object' && !Array.isArray(config) ? config : {};
  return normList(Array.isArray(c.hotfix_tag_patterns) ? c.hotfix_tag_patterns : DEFAULT_HOTFIX_TAG_PATTERNS);
}

/** Normalize config → lower-cased pattern list (legacy `deploy_tag_pattern` string merged in). */
function deployTagPatterns(config = {}) {
  const list = Array.isArray(config.deploy_tag_patterns) ? [...config.deploy_tag_patterns] : [...DEFAULT_DEPLOY_TAG_PATTERNS];
  if (typeof config.deploy_tag_pattern === 'string' && config.deploy_tag_pattern.trim()) list.push(config.deploy_tag_pattern);
  // Explicit hotfix patterns are deployments too (a hotfix tag IS a prod deploy).
  if (Array.isArray(config.hotfix_tag_patterns)) list.push(...config.hotfix_tag_patterns);
  return normList(list);
}

/** Pattern list from either a ready list or a config object (null → defaults). */
function patternsOf(config) {
  if (Array.isArray(config)) return normList(config);
  if (config && typeof config === 'object') return deployTagPatterns(config);
  return DEFAULT_DEPLOY_TAG_PATTERNS;
}

/** One pattern against a lower-cased name; 'hf' honours `hf_word_boundary`. */
function patternHit(lower, p, cfg) {
  if (p === 'hf' && cfg && cfg.hf_word_boundary) return RE_HF_BOUNDARY.test(lower);
  return lower.includes(p);
}

/** Does tag `name` count as a deployment? `config` = config object or pattern list. (pure) */
function isDeployTag(name, config) {
  const lower = String(name || '').toLowerCase();
  if (!lower) return false;
  const cfg = config && typeof config === 'object' && !Array.isArray(config) ? config : null;
  return patternsOf(config).some((p) => patternHit(lower, p, cfg));
}

/** 'hotfix' when the name contains a hotfix pattern (cfg.hotfix_tag_patterns, default hf/hotfix), else 'deploy'. (pure) */
function tagKind(name, config) {
  const lower = String(name || '').toLowerCase();
  const cfg = config && typeof config === 'object' && !Array.isArray(config) ? config : null;
  return hotfixTagPatterns(cfg || {}).some((p) => patternHit(lower, p, cfg)) ? 'hotfix' : 'deploy';
}

/**
 * Deploy tags of a repo, ascending by CREATORDATE (when the deploy was tagged —
 * a late tag on an old commit is a late deployment). `ts` = creatordate,
 * `commit_ts` = tagged commit's date, `sha` = tagged commit, `kind` = hotfix|deploy.
 * `patterns` may be a config object, an array or a legacy single string.
 */
function deployTags(repoPath, patterns) {
  const cfg = patterns && typeof patterns === 'object' && !Array.isArray(patterns) ? patterns : null;
  const pats = typeof patterns === 'string' ? deployTagPatterns({ deploy_tag_pattern: patterns, deploy_tag_patterns: [] }) : patternsOf(patterns);
  const out = git(repoPath, [
    'for-each-ref', 'refs/tags',
    `--format=%(creatordate:unix)${US}%(objectname)${US}%(*objectname)${US}%(committerdate:unix)${US}%(*committerdate:unix)${US}%(refname:short)`,
  ]);
  if (!out) return [];
  const tags = [];
  for (const line of out.split('\n')) {
    const p = line.split(US);
    if (p.length < 6) continue;
    const ts = parseInt(p[0], 10) * 1000;
    const name = p.slice(5).join(US).trim();
    if (Number.isNaN(ts) || !name || !(cfg ? isDeployTag(name, cfg) : isDeployTag(name, pats))) continue;
    const sha = p[2] || p[1];
    const cts = parseInt(p[4] || p[3], 10) * 1000;
    tags.push({ name, ts, sha, commit_ts: Number.isNaN(cts) ? ts : cts, kind: tagKind(name, cfg) });
  }
  tags.sort((a, b) => a.ts - b.ts || a.commit_ts - b.commit_ts || (a.name < b.name ? -1 : 1));
  return tags;
}

// Fixed-size prefix digests avoid retaining quadratic cache keys. The complete
// earlier SHA set is still excluded on cold misses, including divergent release
// branches; subtracting only the previous tag would change first-deploy dates.
const tagRangeCache = new TagRangeCache();
function tagContainsLog(repoPath, tag, earlierShas, prefix, cache) {
  const key = digest(JSON.stringify([repoPath, tag.name, tag.sha, prefix]));
  const cached = cache.get(key);
  if (cached !== undefined) return cached;
  if (earlierShas.has(tag.sha)) {
    cache.set(key, '');
    return '';
  }
  let out = null;
  try {
    out = execFileSync('git', ['-C', repoPath, 'log', tag.sha, `--pretty=%H${US}%s`, '--stdin'], {
      encoding: 'utf8',
      maxBuffer: 256 * 1024 * 1024,
      input: [...earlierShas].map((sha) => `^${sha}`).join('\n') + (earlierShas.size ? '\n' : ''),
    });
  } catch { /* failed ranges are never cached */ }
  if (out !== null) cache.set(key, out);
  return out;
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
 * config: {target_branches?, git_depth?, git_fetch?, deploy_tag_patterns?,
 *          deploy_tag_pattern? (legacy single string, merged in),
 *          feature_repos?: [name]} — repos listed in `feature_repos` also get
 *          git-only feature extraction (work without Jira stories).
 * → {byKey: Map<key, {first_commit_at, done_git_at, delivered_at, last_fix_at,
 *      fix_count, deployed_at, authors: [{name, email, commits}]}>,
 *    features: [...], target_used: {repo: branch}, fetched: {repo: bool},
 *    hasRepos}
 */
function buildIndex(repos, config = {}, ranges = tagRangeCache) {
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
  const deployTagList = []; // [{repo, name, ts, sha, kind}] — DORA input
  const targetAge = {}; // repo -> days since the target ref's tip commit
  const matchedTags = {}; // repo -> [deploy tag names] (UI guardrail: did the rule match anything?)
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
        deployed_tag: null, // name of the earliest deploy tag reaching the key
        deployed_kind: null, // 'hotfix' | 'deploy'
        first_deployed_at: null, // first deploy tag shipping ANY key commit
        first_deployed_tag: null,
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

    // (4) deploy tags sorted by CREATORDATE. tag.contains = commits (merges
    // included) not reachable from ANY earlier-created deploy tag, so every
    // commit belongs to exactly one tag — its FIRST deployment. A key is
    // deployed when its LAST commit ships: deployed_at = max over the key's
    // commits of that commit's first deploy; first_deployed_at = the min.
    const tags = deployTags(r.path, config);
    targetAge[r.name] = targetRefAgeDays(r.path, target);
    const matched = [];
    const earlierShas = new Set();
    let prefix = digest('deploy-ranges-v1');
    for (let i = 0; i < tags.length; i++) {
      const tag = tags[i];
      const out = tagContainsLog(r.path, tag, earlierShas, prefix, ranges);
      earlierShas.add(tag.sha);
      prefix = digest(JSON.stringify([prefix, tag.name, tag.sha]));
      const contains = [];
      if (out !== null) {
        for (const line of out.split('\n')) {
          const parts = line.split(US);
          if (parts.length < 2 || !parts[0]) continue;
          contains.push(parts[0]);
          const feat = featureByMergeHash.get(parts[0]);
          if (feat && (feat.deployed_at == null || tag.ts < feat.deployed_at)) {
            feat.deployed_at = tag.ts;
            feat.deployed_tag = tag.name;
          }
          const keys = parts.slice(1).join(US).match(KEY_RE);
          if (!keys) continue;
          for (const k of new Set(keys)) {
            if (PLACEHOLDER_RE.test(k)) continue;
            const e = entry(k);
            if (e.deployed_at === null || tag.ts > e.deployed_at) {
              e.deployed_at = tag.ts;
              e.deployed_tag = tag.name;
              e.deployed_kind = tag.kind;
            }
            if (e.first_deployed_at === null || tag.ts < e.first_deployed_at) {
              e.first_deployed_at = tag.ts;
              e.first_deployed_tag = tag.name;
            }
          }
        }
      }
      deployTagList.push({ repo: r.name, name: tag.name, ts: tag.ts, sha: tag.sha, kind: tag.kind, contains });
      matched.push(tag.name);
    }
    matchedTags[r.name] = matched;
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
    target_ref_age_days: targetAge,
    deploy_tags: deployTagList.sort((a, b) => a.ts - b.ts),
    matched_tags: matchedTags,
    fetched,
    hasRepos,
  };
}

module.exports = {
  buildIndex,
  featureIndex,
  branchOfMerge,
  KEY_RE,
  PLACEHOLDER_RE,
  DEFAULT_DEPLOY_TAG_PATTERNS,
  DEFAULT_HOTFIX_TAG_PATTERNS,
  hotfixTagPatterns,
  deployTags,
  deployTagPatterns,
  isDeployTag,
  tagKind,
  releaseBranches,
  resolveTarget,
  targetRefAgeDays,
};

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
      const { repos, config, tag_cache_path } = JSON.parse(buf || '{}');
      const cfg = { ...(config || {}) };
      let fetched = {};
      if (cfg.git_fetch !== false) fetched = await prefetch(repos || []);
      cfg.git_fetch = false;
      const ranges = new TagRangeCache(tag_cache_path);
      const idx = buildIndex(repos || [], cfg, ranges);
      ranges.save();
      idx.fetched = fetched;
      process.stdout.write(
        JSON.stringify({
          by_key: Object.fromEntries(idx.byKey),
          features: idx.features,
          unscoped: idx.unscoped,
          repo_activity: idx.repo_activity,
          target_used: idx.target_used,
          target_ref_age_days: idx.target_ref_age_days,
          deploy_tags: idx.deploy_tags,
          matched_tags: idx.matched_tags,
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
