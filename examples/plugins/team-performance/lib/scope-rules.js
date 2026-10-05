// Scope rules shared by every view: which records count as DELIVERED scope,
// who a record is credited to, and the configured weekend. One predicate so
// analytics, metrics, flow and the server never disagree on "what was
// delivered". Pure, zero-dep.
//
// Delivered scope = a real unit of new work:
//   - not a rework ticket (rework_of / scope_excluded — its estimate is not new
//     scope, its time is charged back to the origin),
//   - not excluded (lead override, stale/outlier timing without a manual time,
//     a rolled-up sub-task),
//   - not an epic or feature container (their children carry the work),
//   - not a checklist sub-task. A SUBSTANTIVE sub-task (real work, kept
//     standalone: substantive_subtask && rollup !== true) DOES count, credited
//     to `credited_to` (lib/subtasks.js).
'use strict';

const DEFAULT_WORKWEEK = [1, 2, 3, 4, 5];

/** Weekend day numbers (0=Sun … 6=Sat) — the complement of `workweek`. */
function weekendOf(workweek) {
  const ww = new Set(Array.isArray(workweek) && workweek.length ? workweek : DEFAULT_WORKWEEK);
  return [0, 1, 2, 3, 4, 5, 6].filter((d) => !ww.has(d));
}

const typeOf = (r) => String((r && r.type) || '').toLowerCase();
const isStale = (r) => (r.flags || []).includes('stale_timing') || (r.flags || []).includes('zero_time');

/**
 * Excluded from every median/baseline/throughput: an epic, a lead-excluded
 * story (excluded_override — hard, wins over everything), a sub-task rolled up
 * into its parent (counting both would double the same work), or stale /
 * outlier timing. A manual time override cures stale/outlier.
 */
const isExcluded = (r) =>
  typeOf(r) === 'epic' || r.excluded_override === true || r.rollup === true || (r.manual_days == null && (r.outlier === true || isStale(r)));

/** Epic / feature containers (never delivered scope themselves). */
const isContainer = (r) => typeOf(r) === 'epic' || typeOf(r) === 'feature' || r.feature === true;

/** A sub-task that carries real, standalone work (not rolled into its parent). */
const isSubstantiveSubtask = (r) => Boolean(r && r.subtask && r.substantive_subtask && r.rollup !== true);

/** True when the record counts as delivered (new) scope. */
function isDeliveredScope(r) {
  if (!r) return false;
  if (r.rework_of || r.scope_excluded) return false;
  if (r.excluded_override === true || isExcluded(r)) return false;
  if (isContainer(r)) return false;
  if (r.subtask && !isSubstantiveSubtask(r)) return false;
  return true;
}

/** Who a delivered record is credited to (substantive sub-task → credited_to). */
function creditedOwner(r) {
  if (!r) return null;
  if (isSubstantiveSubtask(r)) return r.credited_to ?? r.assignee_id ?? null;
  return r.assignee_id ?? null;
}

module.exports = { isDeliveredScope, weekendOf, isExcluded, isStale, isContainer, isSubstantiveSubtask, creditedOwner, DEFAULT_WORKWEEK };
