//! Pure pieces of the chat trigger's answer to "which workflow did they mean,
//! and when will it start" — kept out of `chat.rs` so they test in isolation.
//!
//! Name matching: the owner types workflow names into Slack by hand, and the
//! exact `name = ? COLLATE NOCASE` lookup missed "PR Reviewer - Angular" for the
//! real "PR Reviewer — Angular" (em dash). The message then fell through to a
//! plain chat session with no reply at all. [`normalize_name`] folds the
//! differences a person can't see or doesn't type (dash variants, curly quotes,
//! spacing, case) on BOTH sides; [`resolve_name`] matches on that and, on a
//! miss, [`suggestions`] names the closest workflows so the reply can say
//! "Did you mean …".
//!
//! Run slots: the engine executes at most `OTTO_WF_MAX_PARALLEL_RUNS` (default
//! 2, the one reader is `run_queue::max_parallel_runs`) runs at once and
//! queues the rest FIFO. [`slot_outlook`] turns the live run counts into what
//! the acknowledgement may honestly promise.

/// Fold a workflow name for comparison: Unicode dashes (‐ ‑ ‒ – — ― −) → `-`,
/// curly quotes → straight, whitespace runs → one space, no space around a
/// dash (so "A - B", "A — B" and "A—B" agree), trimmed, lowercased.
pub fn normalize_name(name: &str) -> String {
    let mapped: String = name
        .chars()
        .map(|c| match c {
            '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2015}'
            | '\u{2212}' => '-',
            '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{2032}' => '\'',
            '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{2033}' => '"',
            c if c.is_whitespace() => ' ',
            c => c,
        })
        .collect();
    let collapsed = mapped.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed
        .replace(" -", "-")
        .replace("- ", "-")
        .to_lowercase()
}

/// How a typed name resolved against a workspace's workflows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameMatch {
    /// Exactly one workflow matches after normalisation: `(id, real name)`.
    One(String, String),
    /// Several workflows normalise to the same name — the user must type the
    /// exact one. Their real names.
    Ambiguous(Vec<String>),
    /// Nothing matches.
    None,
}

/// Match `typed` against `candidates` (`(id, name)`) by [`normalize_name`].
pub fn resolve_name(typed: &str, candidates: &[(String, String)]) -> NameMatch {
    let want = normalize_name(typed);
    if want.is_empty() {
        return NameMatch::None;
    }
    let hits: Vec<&(String, String)> = candidates
        .iter()
        .filter(|(_, name)| normalize_name(name) == want)
        .collect();
    match hits.as_slice() {
        [] => NameMatch::None,
        [(id, name)] => NameMatch::One(id.clone(), name.clone()),
        many => NameMatch::Ambiguous(many.iter().map(|(_, n)| n.clone()).collect()),
    }
}

/// Up to `limit` workflow names closest to `typed`, best first: a name that
/// starts with what was typed beats one that contains it, which beats the rest;
/// within a tier the smaller edit distance wins, then the input order (most
/// recently updated first). Names further than half their length away are
/// dropped — a suggestion that shares nothing with what was typed is noise.
pub fn suggestions(typed: &str, names: &[String], limit: usize) -> Vec<String> {
    let want = normalize_name(typed);
    let mut scored: Vec<(u8, usize, usize, &String)> = names
        .iter()
        .enumerate()
        .filter_map(|(i, name)| {
            let have = normalize_name(name);
            let dist = edit_distance(&want, &have);
            let tier = if have.starts_with(&want) {
                0
            } else if have.contains(&want) {
                1
            } else {
                2
            };
            let longest = want.chars().count().max(have.chars().count());
            (tier < 2 || dist * 2 <= longest).then_some((tier, dist, i, name))
        })
        .collect();
    scored.sort();
    scored
        .into_iter()
        .take(limit)
        .map(|(_, _, _, n)| n.clone())
        .collect()
}

/// Levenshtein distance over chars (names are short; O(n·m) is fine).
fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            let sub = prev[j] + usize::from(ca != *cb);
            cur[j + 1] = sub.min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}

/// The reply to an explicit trigger whose name matched nothing.
pub fn no_such_workflow_reply(typed: &str, names: &[String]) -> String {
    let close = suggestions(typed, names, 3);
    let typed = typed.trim();
    if names.is_empty() {
        return format!("No workflow named \"{typed}\" — this workspace has no workflows yet.");
    }
    if close.is_empty() {
        return format!("No workflow named \"{typed}\". Reply `help` for the list of workflows.");
    }
    format!(
        "No workflow named \"{typed}\". Did you mean: {}? Reply `help` for the full list.",
        close.join(", ")
    )
}

/// The reply when a typed name matches several workflows once normalised.
pub fn ambiguous_reply(typed: &str, names: &[String]) -> String {
    format!(
        "\"{}\" matches more than one workflow: {}. Type the exact name.",
        typed.trim(),
        names.join(", ")
    )
}

/// What a new run can expect when it is admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotOutlook {
    /// A run slot is free: it starts now.
    StartsNow,
    /// Every slot is taken: it waits behind `ahead` runs (the ones executing
    /// plus the ones already queued).
    Queued { ahead: usize },
}

/// `executing` = other runs holding a slot (live and not parked on an
/// approval, which hands its slot back); `queued` = other runs waiting for
/// one. A queue that is not empty means a new run lines up behind it even if
/// a slot looks free this instant — the gate is FIFO.
pub fn slot_outlook(executing: usize, queued: usize, cap: usize) -> SlotOutlook {
    if executing < cap && queued == 0 {
        SlotOutlook::StartsNow
    } else {
        SlotOutlook::Queued {
            ahead: executing + queued,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cands(names: &[&str]) -> Vec<(String, String)> {
        names
            .iter()
            .enumerate()
            .map(|(i, n)| (format!("id{i}"), n.to_string()))
            .collect()
    }

    #[test]
    fn normalize_folds_dashes_quotes_space_and_case() {
        assert_eq!(
            normalize_name("PR Reviewer — Angular"),
            "pr reviewer-angular"
        );
        assert_eq!(
            normalize_name("pr reviewer - angular"),
            "pr reviewer-angular"
        );
        assert_eq!(normalize_name("PR Reviewer–Angular"), "pr reviewer-angular");
        assert_eq!(
            normalize_name("  PR   Reviewer\t-\u{00A0}Angular "),
            "pr reviewer-angular"
        );
        assert_eq!(normalize_name("Owner\u{2019}s “flow”"), "owner's \"flow\"");
    }

    #[test]
    fn hyphen_typed_for_an_em_dash_resolves_to_the_real_workflow() {
        // The reported miss: an exact NOCASE match could never find this one.
        let c = cands(&["PR Reviewer", "PR Reviewer — Angular"]);
        assert_eq!(
            resolve_name("PR Reviewer - Angular", &c),
            NameMatch::One("id1".into(), "PR Reviewer — Angular".into())
        );
        assert_eq!(
            resolve_name("pr reviewer", &c),
            NameMatch::One("id0".into(), "PR Reviewer".into())
        );
        assert_eq!(resolve_name("Deploy", &c), NameMatch::None);
        assert_eq!(resolve_name("   ", &c), NameMatch::None);
    }

    #[test]
    fn names_that_normalise_alike_are_ambiguous_not_guessed() {
        let c = cands(&["A — B", "A - B", "C"]);
        assert_eq!(
            resolve_name("a-b", &c),
            NameMatch::Ambiguous(vec!["A — B".into(), "A - B".into()])
        );
    }

    #[test]
    fn suggestions_rank_prefix_then_substring_then_distance() {
        let names: Vec<String> = [
            "PR Reviewer — Angular",
            "PR Reviewer",
            "Deploy",
            "Code PR Reviewer",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(
            suggestions("PR Reviewr", &names, 3),
            vec![
                "PR Reviewer".to_string(),
                "Code PR Reviewer".into(),
                "PR Reviewer — Angular".into()
            ]
        );
        assert_eq!(suggestions("PR Rev", &names, 3)[0], "PR Reviewer");
        // Nothing close: no noise suggestions.
        assert!(suggestions("zzzzzzzz", &names, 3).is_empty());
    }

    #[test]
    fn no_such_workflow_reply_names_the_closest_three() {
        let names: Vec<String> = [
            "PR Reviewer",
            "PR Reviewer — Angular",
            "PR Reviewer — Go",
            "Deploy",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(
            no_such_workflow_reply("PR Reviewer Angualr", &names),
            "No workflow named \"PR Reviewer Angualr\". Did you mean: PR Reviewer — Angular, \
             PR Reviewer — Go, PR Reviewer? Reply `help` for the full list."
        );
        assert_eq!(
            no_such_workflow_reply("x", &[]),
            "No workflow named \"x\" — this workspace has no workflows yet."
        );
        assert_eq!(
            no_such_workflow_reply("qqqqqqqqqq", &names),
            "No workflow named \"qqqqqqqqqq\". Reply `help` for the list of workflows."
        );
    }

    #[test]
    fn edit_distance_basics() {
        assert_eq!(edit_distance("", ""), 0);
        assert_eq!(edit_distance("abc", ""), 3);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
    }

    #[test]
    fn slot_outlook_queues_when_full_or_when_a_queue_exists() {
        assert_eq!(slot_outlook(0, 0, 2), SlotOutlook::StartsNow);
        assert_eq!(slot_outlook(1, 0, 2), SlotOutlook::StartsNow);
        assert_eq!(slot_outlook(2, 0, 2), SlotOutlook::Queued { ahead: 2 });
        // FIFO: a waiting run goes first even if a slot looks free.
        assert_eq!(slot_outlook(1, 1, 2), SlotOutlook::Queued { ahead: 2 });
        assert_eq!(slot_outlook(2, 3, 2), SlotOutlook::Queued { ahead: 5 });
    }
}
