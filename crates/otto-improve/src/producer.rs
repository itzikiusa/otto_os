//! Run the analysis agent and parse its proposal. claude is driven via the
//! orchestrator's interactive PTY path (the same real-interactive-claude path
//! the PR review agents use); other providers (codex/agy) run headlessly via
//! the CLI's non-interactive mode. Tests inject a fake producer.

use std::sync::Arc;
use std::time::Duration;

use otto_core::auth::BoxFuture;
use otto_core::{Error, Result};
use otto_orchestrator::Orchestrator;
use otto_orchestrator::PromptArg;

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
/// program name and the args that precede the prompt. claude is driven via the
/// orchestrator's interactive PTY path instead (it reads the reply from the
/// session transcript), so it is handled separately in `run_one`.
///
/// codex has a first-class `exec` subcommand; agy is a claude-compatible fork
/// so it uses claude's `-p` print mode. For ANY OTHER provider — including a
/// user's CUSTOM provider (e.g. `grok`) — we BEST-EFFORT the claude-compatible
/// `-p` print convention (most agent CLIs, and every claude fork, accept it).
/// A provider that doesn't support it simply exits non-zero; the engine logs
/// and skips it, never aborting the others — so a custom provider is offered
/// consistently and works whenever its CLI has a print mode.
fn headless_exec(provider: &str) -> (String, Vec<String>, PromptArg<'static>) {
    match provider {
        "codex" => (
            "codex".into(),
            vec![
                "exec".into(),
                "--dangerously-bypass-approvals-and-sandbox".into(),
                "--skip-git-repo-check".into(),
            ],
            PromptArg::Positional,
        ),
        // agy's `-p`/`--print` is a pflag STRING flag (`agy --help`): it takes
        // the prompt as its value, so `-p --dangerously-skip-permissions …`
        // made the permissions flag the prompt. Flags first, prompt inlined.
        "agy" => (
            "agy".into(),
            vec!["--dangerously-skip-permissions".into()],
            PromptArg::Flag("--print"),
        ),
        // Every other (custom) provider: the claude-style print switch.
        other => (
            other.to_string(),
            vec!["-p".into(), "--dangerously-skip-permissions".into()],
            PromptArg::Positional,
        ),
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
            self.orchestrator
                .run_agent(prompt, cwd, None, self.timeout)
                .await
        } else {
            // codex/agy/custom → headless CLI exec (best-effort print mode for
            // custom providers; a non-supporting CLI errors and is skipped).
            let (program, args, prompt_arg) = headless_exec(provider);
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            otto_orchestrator::run_cli_exec_with(
                &program,
                &arg_refs,
                prompt_arg,
                prompt,
                cwd,
                self.timeout,
            )
            .await
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
            tracing::warn!(
                provider,
                "self-improvement: first proposal unparseable; retrying once"
            );
            // Attempt 2: append a stricter reminder.
            let strict = format!(
                "{prompt}\n\nIMPORTANT: your previous reply was not valid JSON. Reply with \
                 ONLY the JSON object, no prose, no markdown fence."
            );
            let reply = self.run_one(&strict, cwd, provider).await?;
            parse_proposal(&reply).map_err(|e| {
                Error::Upstream(format!(
                    "analysis agent ({provider}) returned no valid proposal: {e}"
                ))
            })
        })
    }
}
