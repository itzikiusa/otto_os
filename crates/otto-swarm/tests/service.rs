//! Service-level tests for the Agent Swarm CRUD + org tree, on an in-memory
//! SQLite with every migration applied (`otto_state::db::test_pool`).
//!
//! Requests are built from JSON (the HTTP body shape) rather than struct
//! literals, so these tests pin the wire contract and survive new optional
//! fields on the request types.

use otto_core::Id;
use otto_state::SwarmRepo;
use otto_swarm::presets::{instantiate, list_presets};
use otto_swarm::{
    CreateAgentReq, CreateProjectReq, CreateSwarmReq, CreateTaskReq, SwarmService, UpdateAgentReq,
    UpdateSwarmReq,
};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

fn req<T: serde::de::DeserializeOwned>(v: Value) -> T {
    serde_json::from_value(v).expect("request body deserializes")
}

async fn service() -> SwarmService {
    SwarmService::new(SwarmRepo::new(otto_state::db::test_pool().await))
}

fn ws() -> Id {
    "ws-1".into()
}
fn user() -> Id {
    "user-1".into()
}

#[tokio::test]
async fn create_swarm_applies_defaults_and_threads_the_default_provider() {
    let svc = service().await;
    let s = svc
        .create_swarm(
            &ws(),
            &user(),
            req::<CreateSwarmReq>(json!({"name": "Team"})),
            "codex",
        )
        .await
        .unwrap();
    assert_eq!(s.name, "Team");
    assert_eq!(s.workspace_id, ws());
    assert_eq!(s.description, "");
    assert_eq!(
        s.config["provider"], "codex",
        "a new swarm honours the caller's default agent"
    );
    assert_eq!(s.config["max_parallel_sessions"], 4);
    assert_eq!(s.config["cwd_mode"], "scratch");
    assert_eq!(svc.list_swarms(&ws()).await.unwrap().len(), 1);
    assert!(
        svc.list_swarms(&"other-ws".into())
            .await
            .unwrap()
            .is_empty(),
        "swarms are workspace-scoped"
    );

    // An explicit config is kept verbatim.
    let s2 = svc
        .create_swarm(
            &ws(),
            &user(),
            req(json!({"name": "Custom", "config": {"provider": "claude", "max_parallel_sessions": 1}})),
            "codex",
        )
        .await
        .unwrap();
    assert_eq!(
        s2.config,
        json!({"provider": "claude", "max_parallel_sessions": 1})
    );
}

#[tokio::test]
async fn update_swarm_is_a_partial_patch_and_null_clears_a_budget() {
    let svc = service().await;
    let s = svc
        .create_swarm(
            &ws(),
            &user(),
            req(
                json!({"name": "A", "description": "d", "max_total_runs": 10, "max_cost_usd": 5.0}),
            ),
            "claude",
        )
        .await
        .unwrap();
    assert_eq!(s.max_total_runs, Some(10));

    // Absent keys stay; present keys change.
    let u = svc
        .update_swarm(&s.id, req::<UpdateSwarmReq>(json!({"name": "B"})))
        .await
        .unwrap();
    assert_eq!(u.name, "B");
    assert_eq!(u.description, "d");
    assert_eq!(u.max_total_runs, Some(10));
    assert_eq!(u.max_cost_usd, Some(5.0));

    // `null` clears (unlimited); a value sets.
    let u = svc
        .update_swarm(
            &s.id,
            req(json!({"max_total_runs": null, "max_cost_usd": 7.5})),
        )
        .await
        .unwrap();
    assert_eq!(u.max_total_runs, None);
    assert_eq!(u.max_cost_usd, Some(7.5));
}

#[tokio::test]
async fn org_tree_reports_to_round_trips_and_detail_counts_match() {
    let svc = service().await;
    let s = svc
        .create_swarm(&ws(), &user(), req(json!({"name": "Org"})), "claude")
        .await
        .unwrap();
    let ceo = svc
        .create_agent(
            &s,
            &user(),
            req::<CreateAgentReq>(json!({"name": "Ceo", "provider": "claude", "title": "CEO"})),
        )
        .await
        .unwrap();
    let cto = svc
        .create_agent(
            &s,
            &user(),
            req(json!({"name": "Cto", "provider": "claude", "reports_to": ceo.id})),
        )
        .await
        .unwrap();
    let dev = svc
        .create_agent(
            &s,
            &user(),
            req(json!({"name": "Dev", "provider": "codex", "reports_to": cto.id})),
        )
        .await
        .unwrap();
    assert_eq!(ceo.reports_to, None);
    assert_eq!(ceo.swarm_id, s.id);
    assert_eq!(
        ceo.workspace_id, s.workspace_id,
        "agents inherit the swarm's workspace"
    );
    assert_eq!(cto.reports_to.as_ref(), Some(&ceo.id));
    assert_eq!(
        dev.skills,
        json!([]),
        "omitted skills default to an empty list"
    );

    // Re-parent the dev under the CEO; other fields stay.
    let moved = svc
        .update_agent(
            &dev.id,
            req::<UpdateAgentReq>(json!({"reports_to": ceo.id})),
        )
        .await
        .unwrap();
    assert_eq!(moved.reports_to.as_ref(), Some(&ceo.id));
    assert_eq!(moved.provider, "codex");
    assert_eq!(moved.name, "Dev");

    let p = svc
        .create_project(&s, &user(), req::<CreateProjectReq>(json!({"name": "P"})))
        .await
        .unwrap();
    let t = svc
        .create_task(
            &p,
            &user(),
            req::<CreateTaskReq>(json!({"title": "T", "assignee_agent_id": dev.id})),
        )
        .await
        .unwrap();
    assert_eq!(
        t.status, "todo",
        "a hand-added task is immediately schedulable"
    );
    assert_eq!(t.priority, "medium");
    assert_eq!(t.swarm_id, s.id);

    let d = svc.detail(&s.id).await.unwrap();
    assert_eq!(d.counts.agents, 3);
    assert_eq!(d.counts.projects, 1);
    assert_eq!(d.counts.tasks, 1);
    assert_eq!(d.counts.running_runs, 0);
    assert_eq!(d.counts.total_runs, 0);
    // Every non-root agent's manager is a member of the same swarm.
    let ids: HashSet<_> = d.agents.iter().map(|a| a.id.clone()).collect();
    for a in &d.agents {
        if let Some(m) = &a.reports_to {
            assert!(ids.contains(m), "{} reports outside its swarm", a.name);
        }
    }
}

#[tokio::test]
async fn delete_swarm_cascades_to_every_child_and_leaves_siblings() {
    let svc = service().await;
    let doomed = svc
        .create_swarm(&ws(), &user(), req(json!({"name": "Doomed"})), "claude")
        .await
        .unwrap();
    let keep = svc
        .create_swarm(&ws(), &user(), req(json!({"name": "Keep"})), "claude")
        .await
        .unwrap();
    for s in [&doomed, &keep] {
        let a = svc
            .create_agent(s, &user(), req(json!({"name": "A", "provider": "claude"})))
            .await
            .unwrap();
        let p = svc
            .create_project(s, &user(), req(json!({"name": "P"})))
            .await
            .unwrap();
        svc.create_task(
            &p,
            &user(),
            req(json!({"title": "T", "assignee_agent_id": a.id})),
        )
        .await
        .unwrap();
    }
    svc.delete_swarm(&doomed.id).await.unwrap();

    assert!(svc.get_swarm(&doomed.id).await.is_err());
    assert!(svc.list_agents(&doomed.id).await.unwrap().is_empty());
    assert!(svc.list_projects(&doomed.id).await.unwrap().is_empty());
    assert!(svc
        .list_tasks_for_swarm(&doomed.id)
        .await
        .unwrap()
        .is_empty());

    let d = svc.detail(&keep.id).await.unwrap();
    assert_eq!(
        (d.counts.agents, d.counts.projects, d.counts.tasks),
        (1, 1, 1)
    );
    assert_eq!(svc.list_swarms(&ws()).await.unwrap().len(), 1);
}

#[tokio::test]
async fn delete_agent_and_task_remove_only_that_row() {
    let svc = service().await;
    let s = svc
        .create_swarm(&ws(), &user(), req(json!({"name": "S"})), "claude")
        .await
        .unwrap();
    let a = svc
        .create_agent(&s, &user(), req(json!({"name": "A", "provider": "claude"})))
        .await
        .unwrap();
    let b = svc
        .create_agent(&s, &user(), req(json!({"name": "B", "provider": "claude"})))
        .await
        .unwrap();
    let p = svc
        .create_project(&s, &user(), req(json!({"name": "P"})))
        .await
        .unwrap();
    let t1 = svc
        .create_task(&p, &user(), req(json!({"title": "1"})))
        .await
        .unwrap();
    let t2 = svc
        .create_task(&p, &user(), req(json!({"title": "2", "status": "backlog"})))
        .await
        .unwrap();
    assert_eq!(t2.status, "backlog", "an explicit status is preserved");

    svc.delete_agent(&a.id).await.unwrap();
    svc.delete_task(&t1.id).await.unwrap();
    let agents: Vec<_> = svc
        .list_agents(&s.id)
        .await
        .unwrap()
        .into_iter()
        .map(|a| a.id)
        .collect();
    assert_eq!(agents, vec![b.id]);
    let tasks: Vec<_> = svc
        .list_tasks(&p.id)
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.id)
        .collect();
    assert_eq!(tasks, vec![t2.id]);
}

/// Every bundled preset parses (a bad YAML is dropped with only a warn) and
/// describes a well-formed org TREE: unique keys, managers that exist, exactly
/// one root, no cycles.
#[test]
fn every_bundled_preset_parses_into_a_well_formed_org_tree() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/presets");
    let files = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|x| x == "yaml" || x == "yml")
        })
        .count();
    let presets = list_presets();
    assert!(files > 0);
    assert_eq!(presets.len(), files, "a bundled preset failed to parse");

    let mut slugs = HashSet::new();
    for p in &presets {
        assert!(
            slugs.insert(p.slug.clone()),
            "duplicate preset slug {}",
            p.slug
        );
        assert!(!p.agents.is_empty(), "{}: no agents", p.slug);
        assert!(p.max_parallel_sessions > 0, "{}: parallel cap", p.slug);
        let keys: HashSet<_> = p.agents.iter().map(|a| a.key.as_str()).collect();
        assert_eq!(
            keys.len(),
            p.agents.len(),
            "{}: duplicate agent keys",
            p.slug
        );
        let parent: HashMap<&str, Option<&str>> = p
            .agents
            .iter()
            .map(|a| (a.key.as_str(), a.reports_to.as_deref()))
            .collect();
        let roots = parent.values().filter(|m| m.is_none()).count();
        assert_eq!(roots, 1, "{}: an org tree has exactly one root", p.slug);
        for a in &p.agents {
            if let Some(m) = &a.reports_to {
                assert!(
                    keys.contains(m.as_str()),
                    "{}: {} reports to unknown {m}",
                    p.slug,
                    a.key
                );
            }
            // Walk up: must reach the root within N steps (no cycle).
            let mut cur = Some(a.key.as_str());
            let mut steps = 0;
            while let Some(k) = cur {
                cur = parent[k];
                steps += 1;
                assert!(
                    steps <= p.agents.len(),
                    "{}: reports_to cycle through {}",
                    p.slug,
                    a.key
                );
            }
        }
    }
}

#[tokio::test]
async fn instantiating_a_preset_wires_reports_to_by_agent_id() {
    let svc = service().await;
    let preset = list_presets()
        .into_iter()
        .max_by_key(|p| p.agents.len())
        .unwrap();
    let s = svc
        .create_swarm(
            &ws(),
            &user(),
            req(json!({"name": "From preset", "preset_slug": preset.slug})),
            "claude",
        )
        .await
        .unwrap();
    instantiate(
        &svc.repo,
        &s,
        &user(),
        &preset.slug,
        &["claude".to_string()],
        "claude",
    )
    .await
    .unwrap();

    let agents = svc.list_agents(&s.id).await.unwrap();
    assert_eq!(agents.len(), preset.agents.len());
    let by_name: HashMap<&str, &otto_state::SwarmAgent> =
        agents.iter().map(|a| (a.name.as_str(), a)).collect();
    let id_to_name: HashMap<&str, &str> = agents
        .iter()
        .map(|a| (a.id.as_str(), a.name.as_str()))
        .collect();
    let key_to_name: HashMap<&str, &str> = preset
        .agents
        .iter()
        .map(|a| (a.key.as_str(), a.name.as_str()))
        .collect();
    for pa in &preset.agents {
        let a = by_name[pa.name.as_str()];
        assert_eq!(
            a.provider, "claude",
            "unavailable template providers map to the default"
        );
        let want = pa.reports_to.as_deref().map(|k| key_to_name[k]);
        let got = a.reports_to.as_deref().map(|id| id_to_name[id]);
        assert_eq!(got, want, "{}: manager", pa.name);
    }
    let s = svc.get_swarm(&s.id).await.unwrap();
    assert_eq!(
        s.config["max_parallel_sessions"],
        json!(preset.max_parallel_sessions)
    );
    assert_eq!(s.max_total_runs, preset.max_total_runs);

    // Unknown slug: a no-op, not an error.
    let blank = svc
        .create_swarm(&ws(), &user(), req(json!({"name": "Blank"})), "claude")
        .await
        .unwrap();
    instantiate(&svc.repo, &blank, &user(), "no-such-preset", &[], "claude")
        .await
        .unwrap();
    assert!(svc.list_agents(&blank.id).await.unwrap().is_empty());
}
