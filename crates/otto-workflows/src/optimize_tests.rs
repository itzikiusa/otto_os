use super::*;
use chrono::{Duration, TimeZone, Utc};
use otto_core::workflows::{NodeRunState, PhaseMark};

fn wf(nodes: &[(&str, &str, &str, Value)], edges: &[(&str, &str, Option<&str>)]) -> Workflow {
    serde_json::from_value(json!({
        "id": "wf1", "workspace_id": "ws", "name": "Shape", "description": "",
        "created_by": "u", "created_at": "2026-11-01T00:00:00Z", "updated_at": "2026-11-01T00:00:00Z",
        "version": 7,
        "graph": {
            "nodes": nodes.iter().map(|(id, kind, name, params)| json!({"id": id, "kind": kind, "name": name, "params": params})).collect::<Vec<_>>(),
            "edges": edges.iter().map(|(s, t, c)| json!({"id": format!("{s}-{t}"), "source": s, "target": t, "condition": c})).collect::<Vec<_>>(),
        }
    }))
    .unwrap()
}

const CHECKOUT: &str = "Read run-brief.md and repos.json in the run context, plus the step-1 *.output.json (the trigger input). For each entry: fetch, git checkout --detach the PR head, compute the merge-base with the destination and record base_branch. UPDATE repos.json before you finish. Use curl against the forge only when the PR number is missing. Steps: step 4 verify HEAD, step 6 paste the table.";
const CONTEXT: &str = "Read repos.json and the jira-<KEY>.md file in the run context. Write a short context brief: the flows this change touches.";
const CROSS: &str = "Read repos.json in the run context. If the run declares only one repo, write a one-line note that the check was skipped and stop. Otherwise check the interactions between the repos, using the context brief (step4) as a map.";
const GATES: &str = "Read repos.json in the run context. For every entry with Go changes run once: ~/.claude/skills/otto-go-gates/scripts/pr_gates.sh \"<worktree>\" \"<base>\" --main \"<repo>\" --out \"<run context dir>/go-standards-evals.md\" and reply with its table verbatim.";
const POST: &str = "Fetch the review's full findings first. Post every bug and warn finding as an inline comment with otto.comment_pr at its path and line; all info findings in one combined comment. Then post one summary comment with the score, the reviewed HEAD and a 'Gotchas' section, plus the gate table from go-standards-evals.md. The checkout table is in step3; the repos are listed in repos.json.";

fn reviewer() -> Workflow {
    wf(
        &[
            ("n1", "manual_trigger", "Start", json!({})),
            (
                "n2",
                "prepare_context",
                "Fetch Jira",
                json!({"require": false}),
            ),
            (
                "n3",
                "agent_prompt",
                "Checkout PR branch",
                json!({"provider": "claude", "prompt": CHECKOUT}),
            ),
            (
                "n4",
                "agent_prompt",
                "Gather context",
                json!({"provider": "claude", "skills": ["okf-authoring"], "prompt": CONTEXT}),
            ),
            (
                "n11",
                "agent_prompt",
                "Cross-repo check",
                json!({"provider": "claude", "prompt": CROSS}),
            ),
            (
                "n12",
                "agent_prompt",
                "Go PR gates",
                json!({"provider": "claude", "skills": ["otto-go-gates"], "prompt": GATES}),
            ),
            (
                "n5",
                "review_run",
                "Review",
                json!({"threshold": 80, "reviewers": [{"lens": "correctness-review"}, {"lens": "security-review"}]}),
            ),
            (
                "n7",
                "condition",
                "Passed?",
                json!({"expr": "passed == true"}),
            ),
            (
                "n8",
                "channel_notify",
                "Notify passed",
                json!({"message": "passed"}),
            ),
            (
                "n9",
                "channel_notify",
                "Notify failed",
                json!({"message": "failed"}),
            ),
            (
                "n6",
                "agent_prompt",
                "Post inline comments",
                json!({"provider": "codex", "prompt": POST}),
            ),
            ("n10", "log", "Final", json!({})),
        ],
        &[
            ("n1", "n2", None),
            ("n2", "n3", None),
            ("n3", "n4", None),
            ("n4", "n11", None),
            ("n5", "n7", None),
            ("n7", "n8", Some("output.result == true")),
            ("n7", "n9", Some("output.result == false")),
            ("n8", "n6", None),
            ("n9", "n6", None),
            ("n6", "n10", None),
            ("n11", "n12", None),
            ("n12", "n5", None),
        ],
    )
}

fn no_problems(_: &WorkflowGraph) -> Vec<String> {
    vec![]
}

fn opts() -> Options<'static> {
    Options {
        selection: None,
        max_parallel_steps: 4,
        validate: &no_problems,
    }
}

fn edges(g: &WorkflowGraph) -> BTreeSet<String> {
    g.edges
        .iter()
        .map(|e| match &e.condition {
            Some(c) => format!("{}->{} [{c}]", e.source, e.target),
            None => format!("{}->{}", e.source, e.target),
        })
        .collect()
}

fn ids(d: &OptimizeDraft) -> Vec<(String, bool)> {
    d.changes
        .iter()
        .map(|c| (c.id.clone(), c.preselected))
        .collect()
}

#[test]
fn a_pr_reviewer_shape_gets_the_engine_md_8_1_draft_without_moving_the_notify_steps() {
    let base = reviewer();
    let d = plan(&base, &[], &opts());
    assert_eq!(
        ids(&d),
        [
            ("git_checkout:n3", true),
            ("single_repo_skip:n11", true),
            ("go_gates_script:n12", true),
            ("post_review_comments:n6", true),
            ("parallel:n3", true),
            ("parallel:n12", true),
        ]
        .map(|(i, p)| (i.to_string(), p)),
        "not offered: {:?}",
        d.not_offered
    );
    let want: BTreeSet<String> = [
        "n1->n2",
        "n1->n3",
        "n2->n4",
        "n3->n4",
        "n4->n11 [len(run.repos) > 1]",
        "n3->n12",
        "n12->n5",
        "n11->n5",
        "n4->n5",
        "n5->n7",
        "n7->n8 [output.result == true]",
        "n7->n9 [output.result == false]",
        "n8->n13",
        "n9->n13",
        "n13->n6",
        "n6->n10",
    ]
    .map(String::from)
    .into();
    assert_eq!(edges(&d.draft_graph), want);
    let g = &d.draft_graph;
    let n = |id: &str| node(g, id).unwrap();
    assert_eq!(
        (n("n3").kind.as_str(), n("n3").name.as_str()),
        ("git_checkout", "Checkout PR branch")
    );
    assert_eq!(n("n3").params, json!({"mode": "pr"}));
    assert_eq!(n("n12").kind, "script");
    assert_eq!(n("n12").params["output_file"], "go-standards-evals.md");
    assert_eq!(n("n6").kind, "post_review_comments");
    assert_eq!(
        n("n6").params,
        json!({"review_from": "n5", "summary_files": ["go-standards-evals.md"], "summary_intro_from": "n13"})
    );
    assert_eq!(
        (n("n13").name.as_str(), n("n13").params["provider"].as_str()),
        ("Draft PR summary", Some("codex"))
    );
    let pos = |id: &str| g.nodes.iter().position(|x| x.id == id).unwrap();
    assert_eq!(
        pos("n13") + 1,
        pos("n6"),
        "the new step sits before the one it feeds"
    );
    // Kept steps are byte-identical.
    for id in ["n2", "n4", "n11", "n5", "n7", "n8", "n9", "n10"] {
        let b = node(&base.graph, id).unwrap();
        assert_eq!(
            (n(id).kind.clone(), n(id).params.clone()),
            (b.kind.clone(), b.params.clone()),
            "{id}"
        );
    }
    assert_eq!(d.base_version, 7);
    // step4 (the context brief Cross-repo reads) keeps its node in every outcome.
    assert_eq!(
        d.checks.numbering,
        vec![NumberingCheck {
            step_no: 4,
            node_id: "n4".into(),
            ok: true
        }]
    );
    assert!(
        d.checks
            .renumbered
            .iter()
            .any(|r| r.node_id == "n12" && r.from == "step6" && r.to == "step5"),
        "{:?}",
        d.checks.renumbered
    );
}

#[test]
fn checks_files_reads_writes_reads_added() {
    let d = plan(&reviewer(), &[], &opts());
    let row = |id: &str| {
        d.checks
            .files
            .iter()
            .find(|f| f.node_id == id)
            .unwrap()
            .clone()
    };
    let post = row("n6");
    assert_eq!(post.reads, ["go-standards-evals.md", "prs.json"]);
    assert_eq!(post.reads_added, ["prs.json"]);
    // The registry the old prompt read is not summary text (and not read).
    assert_eq!(post.reads_lost, ["repos.json"]);
    let gates = row("n12");
    assert_eq!(gates.reads, ["repos.json"]);
    assert!(
        gates.writes.contains(&"go-standards-evals.md".to_string())
            && gates
                .writes
                .iter()
                .any(|w| w.starts_with("step5-go-pr-gates")),
        "{:?}",
        gates.writes
    );
    // Go PR gates no longer descends from the context brief and the cross-repo check.
    assert_eq!(
        gates.listing_removed,
        [
            "step2-fetch-jira.md",
            "step4-gather-context.md",
            "step5-cross-repo-check.md"
        ]
    );
    let checkout = row("n3");
    assert_eq!(
        (checkout.reads_added.clone(), checkout.reads_lost.clone()),
        (
            vec!["prs.json".to_string()],
            vec!["run-brief.md".to_string()]
        )
    );
    assert_eq!(
        (checkout.reads.clone(), checkout.listing_removed.clone()),
        (
            vec!["prs.json".to_string(), "repos.json".to_string()],
            vec!["step2-fetch-jira.md".to_string()]
        )
    );
}

#[test]
fn a_quick_review_shape_skips_info_and_gotchas_and_offers_the_lens_hold_unselected() {
    let quick_post = "QUICK MODE: post only bug and warn findings inline with otto_comment_pr (there are no 'info' findings). In the summary comment OMIT the 'Gotchas' section.";
    let base = wf(
        &[
            ("n1", "manual_trigger", "Start", json!({})),
            ("n2", "prepare_context", "Fetch Jira", json!({})),
            (
                "n3",
                "agent_prompt",
                "Checkout PR branch",
                json!({"provider": "claude", "prompt": CHECKOUT}),
            ),
            (
                "n12",
                "agent_prompt",
                "Go PR gates",
                json!({"provider": "claude", "skills": ["otto-go-gates"], "prompt": GATES}),
            ),
            (
                "n5",
                "review_run",
                "Quick review",
                json!({"mode": "orchestrator", "reviewers": [{"lens": "a"}, {"lens": "b"}]}),
            ),
            (
                "n6",
                "agent_prompt",
                "Post inline comments",
                json!({"provider": "claude", "prompt": quick_post}),
            ),
        ],
        &[
            ("n1", "n2", None),
            ("n2", "n3", None),
            ("n3", "n12", None),
            ("n12", "n5", None),
            ("n5", "n6", None),
        ],
    );
    let d = plan(&base, &[], &opts());
    assert!(
        ids(&d).contains(&("review_declared_lenses:n5".into(), false)),
        "{:?}",
        ids(&d)
    );
    assert!(d
        .changes
        .iter()
        .any(|c| c.id == "review_declared_lenses:n5" && c.effect == Effect::ChangesFindings));
    let g = &d.draft_graph;
    assert!(
        node(g, "n5")
            .unwrap()
            .params
            .get("scope_assessment")
            .is_none(),
        "never pre-selected"
    );
    assert_eq!(
        node(g, "n6").unwrap().params,
        json!({"review_from": "n5", "summary_files": [], "info": "skip"})
    );
    assert_eq!(g.nodes.len(), 6, "no Gotchas step");
    let want: BTreeSet<String> = ["n1->n2", "n1->n3", "n3->n12", "n12->n5", "n2->n5", "n5->n6"]
        .map(String::from)
        .into();
    assert_eq!(edges(g), want);
    // Ticking the lens hold sets it.
    let sel: Vec<String> = d.changes.iter().map(|c| c.id.clone()).collect();
    let all = plan(
        &base,
        &[],
        &Options {
            selection: Some(&sel),
            ..opts()
        },
    );
    assert_eq!(
        node(&all.draft_graph, "n5").unwrap().params["scope_assessment"],
        "configured"
    );
}

#[test]
fn a_branch_checkout_and_a_report_without_a_pr_are_recognised_correctly() {
    let checkout = format!("{CHECKOUT} This workflow reviews a branch: do not look for a PR.");
    let report = "Write the review report. There is no PR in this workflow, so there is nowhere to anchor an inline comment: carry each finding's file and line in the report. Do not call otto_comment_pr.";
    let base = wf(
        &[
            ("n1", "manual_trigger", "Start", json!({})),
            (
                "n3",
                "agent_prompt",
                "Checkout feature branch",
                json!({"prompt": checkout}),
            ),
            ("n5", "review_run", "Review", json!({})),
            (
                "n6",
                "agent_prompt",
                "Write review report",
                json!({"prompt": report}),
            ),
        ],
        &[("n1", "n3", None), ("n3", "n5", None), ("n5", "n6", None)],
    );
    let d = plan(&base, &[], &opts());
    assert_eq!(
        node(&d.draft_graph, "n3").unwrap().params,
        json!({"mode": "branch"})
    );
    assert!(
        !d.changes.iter().any(|c| c.rule == Rule::PostReviewComments),
        "{:?}",
        ids(&d)
    );
}

#[test]
fn steps_that_only_say_what_they_read_in_prose_are_never_moved() {
    // Two sweeps after the document step that name no file: they stay put.
    let base = wf(
        &[
            ("n1", "manual_trigger", "Start", json!({})),
            (
                "n4",
                "agent_prompt",
                "Write the document",
                json!({"prompt": "Write the test case document for the story."}),
            ),
            (
                "n10",
                "agent_prompt",
                "Sweep A",
                json!({"prompt": "Read the document produced in this run and list what is missing. Do not post anything."}),
            ),
            (
                "n11",
                "agent_prompt",
                "Sweep B",
                json!({"prompt": "Check the document produced in this run against the epic."}),
            ),
        ],
        &[("n1", "n4", None), ("n4", "n10", None), ("n4", "n11", None)],
    );
    let d = plan(&base, &[], &opts());
    assert!(d.changes.is_empty(), "{:?}", ids(&d));
    assert_eq!(edges(&d.draft_graph), edges(&base.graph));
}

#[test]
fn outward_or_previous_step_wording_makes_a_step_unmovable_and_look_alikes_do_not_fire() {
    let step = |prompt: &str| {
        wf(
            &[
                ("n1", "manual_trigger", "Start", json!({})),
                (
                    "n2",
                    "agent_prompt",
                    "Notes",
                    json!({"prompt": "Write the notes to notes.md."}),
                ),
                (
                    "n3",
                    "agent_prompt",
                    "Other",
                    json!({"prompt": "Write the summary to other.md."}),
                ),
                ("n4", "agent_prompt", "Reader", json!({"prompt": prompt})),
            ],
            &[("n1", "n2", None), ("n2", "n3", None), ("n3", "n4", None)],
        )
    };
    let moved = |prompt: &str| {
        plan(&step(prompt), &[], &opts())
            .changes
            .iter()
            .any(|c| c.id == "parallel:n4")
    };
    assert!(
        moved("Read notes.md and summarise it at a high level."),
        "'high' is not gh"
    );
    for blocked in [
        "Read notes.md, then curl the result to the server.",
        "Read notes.md and git push the branch.",
        "Read notes.md, then open it with gh pr create.",
        "Read notes.md and call otto_create_pr.",
        "Read notes.md and use otto.slack_post_message.",
        "Read notes.md from the previous step.",
        "Read notes.md; authenticate with OTTO_MCP_TOKEN.",
    ] {
        assert!(!moved(blocked), "{blocked}");
    }
    // A prompt that mentions merge-base but is no checkout recipe.
    let look = wf(
        &[
            ("n1", "manual_trigger", "Start", json!({})),
            (
                "n2",
                "agent_prompt",
                "Explain",
                json!({"prompt": "Explain what merge-base means for repos.json."}),
            ),
        ],
        &[("n1", "n2", None)],
    );
    assert!(plan(&look, &[], &opts())
        .changes
        .iter()
        .all(|c| c.rule != Rule::GitCheckout));
}

#[test]
fn the_gates_out_name_must_be_plain_and_never_reaches_the_shell_text() {
    let gates = |out: &str| {
        wf(
            &[
                ("n1", "manual_trigger", "Start", json!({})),
                (
                    "n2",
                    "agent_prompt",
                    "Gates",
                    json!({"skills": ["otto-go-gates"], "prompt": format!("Run ~/.claude/skills/otto-go-gates/scripts/pr_gates.sh \"<worktree>\" \"<base>\" --out {out} and reply with the table.")}),
                ),
            ],
            &[("n1", "n2", None)],
        )
    };
    for bad in [
        "\"x; rm -rf ~\"",
        "../a",
        "\"<dir>/../../etc/passwd\"",
        "\"$(id).md\"",
    ] {
        let d = plan(&gates(bad), &[], &opts());
        assert!(
            d.changes.iter().all(|c| c.rule != Rule::GoGatesScript),
            "{bad}"
        );
        assert_eq!(
            d.not_offered[0].reason, "output file name in the prompt is not a plain file name",
            "{bad}"
        );
    }
    let d = plan(&gates("\"<run context dir>/gates.md\""), &[], &opts());
    let p = &node(&d.draft_graph, "n2").unwrap().params;
    assert_eq!(
        (
            p["output_file"].as_str(),
            p["foreach"].as_str(),
            p["idempotent"].as_bool()
        ),
        (Some("gates.md"), Some("repos"), Some(true))
    );
    let cmd = p["command"].as_str().unwrap();
    assert!(
        !cmd.contains("gates.md")
            && cmd.contains("$OTTO_OUT")
            && cmd.contains("\"$HOME/.claude/skills/otto-go-gates/scripts/pr_gates.sh\""),
        "{cmd}"
    );
}

/// The script path is read through trailing punctuation around quotes, and a
/// prompt that names no path ending in `pr_gates.sh` is not offered (CR1-013).
#[test]
fn the_gates_script_path_survives_punctuation_and_is_never_empty() {
    let gates = |prompt: &str| {
        let g = wf(
            &[
                ("n1", "manual_trigger", "Start", json!({})),
                (
                    "n2",
                    "agent_prompt",
                    "Gates",
                    json!({"skills": ["otto-go-gates"], "prompt": prompt}),
                ),
            ],
            &[("n1", "n2", None)],
        );
        plan(&g, &[], &opts())
    };
    let d = gates("Run `~/bin/pr_gates.sh`. With --out gates.md reply with the table.");
    let cmd = node(&d.draft_graph, "n2").unwrap().params["command"]
        .as_str()
        .unwrap_or_else(|| panic!("not offered: {:?}", d.not_offered))
        .to_string();
    assert!(
        cmd.contains("\n\"$HOME/bin/pr_gates.sh\" \"$OTTO_WORKTREE\""),
        "{cmd}"
    );
    let d = gates("Run pr_gates.sh.bak --out gates.md and reply with the table.");
    assert!(
        d.changes.iter().all(|c| c.rule != Rule::GoGatesScript),
        "{:?}",
        d.changes
    );
    assert_eq!(
        d.not_offered[0].reason,
        "the gates script path in the prompt is not a plain path"
    );
}

/// A graph where moving C earlier would give C the step number a prompt refers to.
#[test]
fn parallel_breaking_referenced_number_not_offered() {
    let base = wf(
        &[
            ("n1", "manual_trigger", "Start", json!({})),
            (
                "n2",
                "agent_prompt",
                "A",
                json!({"prompt": "Write the findings to a.md."}),
            ),
            (
                "n3",
                "agent_prompt",
                "B",
                json!({"prompt": "Read a.md and write the plan to b.md."}),
            ),
            (
                "n4",
                "agent_prompt",
                "C",
                json!({"prompt": "Read run-brief.md and write c.md."}),
            ),
            (
                "n5",
                "agent_prompt",
                "D",
                json!({"prompt": "Read b.md and the summary in step3, then write d.md."}),
            ),
        ],
        &[
            ("n1", "n2", None),
            ("n2", "n3", None),
            ("n3", "n4", None),
            ("n4", "n5", None),
        ],
    );
    let d = plan(&base, &[], &opts());
    let no = d
        .not_offered
        .iter()
        .find(|n| n.nodes == ["n4"])
        .expect("C's move is refused");
    assert_eq!(no.rule, Rule::Parallel);
    assert!(
        no.reason.contains("step3") && no.reason.contains("“D”"),
        "{}",
        no.reason
    );
    assert_eq!(
        node(&d.draft_graph, "n4").map(|_| edges(&d.draft_graph).contains("n3->n4")),
        Some(true)
    );
}

#[test]
fn offered_drafts_keep_referenced_numbers_all_combinations() {
    for seed in 1..200u64 {
        let base = random_workflow(seed);
        let d = plan(&base, &[], &opts());
        let order = topo_order(&d.draft_graph).unwrap();
        assert!(
            numbering_check(&base.graph, &d.draft_graph, &order).is_ok(),
            "seed {seed}"
        );
    }
}

#[test]
fn lost_reader_drops_change() {
    let base = wf(
        &[
            ("n1", "manual_trigger", "Start", json!({})),
            (
                "n2",
                "agent_prompt",
                "A",
                json!({"prompt": "Write the findings to a.md."}),
            ),
            (
                "n3",
                "agent_prompt",
                "B",
                json!({"prompt": "Read a.md and reply."}),
            ),
        ],
        &[("n1", "n2", None), ("n2", "n3", None)],
    );
    let mut draft = base.graph.clone();
    draft.edges.retain(|e| e.id != "n2-n3");
    draft.edges.push(WorkflowEdge {
        id: "x".into(),
        source: "n1".into(),
        target: "n3".into(),
        condition: None,
    });
    let err = check(&base.graph, &draft, &[], &opts()).unwrap_err();
    assert_eq!(err, "“B” reads what “A” writes but would start before it");
}

#[test]
fn outward_steps_keep_their_order_and_validation_failures_are_not_offered() {
    let base = wf(
        &[
            ("n1", "manual_trigger", "Start", json!({})),
            ("n2", "channel_notify", "Tell", json!({"message": "x"})),
            (
                "n3",
                "agent_prompt",
                "Publish",
                json!({"prompt": "curl the page to the wiki."}),
            ),
        ],
        &[("n1", "n2", None), ("n2", "n3", None)],
    );
    let mut draft = base.graph.clone();
    draft.edges[1].source = "n1".into();
    assert_eq!(
        check(&base.graph, &draft, &[], &opts()).unwrap_err(),
        "“Publish” would no longer wait for “Tell”"
    );
    let refuse = |g: &WorkflowGraph| {
        if g.nodes.iter().any(|n| n.kind == "git_checkout") {
            vec!["unknown kind".to_string()]
        } else {
            vec![]
        }
    };
    let d = plan(
        &reviewer(),
        &[],
        &Options {
            validate: &refuse,
            ..opts()
        },
    );
    let no = d
        .not_offered
        .iter()
        .find(|n| n.rule == Rule::GitCheckout)
        .unwrap();
    assert_eq!(no.reason, "the draft does not validate: unknown kind");
}

#[test]
fn property_protected_fields_and_outward_order() {
    let mut offered = 0;
    for seed in 1..300u64 {
        let base = random_workflow(seed);
        let d = plan(&base, &[], &opts());
        offered += d.changes.len();
        let replaced: Vec<&String> = d
            .changes
            .iter()
            .filter(|c| {
                c.preselected
                    && matches!(
                        c.rule,
                        Rule::GitCheckout | Rule::GoGatesScript | Rule::PostReviewComments
                    )
            })
            .flat_map(|c| &c.nodes)
            .collect();
        for b in &base.graph.nodes {
            let n = node(&d.draft_graph, &b.id)
                .unwrap_or_else(|| panic!("seed {seed}: {} removed", b.id));
            assert_eq!(n.name, b.name, "seed {seed}");
            if !replaced.contains(&&b.id) {
                assert_eq!(
                    serde_json::to_string(&n.params).unwrap(),
                    serde_json::to_string(&b.params).unwrap(),
                    "seed {seed} {}",
                    b.id
                );
                assert_eq!(n.kind, b.kind);
            }
        }
        for b in base.graph.nodes.iter().filter(|n| outward(n)) {
            let now = ancestors(&d.draft_graph, &b.id);
            for a in ancestors(&base.graph, &b.id) {
                if node(&base.graph, &a).is_some_and(outward) {
                    assert!(
                        now.contains(&a),
                        "seed {seed}: {} no longer after {a}",
                        b.id
                    );
                }
            }
        }
        assert!(topo_order(&d.draft_graph).is_ok());
    }
    assert!(
        offered > 50,
        "the random graphs exercise the rules: {offered}"
    );
}

/// Random DAGs of 3–12 nodes mixing movable agents (with files they read
/// and write), outward steps, conditions and notify steps.
fn random_workflow(seed: u64) -> Workflow {
    let mut s = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    let mut next = |m: u64| {
        s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (s >> 33) % m
    };
    let count = 3 + next(10) as usize;
    let mut nodes: Vec<(String, &str, String, Value)> =
        vec![("n1".into(), "manual_trigger", "Start".into(), json!({}))];
    for i in 2..=count {
        let (kind, params) = match next(6) {
            0 => ("channel_notify", json!({"message": "hi"})),
            1 => ("condition", json!({"expr": "true"})),
            2 => (
                "agent_prompt",
                json!({"prompt": format!("Read f{}.md and curl it.", next(count as u64))}),
            ),
            3 => (
                "agent_prompt",
                json!({"prompt": format!("Read f{}.md and step{}; write the result to f{i}.md.", next(count as u64), 1 + next(count as u64))}),
            ),
            _ => (
                "agent_prompt",
                json!({"prompt": format!("Read f{}.md; write the notes to f{i}.md.", next(count as u64))}),
            ),
        };
        nodes.push((format!("n{i}"), kind, format!("Step {i}"), params));
    }
    let mut es: Vec<(String, String, Option<&str>)> = Vec::new();
    for i in 2..=count {
        let parents = 1 + next(2);
        for _ in 0..parents {
            let p = 1 + next(i as u64 - 1);
            let from_condition = nodes[p as usize - 1].1 == "condition";
            let e = (
                format!("n{p}"),
                format!("n{i}"),
                from_condition.then_some(if next(2) == 0 {
                    "output.result == true"
                } else {
                    "output.result == false"
                }),
            );
            if !es.iter().any(|x| x.0 == e.0 && x.1 == e.1) {
                es.push(e);
            }
        }
    }
    let n: Vec<(&str, &str, &str, Value)> = nodes
        .iter()
        .map(|(a, b, c, d)| (a.as_str(), *b, c.as_str(), d.clone()))
        .collect();
    let e: Vec<(&str, &str, Option<&str>)> = es
        .iter()
        .map(|(a, b, c)| (a.as_str(), b.as_str(), *c))
        .collect();
    wf(&n, &e)
}

fn run(at_day: i64, times: &[(&str, u64)], repos: usize, with_phases: bool) -> WorkflowRun {
    let start = Utc.with_ymd_and_hms(2026, 11, 1, 9, 0, 0).unwrap() + Duration::days(at_day);
    let nodes = times
        .iter()
        .map(|(id, ms)| {
            let mut s = NodeRunState {
                node_id: id.to_string(),
                status: NodeStatus::Success,
                duration_ms: Some(*ms),
                ..NodeRunState::default()
            };
            if with_phases {
                // 10 s waiting for the machine before working.
                s.phases = vec![
                    PhaseMark {
                        phase: StepPhase::Waiting,
                        at: start,
                        note: None,
                    },
                    PhaseMark {
                        phase: StepPhase::Working,
                        at: start + Duration::seconds(10),
                        note: None,
                    },
                ];
            }
            s
        })
        .collect();
    serde_json::from_value(json!({
        "id": format!("r{at_day}"), "workflow_id": "wf1", "workspace_id": "ws", "status": "success",
        "input": {"repos": vec![json!({}); repos]}, "started_at": start,
    }))
    .map(|mut r: WorkflowRun| {
        r.nodes = nodes;
        r
    })
    .unwrap()
}

#[test]
fn estimate_from_fixture_history() {
    // A → B → C, where C reads only what A writes: moving C beside B saves B's time.
    let base = wf(
        &[
            ("n1", "manual_trigger", "Start", json!({})),
            (
                "n2",
                "agent_prompt",
                "A",
                json!({"prompt": "Write the findings to a.md."}),
            ),
            (
                "n3",
                "agent_prompt",
                "B",
                json!({"prompt": "Read a.md and write b.md."}),
            ),
            (
                "n4",
                "agent_prompt",
                "C",
                json!({"prompt": "Read a.md and write c.md."}),
            ),
        ],
        &[("n1", "n2", None), ("n2", "n3", None), ("n3", "n4", None)],
    );
    let history = vec![
        run(
            0,
            &[("n2", 60_000), ("n3", 120_000), ("n4", 30_000)],
            1,
            false,
        ),
        run(
            1,
            &[("n2", 70_000), ("n3", 130_000), ("n4", 40_000)],
            1,
            true,
        ),
        run(
            2,
            &[("n2", 80_000), ("n3", 140_000), ("n4", 50_000)],
            1,
            true,
        ),
        run(
            3,
            &[("n2", 90_000), ("n3", 400_000), ("n4", 60_000)],
            1,
            true,
        ),
    ];
    let d = plan(&base, &history, &opts());
    let e = &d.estimate;
    assert_eq!(e.runs, 4);
    // Working times (minus 10 s of waiting where phases exist):
    // A 60,60,70,80 → median 60, P90 80; B 120,120,130,390 → 120 / 390; C 30,30,40,50 → 30 / 50.
    assert_eq!(
        (e.typical_before_ms, e.typical_after_ms),
        (Some(210_000), Some(180_000))
    );
    assert_eq!(
        (e.slow_before_ms, e.slow_after_ms),
        (Some(520_000), Some(470_000))
    );
    assert!(
        e.note
            .as_deref()
            .unwrap()
            .starts_with("older runs include machine waits"),
        "{:?}",
        e.note
    );
    assert_eq!(
        d.changes
            .iter()
            .find(|c| c.id == "parallel:n4")
            .unwrap()
            .saving_ms,
        Some(30_000)
    );
    assert_eq!(e.since, Some(history[0].started_at));
}

#[test]
fn under_three_runs_no_number() {
    let history = vec![
        run(0, &[("n3", 1_000)], 1, false),
        run(1, &[("n3", 1_000)], 1, false),
    ];
    let e = plan(&reviewer(), &history, &opts()).estimate;
    assert_eq!(
        (e.runs, e.typical_before_ms, e.note.as_deref()),
        (2, None, Some("Not enough runs to estimate yet"))
    );
    assert!(plan(&reviewer(), &history, &opts())
        .changes
        .iter()
        .all(|c| c.saving_ms.is_none()));
}

#[test]
fn a_single_repo_guard_counts_zero_in_single_repo_runs() {
    let times = [
        ("n2", 10_000),
        ("n3", 60_000),
        ("n4", 60_000),
        ("n11", 300_000),
        ("n12", 100_000),
        ("n5", 400_000),
        ("n6", 100_000),
    ];
    let history: Vec<WorkflowRun> = (0..5).map(|i| run(i, &times, 1, true)).collect();
    let d = plan(&reviewer(), &history, &opts());
    let e = &d.estimate;
    assert!(
        e.typical_after_ms.unwrap() + 300_000 <= e.typical_before_ms.unwrap(),
        "{e:?}"
    );
    assert_eq!(
        d.changes
            .iter()
            .find(|c| c.id == "single_repo_skip:n11")
            .unwrap()
            .saving_ms,
        Some(290_000)
    );
}

#[test]
fn slow_steps_get_an_unselected_timeout_from_their_own_history() {
    let history: Vec<WorkflowRun> = (0..4)
        .map(|i| {
            run(
                i,
                &[("n4", 20 * 60_000 + 10_000), ("n12", 30 * 60_000)],
                1,
                true,
            )
        })
        .collect();
    let d = plan(&reviewer(), &history, &opts());
    let c = d
        .changes
        .iter()
        .find(|c| c.id == "step_timeout:n4")
        .unwrap();
    assert!(!c.preselected);
    assert_eq!(c.title, "Stop “Gather context” after 40 minutes");
    assert!(
        !d.changes.iter().any(|c| c.id == "step_timeout:n12"),
        "a step a rule replaces gets no agent timeout"
    );
}

#[test]
fn apply_rederives_exactly_the_chosen_changes() {
    let base = reviewer();
    let ids = vec!["git_checkout:n3".to_string(), "parallel:n3".to_string()];
    let (g, _) = apply(&base, &[], &ids, &opts()).unwrap();
    assert_eq!(
        edges(&g),
        edges(
            &plan(
                &base,
                &[],
                &Options {
                    selection: Some(&ids),
                    ..opts()
                }
            )
            .draft_graph
        )
    );
    assert_eq!(
        node(&g, "n12").unwrap().kind,
        "agent_prompt",
        "only what was chosen"
    );
    assert!(edges(&g).contains("n1->n3"));
    let err = apply(&base, &[], &["parallel:n99".to_string()], &opts()).unwrap_err();
    assert_eq!(err, "parallel:n99: it no longer applies to this version");
    let refuse = |g: &WorkflowGraph| {
        if g.nodes.iter().any(|n| n.kind == "script") {
            vec!["script needs a command".to_string()]
        } else {
            vec![]
        }
    };
    let err = apply(
        &base,
        &[],
        &["go_gates_script:n12".to_string()],
        &Options {
            validate: &refuse,
            ..opts()
        },
    )
    .unwrap_err();
    assert_eq!(
        err,
        "go_gates_script:n12: the draft does not validate: script needs a command"
    );
}

#[test]
fn the_version_note_names_the_applied_changes() {
    let d = plan(&reviewer(), &[], &opts());
    let note = version_note(
        "Dana",
        &d.changes,
        &[
            "git_checkout:n3".to_string(),
            "single_repo_skip:n11".to_string(),
        ],
    );
    assert_eq!(note, "Optimized by Dana: Check out with Otto instead of “Checkout PR branch”; Skip “Cross-repo check” when the run has one repository");
}

#[test]
fn the_wire_shape_matches_the_contract() {
    let v = serde_json::to_value(plan(&reviewer(), &[], &opts())).unwrap();
    for k in [
        "base_version",
        "changes",
        "not_offered",
        "draft_graph",
        "estimate",
        "checks",
    ] {
        assert!(v.get(k).is_some(), "{k}");
    }
    assert_eq!(v["changes"][0]["rule"], "git_checkout");
    assert_eq!(v["changes"][0]["effect"], "may_change_output");
    for k in ["numbering", "renumbered", "files", "warnings"] {
        assert!(v["checks"].get(k).is_some(), "{k}");
    }
    for k in [
        "node_id",
        "reads",
        "writes",
        "reads_added",
        "reads_lost",
        "listing_removed",
    ] {
        assert!(v["checks"]["files"][0].get(k).is_some(), "{k}");
    }
}

/// The drafted gates command, run by `sh` against a fake `pr_gates.sh`: a
/// failed gate with its report is the table (success), a script that writes
/// no fresh report fails the step instead of passing on a stale one.
#[test]
#[allow(clippy::disallowed_methods)] // a test drives sh and git directly
fn the_gates_command_fails_without_a_fresh_report_and_keeps_failed_gate_rows() {
    let td = tempfile::tempdir().unwrap();
    let root = td.path().canonicalize().unwrap();
    let (repo, ctx) = (root.join("repo"), root.join("ctx"));
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(&ctx).unwrap();
    let git = |args: &[&str]| {
        let ok = otto_git::hardened_std_command()
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(&repo)
            .output()
            .unwrap();
        assert!(ok.status.success(), "{ok:?}");
    };
    git(&["init", "-q"]);
    git(&["commit", "-q", "--allow-empty", "-m", "base"]);
    std::fs::write(repo.join("a.go"), "package a\n").unwrap();
    git(&["add", "a.go"]);
    git(&["commit", "-q", "-m", "go"]);
    let script = root.join("pr_gates.sh");
    std::fs::write(
        &script,
        "#!/bin/sh\nwhile [ \"$1\" != --out ]; do shift; done\ncase \"$FAKE\" in report-fail) echo '| vet | FAIL |' > \"$2\"; exit 1;; *) exit 2;; esac\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    // "pr_gates.sh:" — trailing punctuation is not part of the path.
    let prompt = format!(
        "Run {}: \"<worktree>\" --out gates.md and reply with the table.",
        script.display()
    );
    let g = wf(
        &[
            ("n1", "manual_trigger", "Start", json!({})),
            (
                "n2",
                "agent_prompt",
                "Gates",
                json!({"skills": ["otto-go-gates"], "prompt": prompt}),
            ),
        ],
        &[("n1", "n2", None)],
    );
    let d = plan(&g, &[], &opts());
    let cmd = node(&d.draft_graph, "n2").unwrap().params["command"]
        .as_str()
        .unwrap_or_else(|| panic!("not offered: {:?}", d.not_offered))
        .to_string();
    let run = |fake: &str| {
        std::process::Command::new("/bin/sh")
            .args(["-c", &cmd])
            .env("FAKE", fake)
            .env("OTTO_WORKTREE", &repo)
            .env("OTTO_BASE", "HEAD~1")
            .env("OTTO_REPO", &repo)
            .env("OTTO_REPO_NAME", "app")
            .env("OTTO_CONTEXT_DIR", &ctx)
            .env("OTTO_OUT", "gates.md")
            .status()
            .unwrap()
            .success()
    };
    assert!(
        run("report-fail"),
        "a failed gate with its report is the table"
    );
    let table = std::fs::read_to_string(ctx.join("gates.md")).unwrap();
    assert!(table.contains("| vet | FAIL |"), "{table}");
    // A retry whose script crashes: the last attempt's report is not reused.
    std::fs::remove_file(ctx.join("gates.md")).unwrap();
    assert!(!run("crash"), "no fresh report fails the step");
    assert!(!ctx.join("gates.md").exists(), "nothing stale appended");
}
