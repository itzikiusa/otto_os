// The view period is a data boundary, shared by team and person denominators.
// Capacity currently counts UTC midnights (no tz option is passed by views), so
// an All time start includes the entire first observed UTC day as well.
'use strict';
const DAY = 86400000;
const RECORD_TIMES = ['created', 'created_at', 'first_active_at', 'started_at', 'eff_start_at', 'first_commit_at', 'done_at', 'eff_done_at', 'done_git_at', 'first_deployed_at', 'deployed_at'];

function resolveViewWindow(scope, sinceMs, until = Date.now()) {
  if (!Number.isFinite(until) || until <= 0) throw new TypeError('view window requires a finite positive end');
  if (Number.isFinite(sinceMs) && sinceMs > 0) return { since: sinceMs, until };
  let earliest = until;
  let observed = false;
  const consider = (value) => {
    if (value == null || value === '' || (typeof value !== 'number' && typeof value !== 'string')) return;
    const ms = typeof value === 'number' ? value : /^\d+$/.test(value) ? Number(value) : Date.parse(value);
    if (Number.isFinite(ms) && ms > 0 && ms <= until) {
      earliest = Math.min(earliest, ms);
      observed = true;
    }
  };
  // Only selected records establish ownership. Raw PR/tag caches are global:
  // Jira keys can collide across accounts and repo/tag names are not enough to
  // associate a deployment. Owned Git history is already carried by records.
  for (const record of scope.records || []) {
    for (const field of RECORD_TIMES) consider(record[field]);
  }
  return { since: observed ? Math.floor(earliest / DAY) * DAY : until, until };
}
module.exports = { resolveViewWindow };
