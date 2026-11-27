//! Goal progress across one or more check steps (and loops): keying every
//! criterion by `(step_id, id)`, building bounded per-round history from loop
//! checkpoints (or settled loop history), and formatting the headline and chat
//! status lines from one shared source of truth. Pure.

use std::collections::{BTreeMap, BTreeSet};

use otto_core::workflows::{
    Criterion, CriterionPoint, CriterionProgress, GoalCriterion, GoalProgress, HoldKind,
    NodeRunState, NodeStatus, WorkflowGraph, WorkflowNode, WorkflowRun,
};
use serde_json::Value;

use crate::criteria::LoopBudget;

/// Bounded per-criterion history (a loop has at most 50 rounds).
pub const MAX_HISTORY: usize = 50;
/// Bounded criteria list on `RunOverview.goal` (matches `validate_spec`'s 40).
pub const MAX_GOAL_CRITERIA: usize = 40;
/// Bounded per-round lines shown directly on a chat status card before folding.
pub const MAX_CHAT_ROUNDS: usize = 8;

/// Whether the workflow graph contains any `check` step (inside a loop or at
/// top level) — used by callers to decide whether to load checkpoints before
/// computing [`crate::overview::run_overview`].
pub fn has_goal_checks(graph: &WorkflowGraph) -> bool {
    graph.nodes.iter().any(|n| {
        n.kind == "check"
            || (n.kind == "loop"
                && n.params["steps"]
                    .as_array()
                    .is_some_and(|steps| steps.iter().any(|s| s["kind"] == "check")))
    })
}

struct CheckSlot<'a> {
    step_id: String,
    loop_id: Option<&'a str>,
    step_index: Option<usize>,
    inner_name: String,
    step_name: String,
    checks_in_loop: usize,
    criteria: Vec<Criterion>,
}

/// Compute `RunOverview.goal` across every loop with a `check` step (plus any
/// top-level `check` step when the graph has a goal loop). Keyed by
/// `(step_id, id)` so two check steps with identical criterion ids stay
/// distinct.
pub fn compute(
    run: &WorkflowRun,
    graph: &WorkflowGraph,
    prev: Option<&GoalProgress>,
) -> Option<GoalProgress> {
    let goal_loops: Vec<&WorkflowNode> = graph
        .nodes
        .iter()
        .filter(|n| {
            n.kind == "loop"
                && n.params["steps"]
                    .as_array()
                    .is_some_and(|steps| steps.iter().any(|s| s["kind"] == "check"))
        })
        .collect();
    if goal_loops.is_empty() {
        return None;
    }

    let loop_states: Vec<(&WorkflowNode, &NodeRunState)> = goal_loops
        .iter()
        .filter_map(|&lp| {
            run.nodes
                .iter()
                .find(|s| s.node_id == lp.id)
                .map(|st| (lp, st))
        })
        .collect();
    if loop_states.is_empty() {
        return None;
    }

    let slots = collect_check_slots(graph, &goal_loops);
    let mut criteria_out: Vec<GoalCriterion> = Vec::new();
    let mut any_check_output = false;
    let mut all_total: u32 = 0;
    let mut all_met: u32 = 0;
    let mut slot_verdicts: Vec<Option<&'static str>> = Vec::with_capacity(slots.len());

    for slot in &slots {
        let rounds = collect_slot_rounds(run, slot);
        if !rounds.is_empty() {
            any_check_output = true;
        }
        let awaiting_id = slot
            .loop_id
            .or(Some(slot.step_id.as_str()))
            .and_then(|nid| {
                run.nodes
                    .iter()
                    .find(|s| s.node_id == nid)
                    .and_then(|s| s.attention.as_ref())
                    .and_then(|a| a.criterion_id.as_deref())
            });
        let latest = rounds.last().map(|(_, v)| v);
        slot_verdicts.push(
            latest
                .and_then(|v| v.get("verdict").and_then(Value::as_str))
                .and_then(|v| match v {
                    "achieved" => Some("achieved"),
                    "continue" => Some("continue"),
                    "blocked" => Some("blocked"),
                    _ => None,
                }),
        );
        let defs = slot_criteria_defs(slot, latest);
        for (id, text) in defs {
            let awaiting = awaiting_id == Some(id.as_str());
            let mut last_row: Option<&Value> = None;
            let keep_row = criteria_out.len() < MAX_GOAL_CRITERIA;
            let mut history = Vec::new();
            if keep_row {
                for (iter, check_val) in &rounds {
                    if let Some(row) = find_criterion_row(check_val, &id) {
                        let met = row.get("met").and_then(Value::as_bool).unwrap_or(false);
                        let progress = parse_progress(row.get("progress"));
                        history.push(CriterionPoint {
                            iteration: *iter,
                            met,
                            value: progress.as_ref().and_then(|p| p.value),
                        });
                        last_row = Some(row);
                    }
                }
                if history.len() > MAX_HISTORY {
                    let drop = history.len() - MAX_HISTORY;
                    history.drain(..drop);
                }
            } else {
                // Past the overview row cap: still count met/total from the
                // latest check so clipping never reports an unmet goal as met.
                last_row = latest.and_then(|v| find_criterion_row(v, &id));
            }
            let met = if awaiting {
                None
            } else {
                last_row.and_then(|r| r.get("met").and_then(Value::as_bool))
            };
            all_total += 1;
            if !awaiting && met == Some(true) {
                all_met += 1;
            }
            if keep_row {
                let progress = last_row
                    .and_then(|r| parse_progress(r.get("progress")))
                    .map(compact_progress);
                criteria_out.push(GoalCriterion {
                    step_id: slot.step_id.clone(),
                    step_name: slot.step_name.clone(),
                    id,
                    text: clip(&text, 120),
                    met,
                    awaiting,
                    progress,
                    history,
                });
            }
        }
    }

    let mut awaiting: Vec<String> = Vec::new();
    for (_, st) in &loop_states {
        if let Some(cid) = st.attention.as_ref().and_then(|a| a.criterion_id.clone()) {
            if !awaiting.contains(&cid) {
                awaiting.push(cid);
            }
        }
    }

    let effective_hold = crate::overview::effective_hold(run.status, run.hold.as_ref());
    // When a specific goal loop is held, report that held loop's iteration,
    // active time, and limits so "Extend and resume…" matches the server's
    // `hold::prepare_resume` target rather than a completed sibling loop.
    let held_loop = effective_hold.and_then(|h| {
        h.node_ids
            .iter()
            .find_map(|id| loop_states.iter().find(|(lp, _)| &lp.id == id).copied())
    });

    let mut running_iter: Option<(u32, u32)> = None;
    let mut best_iter: Option<(u32, u32)> = None;
    let mut active_ms: u64 = 0;
    let mut max_active_ms: Option<u64> = None;
    let mut any_settled_output = false;

    if let Some((lp, st)) = held_loop {
        let limits = run.limits.as_ref().and_then(|l| l.loops.get(&lp.id));
        let budget = LoopBudget::new(&lp.params, limits);
        let hold_iter = effective_hold.and_then(|h| h.iteration).filter(|&i| i > 0);
        let from_out = st
            .output
            .as_ref()
            .and_then(|o| o.get("iterations"))
            .and_then(Value::as_u64)
            .map(|i| i as u32)
            .filter(|&i| i > 0);
        let from_logs = loop_iteration(st).map(|(i, _)| i);
        let from_cps = run
            .checkpoints
            .iter()
            .filter(|cp| cp.loop_id == lp.id)
            .map(|cp| cp.iteration as u32)
            .max()
            .filter(|&i| i > 0);
        let it = hold_iter
            .or(from_out)
            .or(from_logs)
            .or(from_cps)
            .unwrap_or(0);
        let max_it = st
            .output
            .as_ref()
            .and_then(|o| o.get("max_iterations"))
            .and_then(Value::as_u64)
            .map(|i| i as u32)
            .filter(|&m| m > 0 && limits.is_none())
            .unwrap_or(budget.max_iterations);
        if it > 0 || max_it > 0 {
            best_iter = Some((it, max_it));
        }
        let banked: u64 = run
            .checkpoints
            .iter()
            .filter(|cp| cp.loop_id == lp.id && cp.node_id != lp.id)
            .map(|cp| cp.active_ms)
            .sum();
        active_ms = if banked > 0 {
            banked
        } else {
            st.output
                .as_ref()
                .and_then(|o| o.get("active_ms"))
                .and_then(Value::as_u64)
                .unwrap_or(0)
        };
        max_active_ms = budget.max_active_s.map(|s| s * 1000).or_else(|| {
            st.output
                .as_ref()
                .and_then(|o| o.get("max_active_s"))
                .and_then(Value::as_u64)
                .map(|s| s * 1000)
        });
        any_settled_output = st.output.is_some() || hold_iter.is_some();
    } else {
        for (lp, st) in &loop_states {
            let limits = run.limits.as_ref().and_then(|l| l.loops.get(&lp.id));
            let budget = LoopBudget::new(&lp.params, limits);
            if let Some(out) = st.output.as_ref() {
                any_settled_output = true;
                let num = |k: &str| out.get(k).and_then(Value::as_u64);
                let it = num("iterations").unwrap_or(0) as u32;
                let max_it = num("max_iterations").unwrap_or(0) as u32;
                let max_it = if max_it > 0 || loop_states.len() == 1 {
                    max_it
                } else {
                    budget.max_iterations
                };
                best_iter = pick_higher_iter(best_iter, (it, max_it));
                active_ms = active_ms.saturating_add(num("active_ms").unwrap_or(0));
                if let Some(ms) = num("max_active_s").map(|s| s * 1000) {
                    max_active_ms = Some(max_active_ms.map_or(ms, |cur| cur.max(ms)));
                }
            } else {
                let from_logs = loop_iteration(st);
                let from_cps = run
                    .checkpoints
                    .iter()
                    .filter(|cp| cp.loop_id == lp.id)
                    .map(|cp| cp.iteration as u32)
                    .max()
                    .filter(|&i| i > 0)
                    .map(|i| (i, budget.max_iterations));
                if let Some(pair) = from_logs.or(from_cps) {
                    best_iter = pick_higher_iter(best_iter, pair);
                    if st.status == NodeStatus::Running {
                        running_iter = pick_higher_iter(running_iter, pair);
                    }
                }
                let banked: u64 = run
                    .checkpoints
                    .iter()
                    .filter(|cp| cp.loop_id == lp.id && cp.node_id != lp.id)
                    .map(|cp| cp.active_ms)
                    .sum();
                active_ms = active_ms.saturating_add(banked);
                if let Some(ms) = budget.max_active_s.map(|s| s * 1000) {
                    max_active_ms = Some(max_active_ms.map_or(ms, |cur| cur.max(ms)));
                }
            }
        }
    }

    let (iteration, max_iterations) = match running_iter.or(best_iter) {
        Some(pair) => pair,
        None => {
            let p = prev?;
            (p.iteration, p.max_iterations)
        }
    };

    if !any_settled_output && max_iterations == 0 {
        return None;
    }

    // If a live progress update did not load checkpoints, carry over richer
    // per-criterion state from the previous overview rather than wiping it.
    let mut carried_from_prev = false;
    if !any_check_output {
        if let Some(p) = prev.filter(|p| !p.criteria.is_empty()) {
            criteria_out = p.criteria.clone();
            carried_from_prev = true;
            any_check_output = criteria_out
                .iter()
                .any(|c| c.met.is_some() || c.progress.is_some() || !c.history.is_empty());
        }
    }

    let mut settled_verdicts: Vec<Option<&'static str>> = Vec::new();
    let (criteria_met, criteria_total) = if any_check_output && all_total > 0 && !carried_from_prev
    {
        (all_met, all_total)
    } else if any_check_output && carried_from_prev {
        if let Some(p) = prev.filter(|p| p.criteria_total > 0) {
            (p.criteria_met, p.criteria_total)
        } else {
            let met = criteria_out
                .iter()
                .filter(|c| !c.awaiting && c.met == Some(true))
                .count() as u32;
            (met, criteria_out.len() as u32)
        }
    } else if any_settled_output && all_total == 0 {
        // Minimal settled loop whose check output only carried `{all_met, met, total}`.
        let (mut met, mut total) = (0u32, 0u32);
        for (_, st) in &loop_states {
            if let Some(last_check) = st
                .output
                .as_ref()
                .and_then(|o| o["history"].as_array())
                .and_then(|h| h.last())
                .and_then(|i| i["steps"].as_object())
                .and_then(|s| s.values().find(|v| v.get("all_met").is_some()))
            {
                met += last_check["met"].as_u64().unwrap_or(0) as u32;
                total += last_check["total"].as_u64().unwrap_or(0) as u32;
                let v = last_check
                    .get("verdict")
                    .and_then(Value::as_str)
                    .and_then(|s| match s {
                        "achieved" => Some("achieved"),
                        "continue" => Some("continue"),
                        "blocked" => Some("blocked"),
                        _ => None,
                    });
                settled_verdicts.push(v);
            }
        }
        (met, total)
    } else {
        // Running loop before any checkpoint output (or log-only tests): read the
        // relayed check log line, else keep the previous overview's counts.
        let mut from_logs = None;
        let mut log_met = 0u32;
        let mut log_total = 0u32;
        for (_, st) in &loop_states {
            if let Some((m, t)) = latest_check_log_counts(st) {
                log_met += m;
                log_total += t;
                from_logs = Some((log_met, log_total));
            }
        }
        match from_logs {
            Some(counts) => counts,
            None => {
                let total = if all_total > 0 {
                    all_total
                } else {
                    prev.map_or(0, |p| p.criteria_total)
                };
                let met = prev.map_or(0, |p| p.criteria_met).min(total);
                (met, total)
            }
        }
    };

    let verdicts = if !slot_verdicts.is_empty() && slot_verdicts.iter().any(Option::is_some) {
        &slot_verdicts
    } else {
        &settled_verdicts
    };
    let verdict = if effective_hold.is_some_and(|h| h.kind == HoldKind::Blocked)
        || verdicts.contains(&Some("blocked"))
    {
        Some("blocked".to_string())
    } else if !verdicts.is_empty()
        && verdicts.iter().all(|v| *v == Some("achieved"))
        && criteria_total > 0
        && criteria_met == criteria_total
    {
        Some("achieved".to_string())
    } else if verdicts.contains(&Some("continue")) {
        Some("continue".to_string())
    } else if carried_from_prev {
        prev.and_then(|p| p.verdict.clone())
    } else {
        None
    };

    Some(GoalProgress {
        iteration,
        max_iterations,
        criteria_met,
        criteria_total,
        active_ms: if active_ms > 0 || held_loop.is_some() {
            active_ms
        } else {
            prev.map_or(0, |p| p.active_ms)
        },
        max_active_ms: max_active_ms.or_else(|| prev.and_then(|p| p.max_active_ms)),
        awaiting,
        verdict,
        criteria: criteria_out,
    })
}

fn collect_check_slots<'a>(
    graph: &'a WorkflowGraph,
    goal_loops: &[&'a WorkflowNode],
) -> Vec<CheckSlot<'a>> {
    let multi_loops = goal_loops.len() > 1;
    let mut slots = Vec::new();
    for node in &graph.nodes {
        if node.kind == "loop" {
            let steps = match node.params["steps"].as_array() {
                Some(s) => s,
                None => continue,
            };
            let checks_in_loop = steps.iter().filter(|s| s["kind"] == "check").count();
            if checks_in_loop == 0 {
                continue;
            }
            let loop_name = display_name(&node.name, &node.id);
            for (k, step) in steps.iter().enumerate() {
                if step["kind"] != "check" {
                    continue;
                }
                let inner_name = step
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("step{k}"));
                let step_name =
                    choose_step_label(&loop_name, &inner_name, checks_in_loop, multi_loops);
                let criteria: Vec<Criterion> = step
                    .pointer("/params/criteria")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();
                slots.push(CheckSlot {
                    step_id: format!("{}.{k}", node.id),
                    loop_id: Some(&node.id),
                    step_index: Some(k),
                    inner_name,
                    step_name,
                    checks_in_loop,
                    criteria,
                });
            }
        } else if node.kind == "check" {
            let criteria: Vec<Criterion> = node
                .params
                .get("criteria")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            slots.push(CheckSlot {
                step_id: node.id.clone(),
                loop_id: None,
                step_index: None,
                inner_name: display_name(&node.name, &node.id),
                step_name: display_name(&node.name, &node.id),
                checks_in_loop: 1,
                criteria,
            });
        }
    }
    slots
}

fn collect_slot_rounds(run: &WorkflowRun, slot: &CheckSlot<'_>) -> Vec<(u32, Value)> {
    if let (Some(loop_id), Some(step_index)) = (slot.loop_id, slot.step_index) {
        let mut by_iter: BTreeMap<u32, Value> = BTreeMap::new();
        for cp in &run.checkpoints {
            if cp.kind == "check"
                && cp.status == NodeStatus::Success
                && cp.loop_id == loop_id
                && cp.step_index == step_index
            {
                if let Some(out) = cp.output.as_ref().filter(|v| is_check_output(v)) {
                    by_iter.insert(cp.iteration as u32, out.clone());
                }
            }
        }
        if !by_iter.is_empty() {
            return bounded_rounds(by_iter);
        }
        // Fallback: settled loop's `output.history`.
        if let Some(history) = run
            .nodes
            .iter()
            .find(|n| n.node_id == loop_id)
            .and_then(|n| n.output.as_ref())
            .and_then(|o| o.get("history"))
            .and_then(Value::as_array)
        {
            for (idx, entry) in history.iter().enumerate() {
                let iter = entry
                    .get("iteration")
                    .and_then(Value::as_u64)
                    .unwrap_or((idx + 1) as u64) as u32;
                let Some(steps) = entry.get("steps").and_then(Value::as_object) else {
                    continue;
                };
                let hit = steps
                    .get(&slot.inner_name)
                    .filter(|v| is_check_output(v))
                    .or_else(|| {
                        (slot.checks_in_loop == 1)
                            .then(|| steps.values().find(|v| is_check_output(v)))
                            .flatten()
                    });
                if let Some(out) = hit {
                    by_iter.insert(iter, out.clone());
                }
            }
            if !by_iter.is_empty() {
                return bounded_rounds(by_iter);
            }
        }
        return Vec::new();
    }
    // Top-level check node.
    run.nodes
        .iter()
        .find(|n| n.node_id == slot.step_id && n.status == NodeStatus::Success)
        .and_then(|n| n.output.as_ref())
        .filter(|v| is_check_output(v))
        .map(|out| vec![(1, out.clone())])
        .unwrap_or_default()
}

fn bounded_rounds(by_iter: BTreeMap<u32, Value>) -> Vec<(u32, Value)> {
    let mut rounds: Vec<(u32, Value)> = by_iter.into_iter().collect();
    if rounds.len() > MAX_HISTORY {
        let drop = rounds.len() - MAX_HISTORY;
        rounds.drain(..drop);
    }
    rounds
}

fn slot_criteria_defs(slot: &CheckSlot<'_>, latest: Option<&Value>) -> Vec<(String, String)> {
    if !slot.criteria.is_empty() {
        return slot
            .criteria
            .iter()
            .map(|c| (c.id.clone(), c.text.clone()))
            .collect();
    }
    latest
        .and_then(|v| v.get("criteria"))
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|r| {
                    let id = r.get("id")?.as_str()?.to_string();
                    let text = r
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or(&id)
                        .to_string();
                    Some((id, text))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn is_check_output(v: &Value) -> bool {
    v.get("criteria").is_some_and(Value::is_array) && v.get("verdict").is_some()
}

fn find_criterion_row<'a>(check_val: &'a Value, id: &str) -> Option<&'a Value> {
    check_val
        .get("criteria")
        .and_then(Value::as_array)?
        .iter()
        .find(|r| r.get("id").and_then(Value::as_str) == Some(id))
}

fn parse_progress(v: Option<&Value>) -> Option<CriterionProgress> {
    let p: CriterionProgress = serde_json::from_value(v?.clone()).ok()?;
    (!p.is_empty()).then_some(p)
}

/// Compact a criterion's progress so `RunOverview` stays small over WS.
fn compact_progress(mut p: CriterionProgress) -> CriterionProgress {
    p.label = p.label.map(|l| clip(&l, 120));
    p.unit = p.unit.map(|u| clip(&u, 24));
    let mut vals = BTreeMap::new();
    for (k, v) in p.values.into_iter().take(8) {
        let small = match v {
            Value::String(s) => Value::String(clip(&s, 120)),
            Value::Bool(_) | Value::Number(_) | Value::Null => v,
            other => Value::String(clip(&other.to_string(), 120)),
        };
        vals.insert(clip(&k, 64), small);
    }
    p.values = vals;
    p
}

fn pick_higher_iter(cur: Option<(u32, u32)>, next: (u32, u32)) -> Option<(u32, u32)> {
    match cur {
        None => Some(next),
        Some((ci, cm)) if next.0 > ci || (next.0 == ci && next.1 > cm) => Some(next),
        Some(prev) => Some(prev),
    }
}

fn latest_check_log_counts(st: &NodeRunState) -> Option<(u32, u32)> {
    st.logs.iter().rev().find_map(|l| {
        let (_, said) = l.strip_prefix("  [")?.split_once("] ")?;
        let (met, total) = said.strip_suffix(" criteria met")?.split_once(" of ")?;
        Some((met.parse().ok()?, total.parse().ok()?))
    })
}

/// The iteration a running loop is in: its newest "🔁 iteration i/n" log line.
pub(crate) fn loop_iteration(n: &NodeRunState) -> Option<(u32, u32)> {
    n.logs.iter().rev().find_map(|l| {
        let (i, of) = l.strip_prefix("🔁 iteration ")?.split_once('/')?;
        Some((i.parse().ok()?, of.trim().parse().ok()?))
    })
}

fn display_name(name: &str, fallback: &str) -> String {
    let t = name.trim();
    if t.is_empty() {
        fallback.to_string()
    } else {
        t.to_string()
    }
}

fn is_generic_check_name(s: &str) -> bool {
    let lower = s.trim().to_ascii_lowercase();
    lower.is_empty() || lower == "check" || lower == "check criteria"
}

fn is_generic_loop_name(s: &str) -> bool {
    let lower = s.trim().to_ascii_lowercase();
    lower.is_empty()
        || lower == "loop"
        || lower == "work toward the goal"
        || lower == "review → fix until passing"
        || lower == "iterate"
}

fn choose_step_label(
    loop_name: &str,
    inner_name: &str,
    checks_in_loop: usize,
    multi_loops: bool,
) -> String {
    if checks_in_loop > 1 {
        if multi_loops {
            format!("{loop_name} · {inner_name}")
        } else {
            inner_name.to_string()
        }
    } else if !is_generic_check_name(inner_name) && is_generic_loop_name(loop_name) {
        inner_name.to_string()
    } else {
        loop_name.to_string()
    }
}

/// Format a finite number cleanly: integers without decimals (`72`), fractions
/// to one decimal place (`9.5`).
pub fn format_num(v: f64) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    if (v - v.round()).abs() < 1e-9 {
        format!("{}", v.round() as i64)
    } else {
        let s = format!("{v:.1}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// Format `value / target` (with optional unit), e.g. `"72 / 95"` or `"64% / 80%"`.
pub fn format_value_target(value: f64, target: Option<f64>, unit: Option<&str>) -> String {
    let v = format_num(value);
    let unit = unit.map(str::trim).filter(|u| !u.is_empty());
    match (target, unit) {
        (Some(t), None) => format!("{v} / {}", format_num(t)),
        (Some(t), Some("%")) => format!("{v}% / {}%", format_num(t)),
        (Some(t), Some(u)) => format!("{v} / {} {u}", format_num(t)),
        (None, Some("%")) => format!("{v}%"),
        (None, Some(u)) => format!("{v} {u}"),
        (None, None) => v,
    }
}

/// Format a criterion's current numeric progress ratio, if it has one.
pub fn format_progress_ratio(p: &CriterionProgress) -> Option<String> {
    let val = p.value?;
    Some(format_value_target(val, p.target, p.unit.as_deref()))
}

/// Short label for one criterion in compact single-line summaries (headline and
/// chat status): uses the step/loop name when multiple check steps exist, or
/// the criterion id when one step has multiple criteria.
pub fn short_label(criteria: &[GoalCriterion], c: &GoalCriterion) -> String {
    let distinct_steps: BTreeSet<&str> = criteria.iter().map(|x| x.step_id.as_str()).collect();
    let in_step = criteria.iter().filter(|x| x.step_id == c.step_id).count();
    if distinct_steps.len() > 1 {
        let base = if c.step_name.trim().is_empty() {
            c.step_id.as_str()
        } else {
            c.step_name.trim()
        };
        if in_step > 1 {
            format!("{base}/{}", c.id)
        } else {
            base.to_string()
        }
    } else if in_step > 1 {
        c.id.clone()
    } else if !is_generic_loop_name(&c.step_name) && !is_generic_check_name(&c.step_name) {
        c.step_name.trim().to_string()
    } else {
        String::new()
    }
}

/// Compact per-criterion numeric status entries (e.g. `["UI 72 / 95", "server 81 / 95"]`).
pub fn numeric_parts(goal: &GoalProgress) -> Vec<String> {
    let mut parts = Vec::new();
    for c in &goal.criteria {
        let Some(ratio) = c.progress.as_ref().and_then(format_progress_ratio) else {
            continue;
        };
        let label = short_label(&goal.criteria, c);
        if label.is_empty() {
            parts.push(ratio);
        } else {
            parts.push(format!("{label} {ratio}"));
        }
    }
    parts
}

/// Headline for a running goal run: `"Round 3 of 5 — UI 72 / 95, server 81 / 95"`
/// when any criterion reports numeric progress, else the classic
/// `"Iteration 3 of 5 — 1 of 3 criteria met"`.
pub fn headline_running(goal: &GoalProgress) -> String {
    let parts = numeric_parts(goal);
    if !parts.is_empty() {
        return format!(
            "Round {} of {} — {}",
            goal.iteration,
            goal.max_iterations,
            parts.join(", ")
        );
    }
    format!(
        "Iteration {} of {} — {} of {} criteria met",
        goal.iteration, goal.max_iterations, goal.criteria_met, goal.criteria_total
    )
}

/// The live chat card's goal status line (matches the run page headline's numbers).
/// Says `"Goal reached"` only when the completion verdict is `"achieved"` (and
/// `allow_reached` is true — not blocked/held); uses `"All criteria met"` when
/// all criteria passed without an achieved completion verdict (count-only or
/// blocked completion review).
pub fn chat_goal_line(goal: &GoalProgress) -> Option<String> {
    chat_goal_line_for(goal, true)
}

pub fn chat_goal_line_for(goal: &GoalProgress, allow_reached: bool) -> Option<String> {
    if goal.criteria_total == 0 && goal.criteria.is_empty() {
        return None;
    }
    let parts = numeric_parts(goal);
    let all_met = goal.criteria_total > 0 && goal.criteria_met == goal.criteria_total;
    let reached = allow_reached && all_met && goal.verdict.as_deref() == Some("achieved");
    if reached {
        if !parts.is_empty() {
            return Some(format!("🎯 Goal reached — {}", parts.join(", ")));
        }
        return Some(format!(
            "🎯 Goal reached — {} of {} criteria met",
            goal.criteria_met, goal.criteria_total
        ));
    }
    if all_met {
        if !parts.is_empty() {
            return Some(format!("🎯 All criteria met — {}", parts.join(", ")));
        }
        return Some(format!(
            "🎯 All criteria met — {} of {} criteria met",
            goal.criteria_met, goal.criteria_total
        ));
    }
    if !parts.is_empty() {
        return Some(if goal.iteration > 0 && goal.max_iterations > 0 {
            format!(
                "🎯 Round {} of {} — {}",
                goal.iteration,
                goal.max_iterations,
                parts.join(", ")
            )
        } else {
            format!("🎯 Goal — {}", parts.join(", "))
        });
    }
    Some(if goal.iteration > 0 && goal.max_iterations > 0 {
        format!(
            "🎯 Round {} of {} — {} of {} criteria met",
            goal.iteration, goal.max_iterations, goal.criteria_met, goal.criteria_total
        )
    } else {
        format!(
            "🎯 Criteria {} of {} met",
            goal.criteria_met, goal.criteria_total
        )
    })
}

/// Compact count-only goal line used as a fallback when a long numeric list
/// exceeds a chat message budget (e.g. Telegram's 4,096-character limit).
pub fn compact_chat_goal_line(goal: &GoalProgress, allow_reached: bool) -> Option<String> {
    if goal.criteria_total == 0 && goal.criteria.is_empty() {
        return None;
    }
    let all_met = goal.criteria_total > 0 && goal.criteria_met == goal.criteria_total;
    let reached = allow_reached && all_met && goal.verdict.as_deref() == Some("achieved");
    if reached {
        Some(format!(
            "🎯 Goal reached — {} of {} criteria met",
            goal.criteria_met, goal.criteria_total
        ))
    } else if all_met {
        Some(format!(
            "🎯 All criteria met — {} of {} criteria met",
            goal.criteria_met, goal.criteria_total
        ))
    } else if goal.iteration > 0 && goal.max_iterations > 0 {
        Some(format!(
            "🎯 Round {} of {} — {} of {} criteria met",
            goal.iteration, goal.max_iterations, goal.criteria_met, goal.criteria_total
        ))
    } else {
        Some(format!(
            "🎯 Criteria {} of {} met",
            goal.criteria_met, goal.criteria_total
        ))
    }
}

/// One line per completed round for the chat status card, e.g.:
/// `"Round 3: UI 72 / 95 (was 58), server 81 / 95 (was 80)"`.
pub fn chat_round_lines(goal: &GoalProgress) -> Vec<String> {
    let iterations: BTreeSet<u32> = goal
        .criteria
        .iter()
        .flat_map(|c| c.history.iter().map(|p| p.iteration))
        .collect();
    if iterations.is_empty() {
        return Vec::new();
    }
    let mut lines = Vec::new();
    for iter in iterations {
        let mut entries = Vec::new();
        for c in &goal.criteria {
            let Some(pos) = c.history.iter().position(|p| p.iteration == iter) else {
                continue;
            };
            let pt = &c.history[pos];
            let prev_pt = pos.checked_sub(1).and_then(|i| c.history.get(i));
            let label = short_label(&goal.criteria, c);
            if let Some(val) = pt.value {
                let target = c.progress.as_ref().and_then(|p| p.target);
                let unit = c.progress.as_ref().and_then(|p| p.unit.as_deref());
                let ratio = format_value_target(val, target, unit);
                let was = prev_pt
                    .and_then(|p| p.value)
                    .map(|pv| format!(" (was {})", format_num(pv)))
                    .unwrap_or_default();
                if label.is_empty() {
                    entries.push(format!("{ratio}{was}"));
                } else {
                    entries.push(format!("{label} {ratio}{was}"));
                }
            } else {
                let word = if pt.met { "met" } else { "not met" };
                if label.is_empty() {
                    entries.push(word.to_string());
                } else {
                    entries.push(format!("{label} {word}"));
                }
            }
        }
        if !entries.is_empty() {
            lines.push(format!("Round {iter}: {}", entries.join(", ")));
        }
    }
    if lines.len() > MAX_CHAT_ROUNDS {
        let omitted = lines.len() - MAX_CHAT_ROUNDS;
        let mut trimmed = vec![format!("… {omitted} earlier rounds")];
        trimmed.extend(lines.drain(omitted..));
        return trimmed;
    }
    lines
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
#[path = "goal_progress_tests.rs"]
mod tests;
