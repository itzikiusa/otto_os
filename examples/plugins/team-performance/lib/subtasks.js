// Sub-task attribution: replaces the blanket "every sub-task rolls up" pass.
//
// Most sub-tasks are checklists of their story (same person, same time) and
// roll up — but a sub-task can carry REAL work of its own:
//   substance:  own commits on >= min_commit_days distinct days, or
//               dev_days >= min_dev_days
//   and either: a different assignee than the parent story (B's work under
//               A's story), or the parent has no own timing at all
// Substantive sub-tasks stay standalone (rollup=false, substantive_subtask=true)
// and are credited to their own assignee. When the parent has no own timing
// and the sub-task is the SAME person's, the story adopts the sub-task's
// timing instead (one unit, one credit — the sub-task is surfaced but rolls up).
// Checklist sub-tasks roll up by the UNION of their dev intervals with the
// story's (parallel sub-tasks are not double counted). Design sub-tasks feed
// the parent's design phase and never its dev rollup. Pure — no I/O.
'use strict';
const A = require('./analytics.js');
const P = require('./phases.js');
const TZ = require('./tz.js');

// One design vocabulary for the whole plugin: phases.js owns it.
const { RE_DESIGN } = P;
const DEV_PHASES = new Set(['implementation']);
const round2 = (v) => Math.round(v * 100) / 100;

/** Distinct LOCAL commit days in the team's zone (a 23:30 UTC+3 commit is that local date). */
const commitDays = (r, tz) => TZ.distinctDays(r.commit_ts || [], tz || 'UTC').length;
const commitCount = (r) => {
  const ts = (r.commit_ts || []).length;
  const au = (r.git_authors || []).reduce((a, x) => a + (x.commits || 0), 0);
  return Math.max(ts, au);
};
const hasOwnTiming = (r) => (r.dev_days || 0) > 0 || (r.commit_ts || []).length > 0 || r.first_commit_at != null || r.manual_days != null;
const devIntervals = (r) => (r.intervals || []).filter((iv) => DEV_PHASES.has(iv.phase) && iv.to > iv.from).map((iv) => [iv.from, iv.to]);

/** Merge overlapping [from, to] intervals. */
function unionIntervals(ivs) {
  const s = [...ivs].sort((a, b) => a[0] - b[0]);
  const out = [];
  for (const [f, t] of s) {
    const last = out[out.length - 1];
    if (last && f <= last[1]) last[1] = Math.max(last[1], t);
    else out.push([f, t]);
  }
  return out;
}
const spanDays = (ivs, workweek) => ivs.reduce((a, [f, t]) => a + A.businessDays(f, t, workweek), 0);

function mergeAuthors(lists) {
  const merged = new Map();
  for (const a of lists.flat()) {
    const k = `${a.name}|${a.email}`;
    const m = merged.get(k) || { ...a, commits: 0 };
    m.commits += a.commits || 0;
    merged.set(k, m);
  }
  return [...merged.values()];
}

/**
 * opts: { min_commit_days = 1, min_dev_days = 0.5, workweek, timezone }
 * Returns a new array; inputs untouched.
 */
function classifySubtasks(records, opts = {}) {
  const minCommitDays = opts.min_commit_days ?? 1;
  const minDevDays = opts.min_dev_days ?? 0.5;
  const parents = new Map();
  for (const r of records) if (!r.subtask) parents.set(r.key, r);
  const kids = new Map(); // parent key -> {dev: [record], design: [record], adopt: [record]}
  const out = records.map((r) => {
    if (!r.subtask || !r.parent_key || !parents.has(r.parent_key)) return r;
    const p = parents.get(r.parent_key);
    const g = kids.get(p.key) || { dev: [], design: [], adopt: [] };
    kids.set(p.key, g);
    if (RE_DESIGN.test(r.type || '') || RE_DESIGN.test(r.summary || '')) {
      g.design.push(r);
      return { ...r, rollup: true, design_subtask: true };
    }
    const substance = commitDays(r, opts.timezone) >= minCommitDays || (r.dev_days || 0) >= minDevDays;
    const otherPerson = Boolean(r.assignee_id) && r.assignee_id !== p.assignee_id;
    const parentTimed = hasOwnTiming(p);
    if (substance && otherPerson) {
      return { ...r, rollup: false, substantive_subtask: true, credit_reason: 'other_assignee', credited_to: r.assignee_id, credited_name: r.assignee_name ?? null };
    }
    if (substance && !parentTimed) {
      g.adopt.push(r);
      return { ...r, rollup: true, substantive_subtask: true, credit_reason: 'parent_untimed', credited_to: r.assignee_id ?? p.assignee_id ?? null };
    }
    g.dev.push(r);
    return { ...r, rollup: true };
  });

  const ww = opts.workweek;
  const standalone = new Map(); // parent key -> [substantive standalone sub-task keys]
  for (const r of out) {
    if (!r.substantive_subtask || r.rollup !== false) continue;
    if (!standalone.has(r.parent_key)) standalone.set(r.parent_key, []);
    standalone.get(r.parent_key).push(r.key);
  }
  for (let i = 0; i < out.length; i++) {
    const p = out[i];
    const g = kids.get(p.key);
    if (!g) continue;
    const next = { ...p };
    const rolled = [...g.dev, ...g.adopt];
    if (rolled.length) {
      // Union of the story's own dev intervals with every rolled-up child's;
      // a child without intervals contributes its dev_days as a lower bound.
      const ivs = unionIntervals([p, ...rolled].flatMap(devIntervals));
      const union = spanDays(ivs, ww);
      const floor = Math.max(0, ...rolled.filter((r) => !devIntervals(r).length).map((r) => r.dev_days || 0));
      next.child_dev_days = round2(Math.max(union, floor));
      next.git_authors = mergeAuthors([p.git_authors || [], ...rolled.map((r) => r.git_authors || [])]);
    }
    if (g.adopt.length) {
      next.timing_source_subtasks = g.adopt.map((r) => r.key);
      if (!((p.dev_days || 0) > 0)) next.dev_days = next.child_dev_days;
      if (next.first_commit_at == null) {
        const fc = g.adopt.map((r) => r.first_commit_at).filter((v) => v != null);
        if (fc.length) next.first_commit_at = Math.min(...fc);
      }
    }
    if (g.design.length) {
      const d = g.design.reduce((a, r) => a + (A.actualDays(r) ?? r.impl_days ?? 0), 0);
      next.design_days_eff = round2((p.design_days || 0) + d);
    }
    if (standalone.has(p.key)) next.substantive_subtasks = standalone.get(p.key);
    out[i] = next;
  }
  return out;
}

/**
 * Substantive sub-tasks surfaced per person, from classifySubtasks() output:
 *   { [person_id]: [{ key, parent_key, summary, dev_days, commits, reason, counted_in_parent }] }
 * reason: 'other_assignee' — real work under someone else's story; credited
 *           standalone, its days are NOT in the parent's dev_days/child_dev_days.
 *         'parent_untimed' — the story had no timing of its own and adopted
 *           this sub-task's (counted_in_parent=true: one unit, credited once).
 * Sorted by dev_days desc, then key.
 */
function creditedSubtasks(records) {
  const out = {};
  for (const r of records || []) {
    if (!r || !r.subtask || !r.substantive_subtask) continue;
    const who = r.credited_to ?? r.assignee_id ?? null;
    if (!who) continue;
    const reason = r.credit_reason || (r.rollup === false ? 'other_assignee' : 'parent_untimed');
    (out[who] ||= []).push({
      key: r.key,
      parent_key: r.parent_key ?? null,
      summary: r.summary ?? null,
      dev_days: round2(r.dev_days || 0),
      commits: commitCount(r),
      reason,
      counted_in_parent: r.rollup !== false,
    });
  }
  for (const list of Object.values(out)) list.sort((a, b) => b.dev_days - a.dev_days || String(a.key).localeCompare(String(b.key)));
  return out;
}

module.exports = { classifySubtasks, creditedSubtasks, unionIntervals, RE_DESIGN };
