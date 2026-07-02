// One-pass git delivery index. Per repo: two `git log` calls total —
//   (1) the target branch's history  -> delivered_at(key) = max ts of a
//       key-mentioning commit reachable from the target (history membership ≡
//       ancestry; merge-commit subjects carry branch names, covering PR flows)
//   (2) --all                        -> first_commit_at(key) = min ts
// Never a per-issue subprocess call. Depth-bounded (config `git_depth`).
'use strict';

const { execFileSync } = require('node:child_process');

const KEY_RE = /\b[A-Z][A-Z0-9]+-\d+\b/g;

function git(repoPath, args) {
  try {
    return execFileSync('git', ['-C', repoPath, ...args], {
      encoding: 'utf8',
      maxBuffer: 64 * 1024 * 1024,
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

function scanLog(out, onHit) {
  if (!out) return;
  for (const line of out.split('\n')) {
    const idx = line.indexOf('\x1f');
    if (idx <= 0) continue;
    const ts = parseInt(line.slice(0, idx), 10) * 1000;
    if (Number.isNaN(ts)) continue;
    const subject = line.slice(idx + 1);
    const keys = subject.match(KEY_RE);
    if (keys) for (const k of new Set(keys)) onHit(k, ts);
  }
}

/**
 * Build the delivery index across registered repos.
 * repos: [{name, path}] · config: {target_branches?, git_depth?}
 * → {byKey: Map<key,{first_commit_at, delivered_at}>, target_used: {repo:branch},
 *    hasRepos}
 */
function buildIndex(repos, config = {}) {
  const targets = config.target_branches || ['develop', 'main', 'master'];
  const depth = String(config.git_depth || 5000);
  const byKey = new Map();
  const targetUsed = {};
  let hasRepos = false;

  const entry = (k) => {
    if (!byKey.has(k)) byKey.set(k, { first_commit_at: null, delivered_at: null });
    return byKey.get(k);
  };

  for (const r of repos || []) {
    const target = resolveTarget(r.path, targets);
    if (target === null) continue; // unreadable / not a repo
    hasRepos = true;
    targetUsed[r.name] = target.replace(/^origin\//, '');

    scanLog(git(r.path, ['log', target, '-n', depth, '--pretty=%ct\x1f%s']), (k, ts) => {
      const e = entry(k);
      if (e.delivered_at === null || ts > e.delivered_at) e.delivered_at = ts;
    });
    scanLog(git(r.path, ['log', '--all', '-n', depth, '--pretty=%ct\x1f%s']), (k, ts) => {
      const e = entry(k);
      if (e.first_commit_at === null || ts < e.first_commit_at) e.first_commit_at = ts;
    });
  }

  return { byKey, target_used: targetUsed, hasRepos };
}

module.exports = { buildIndex, KEY_RE };
