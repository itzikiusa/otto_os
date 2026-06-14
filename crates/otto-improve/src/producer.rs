//! Run the analysis agent and parse its proposal. claude is driven via the
//! orchestrator's interactive PTY path (the same real-interactive-claude path
//! the PR review agents use); other providers (codex/agy) run headlessly via
//! the CLI's non-interactive mode. Tests inject a fake producer.

use std::sync::Arc;
use std::time::Duration;

use otto_core::auth::BoxFuture;
use otto_core::{Error, Result};
use otto_orchestrator::Orchestrator;

use crate::proposal::{parse_proposal, ImprovementProposal};

/// Produce a parsed proposal from an assembled prompt, running it on `provider`
/// (each provider uses its own default model — no model override).
pub trait ProposalProducer: Send + Sync {
    fn produce<'a>(
        &'a self,
        prompt: &'a str,
        cwd: &'a str,
        provider: &'a str,
    ) -> BoxFuture<'a, Result<ImprovementProposal>>;
}

/// Headless (non-interactive) CLI invocation for a non-claude provider: the
/// program and the args that precede the prompt. claude is driven via the
/// orchestrator's interactive PTY path instead (it reads the reply from the
/// session transcript), so it returns `None` here.
///
/// codex has a first-class `exec` subcommand; agy is a claude-compatible fork
/// so it uses claude's `-p` print mode. A failing provider is logged and
/// skipped by the engine, never aborting the others.
fn headless_exec(provider: &str) -> Option<(&'static str, Vec<String>)> {
    match provider {
        "codex" => Some((
            "codex",
            vec![
                "exec".into(),
                "--dangerously-bypass-approvals-and-sandbox".into(),
                "--skip-git-repo-check".into(),
            ],
        )),
        "agy" => Some((
            "agy",
            vec!["-p".into(), "--dangerously-skip-permissions".into()],
        )),
        _ => None,
    }
}

/// Real producer: drives the chosen agent CLI, parses the reply, and retries
/// once on a malformed proposal.
pub struct RealProposalProducer {
    orchestrator: Arc<Orchestrator>,
    timeout: Duration,
}

impl RealProposalProducer {
    pub fn new(orchestrator: Arc<Orchestrator>) -> Self {
        Self {
            orchestrator,
            timeout: Duration::from_secs(180),
        }
    }

    /// Run one analysis turn on `provider` and return the raw reply text.
    /// claude → interactive PTY (default model); others → headless CLI exec.
    async fn run_one(&self, prompt: &str, cwd: &str, provider: &str) -> Result<String> {
        if provider == "claude" {
            // None = provider default model.
            self.orchestrator.run_agent(prompt, cwd, None, self.timeout).await
        } else if let Some((program, args)) = headless_exec(provider) {
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            otto_orchestrator::run_cli_exec(program, &arg_refs, prompt, cwd, self.timeout).await
        } else {
            Err(Error::Invalid(format!(
                "self-improvement: provider '{provider}' has no headless analysis mode"
            )))
        }
    }
}

impl ProposalProducer for RealProposalProducer {
    fn produce<'a>(
        &'a self,
        prompt: &'a str,
        cwd: &'a str,
        provider: &'a str,
    ) -> BoxFuture<'a, Result<ImprovementProposal>> {
        Box::pin(async move {
            // Attempt 1.
            let reply = self.run_one(prompt, cwd, provider).await?;
            if let Ok(p) = parse_proposal(&reply) {
                return Ok(p);
            }
            tracing::warn!(provider, "self-improvement: first proposal unparseable; retrying once");
            // Attempt 2: append a stricter reminder.
            let strict = format!(
                "{prompt}\n\nIMPORTANT: your previous reply was not valid JSON. Reply with \
                 ONLY the JSON object, no prose, no markdown fence."
            );
            let reply = self.run_one(&strict, cwd, provider).await?;
            parse_proposal(&reply).map_err(|e| {
                Error::Upstream(format!("analysis agent ({provider}) returned no valid proposal: {e}"))
            })
        })
    }
}
