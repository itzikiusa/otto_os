//! Code review: the run engine, the summarizer, per-agent retry / stop, the
//! lifecycle transitions and the HTTP routes. The pure core (prompts, diff
//! render, draft parsing, budgets) lives in `otto_review::engine`.

use super::*;

mod lifecycle;
mod retry;
mod routes;
mod run;
mod summarize;

pub(crate) use self::lifecycle::*;
pub(crate) use self::retry::*;
pub(crate) use self::routes::*;
pub(crate) use self::run::*;
pub(crate) use self::summarize::*;

/// The review API the workflow / run / automation engines call. Import from
/// here (or the matching `crate::modules::…` re-export), not from the
/// engine's internals.
pub(crate) mod service {
    pub(crate) use super::lifecycle::cancel_running_review;
    pub(crate) use super::run::{
        default_review_provider, effective_review_mode, review_finding_briefs,
        review_findings_counts, review_open_counts_by_severity, review_wait_budget,
    };
}
