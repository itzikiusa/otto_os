//! Antigravity swarm members with no card get one when the thread's agent
//! (re)starts. agy resumes with `session/resume`, which replays no history,
//! so a member added before an agent restart — or before Otto carded swarm
//! members — is known only to agy's own board on disk
//! (`brain/<session>/.system_generated/swarm`, `native_work::agy_swarm`).
//! It is read ONCE per start (no poll), and each member without a card gets
//! the same subagent item a new member gets (`acp.rs` `swarm_from`). A
//! refused resume falls back to `session/load`, whose replay the driver
//! cards itself.

use otto_core::agent_runtime::RuntimeProvider;

use super::*;

/// The cards for `members` (name, killed) that `items` has none for.
pub(super) fn unknown_members<'a>(
    items: impl Iterator<Item = &'a TurnItem>,
    members: &[(String, bool)],
) -> Vec<TurnItem> {
    let known: std::collections::HashSet<&str> = items
        .filter_map(|i| match &i.kind {
            TurnItemKind::Subagent {
                tool_name,
                description: Some(n),
                ..
            } if tool_name == "swarm" => Some(n.as_str()),
            _ => None,
        })
        .collect();
    members
        .iter()
        .filter(|(n, _)| !known.contains(n.as_str()))
        .map(|(name, killed)| {
            crate::driver::item(
                format!("board:swarm:{name}"),
                if *killed {
                    TurnItemStatus::Interrupted
                } else {
                    TurnItemStatus::Running
                },
                Some(name.clone()),
                TurnItemKind::Subagent {
                    tool_name: "swarm".into(),
                    description: Some(name.clone()),
                    subagent_type: Some("Swarm worker".into()),
                    prompt: None,
                    output: None,
                    background: !killed,
                    last_tool: None,
                    tool_uses: 0,
                },
            )
        })
        .collect()
}

impl AgentRuntime {
    /// After an Antigravity thread's agent started: card the swarm members
    /// agy's board lists that this thread has no card for.
    pub(super) async fn card_swarm_members(&self, live: &Arc<Live>) {
        let native = {
            let st = live.lock();
            if st.session.provider != RuntimeProvider::Agy {
                return;
            }
            st.session.native_thread_id.clone()
        };
        let (Some(native), Some(brain)) = (native, crate::acp_sideband::default_agy_brain_dir())
        else {
            return;
        };
        let brain = std::path::PathBuf::from(brain);
        let Some(swarm) = tokio::task::spawn_blocking(move || {
            crate::native_work::agy_swarm(&brain, &native, None)
        })
        .await
        .ok()
        .flatten() else {
            return;
        };
        let members: Vec<(String, bool)> = swarm
            .members
            .into_iter()
            .map(|m| (m.name, m.killed))
            .collect();
        let mut st = live.lock();
        for it in unknown_members(st.proj.items.values(), &members) {
            upsert_item(&self.inner, live, &mut st, it);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_members_without_a_card_get_one() {
        let card = crate::driver::item(
            "t1:swarm:echoer".into(),
            TurnItemStatus::Running,
            None,
            TurnItemKind::Subagent {
                tool_name: "swarm".into(),
                description: Some("echoer".into()),
                subagent_type: None,
                prompt: None,
                output: None,
                background: true,
                last_tool: None,
                tool_uses: 0,
            },
        );
        let made = unknown_members(
            [card].iter(),
            &[
                ("echoer".into(), false),
                ("old-1".into(), false),
                ("old-2".into(), true),
            ],
        );
        let names: Vec<(String, TurnItemStatus)> =
            made.iter().map(|i| (i.id.to_string(), i.status)).collect();
        assert_eq!(
            names,
            [
                ("board:swarm:old-1".to_string(), TurnItemStatus::Running),
                ("board:swarm:old-2".to_string(), TurnItemStatus::Interrupted)
            ]
        );
        assert!(made[0].is_background_open());
    }
}
