//! otto-issues — issue tracking (Jira, Linear) + Confluence: HTTP router, the
//! provider clients, and the [`Tracker`] facade that picks one per account.

pub mod adf;
pub mod adf_check;
mod atlassian;
pub mod confluence;
pub mod http;
pub mod issue_create;
pub mod jira;
pub mod linear;
pub(crate) mod markdown;
pub mod md_adf;
pub mod md_storage;
pub mod storage_check;
pub mod tracker;

pub use adf::{adf_to_markdown, text_to_adf};
pub use adf_check::{adf_outline, adf_plain_text, validate_adf};
pub use confluence::{
    build_page_cql, markdown_to_storage, storage_to_markdown, storage_to_markdown_async,
    CommentWriteResp, ConfluenceClient, ConfluencePage, ConfluencePageSummary, ConfluenceSpace,
    PageComment, PageExpect, PageWriteResp,
};
pub use http::{router, IssuesCtx};
pub use issue_create::{CreateIssueReq, CreateIssueResp, IssueVerification, VerificationCheck};
pub use jira::{
    build_create_issue_body, cmp_created, parse_issue_full, CommentRef, CreatedIssue, DevBranch,
    DevCommit, DevPr, DevStatus, EditableField, FieldOption, IssueComment, IssueFull,
    JiraAttachment, JiraChangeItem, JiraChangelogEntry, JiraClient, JiraField, JiraLink,
    JiraTransition, JiraUser,
};
pub use linear::LinearClient;
pub use md_adf::markdown_to_adf;
pub use storage_check::{check_well_formed, storage_outline, storage_plain_text, validate_storage};
pub use tracker::{provider_name, require_confluence, RawQuery, Tracker};
