//! The lists an agent reports about itself that a thread shows whole: its
//! slash commands (`commands.updated`) and its MCP servers
//! (`mcp_servers.updated`) — what the spawn gave it, refined by what the CLI
//! reports.

use otto_core::agent_runtime::{McpServerStatus, ProviderCommand, RuntimeEventBody};

use super::{emit, Inner, Live, State};
use crate::driver::{DriverEvent, DriverStep};

/// An agent's command list as the session keeps it: at most 400 commands,
/// unique by name, each description one line of at most 160 characters
/// (Claude Code lists every skill with its whole description).
pub(super) fn bounded_commands(list: Vec<ProviderCommand>) -> Vec<ProviderCommand> {
    let mut seen = std::collections::HashSet::new();
    list.into_iter()
        .filter(|c| !c.name.trim().is_empty() && seen.insert(c.name.clone()))
        .take(400)
        .map(|mut c| {
            let line = c
                .description
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            c.description = match line.char_indices().nth(160) {
                Some((i, _)) => format!("{}…", line[..i].trim_end()),
                None => line,
            };
            c
        })
        .collect()
}

/// The spawn's MCP list (the host's `mcp_status`) ahead of everything the
/// driver's start emits: a new process starts from what Otto gave it.
pub(super) fn with_spawn_mcp(mut step: DriverStep, servers: Vec<McpServerStatus>) -> DriverStep {
    step.events.insert(
        0,
        DriverEvent::McpServers {
            servers,
            spawn: true,
        },
    );
    step
}

/// `spawn`: the list Otto gave the new process, whole. Otherwise what the
/// CLI reports, merged in by name (see [`merge_mcp`]).
pub(super) fn on_mcp(
    inner: &Inner,
    live: &Live,
    st: &mut State,
    servers: Vec<McpServerStatus>,
    spawn: bool,
) {
    let next = if spawn {
        servers.into_iter().take(MAX_MCP).collect()
    } else {
        merge_mcp(&st.proj.mcp_servers, servers)
    };
    if next != st.proj.mcp_servers {
        emit(
            inner,
            live,
            st,
            RuntimeEventBody::McpServersUpdated { mcp_servers: next },
        );
    }
}

const MAX_MCP: usize = 100;

/// A reported server updates the entry of the same name (its state and
/// reason; where it came from stays what the spawn said), or joins the list
/// (a server from the CLI's own config).
pub(super) fn merge_mcp(
    current: &[McpServerStatus],
    reported: Vec<McpServerStatus>,
) -> Vec<McpServerStatus> {
    let mut out = current.to_vec();
    for r in reported {
        if let Some(s) = out.iter_mut().find(|s| s.name == r.name) {
            s.state = r.state;
            s.reason = r.reason;
        } else if out.len() < MAX_MCP {
            out.push(r);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_core::agent_runtime::{McpServerSource as Src, McpServerState as St};

    #[test]
    fn a_report_refines_the_spawn_list_by_name_and_adds_the_clis_own() {
        let spawn = vec![
            McpServerStatus::new("otto", Src::Otto, St::Added),
            McpServerStatus::new("jira", Src::Workspace, St::Added),
            McpServerStatus::new("bad", Src::Workspace, St::Unsupported).because("no"),
        ];
        let merged = merge_mcp(
            &spawn,
            vec![
                McpServerStatus::new("otto", Src::Cli, St::Connected),
                McpServerStatus::new("jira", Src::Cli, St::NeedsAuth).because("sign in"),
                McpServerStatus::new("coralogix", Src::Cli, St::Failed).because("exit 1"),
            ],
        );
        let view: Vec<_> = merged
            .iter()
            .map(|s| (s.name.as_str(), s.source, s.state, s.reason.as_deref()))
            .collect();
        assert_eq!(
            view,
            vec![
                ("otto", Src::Otto, St::Connected, None),
                ("jira", Src::Workspace, St::NeedsAuth, Some("sign in")),
                ("bad", Src::Workspace, St::Unsupported, Some("no")),
                ("coralogix", Src::Cli, St::Failed, Some("exit 1")),
            ]
        );
    }
}
