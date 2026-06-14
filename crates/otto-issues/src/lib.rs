//! otto-issues — Jira issue-tracking integration: HTTP router + Jira API client.

pub mod http;
pub mod jira;

pub use http::{router, IssuesCtx};
pub use jira::JiraClient;
