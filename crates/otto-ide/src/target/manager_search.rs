//! Workspace file listing, multi-mode search, and multi-file replace methods
//! on [`super::super::TargetManager`].

use std::collections::BTreeMap;
use std::time::Duration;

use otto_core::{Error, Result};

use crate::types::{IdeReplaceReq, IdeReplaceResp, IdeSearchReq, IdeSearchResp};

use super::super::search::{
    build_matcher, collect_local_files_multi, replace_local_sync, replace_remote_ssh,
    search_local_sync, search_remote_ssh,
};
use super::super::{
    join_remote, ResolvedTarget, TargetManager, DEFAULT_SEARCH_LIMIT, MAX_SEARCH_LIMIT,
};

impl TargetManager {
    /// List workspace source file paths across primary + attached roots.
    pub async fn list_files(&self, target: &ResolvedTarget, limit: usize) -> Result<Vec<String>> {
        let cap = limit.clamp(1, 20_000);
        match target {
            ResolvedTarget::Local {
                root,
                additional_roots,
                is_root_user,
                ..
            } => {
                let root_clone = root.clone();
                let extra_clone = additional_roots.clone();
                let restricted = !*is_root_user;
                tokio::task::spawn_blocking(move || {
                    collect_local_files_multi(&root_clone, &extra_clone, restricted, None, cap)
                })
                .await
                .map_err(|e| Error::Internal(format!("list_files join: {e}")))?
            }
            ResolvedTarget::Ssh {
                root,
                additional_roots,
                ..
            } => {
                let mut files = Vec::new();
                for scan_root in std::iter::once(root.as_str())
                    .chain(additional_roots.iter().map(String::as_str))
                {
                    let rem = cap.saturating_sub(files.len());
                    if rem == 0 {
                        break;
                    }
                    let script = format!(
                        "if command -v rg >/dev/null 2>&1; then rg --files | head -n {rem}; \
                         else find . -maxdepth 8 -type f -not -path '*/.*' -not -path '*/node_modules/*' -not -path './target/*' | head -n {rem}; fi"
                    );
                    let out = self
                        .exec_shell(
                            target,
                            &script,
                            Some(scan_root),
                            &BTreeMap::new(),
                            Duration::from_secs(15),
                        )
                        .await?;
                    for rel in out
                        .stdout
                        .lines()
                        .map(|l| l.trim().trim_start_matches("./"))
                        .filter(|l| !l.is_empty())
                    {
                        if files.len() >= cap {
                            break;
                        }
                        files.push(join_remote(scan_root, rel));
                    }
                }
                Ok(files)
            }
        }
    }

    /// Fast multi-mode workspace search (`files`, `text`, `symbols`, `all`).
    pub async fn search(
        &self,
        target: &ResolvedTarget,
        req: &IdeSearchReq,
    ) -> Result<IdeSearchResp> {
        let query = req.query.trim().to_string();
        if query.is_empty() {
            return Ok(IdeSearchResp {
                items: Vec::new(),
                truncated: false,
            });
        }
        let limit = req
            .limit
            .unwrap_or(DEFAULT_SEARCH_LIMIT)
            .clamp(1, MAX_SEARCH_LIMIT);
        match target {
            ResolvedTarget::Local {
                root,
                additional_roots,
                is_root_user,
                ..
            } => {
                let root_clone = root.clone();
                let extra_clone = additional_roots.clone();
                let restricted = !*is_root_user;
                let req_clone = req.clone();
                tokio::task::spawn_blocking(move || {
                    search_local_sync(&root_clone, &extra_clone, restricted, &req_clone, limit)
                })
                .await
                .map_err(|e| Error::Internal(format!("search join: {e}")))?
            }
            ResolvedTarget::Ssh {
                root,
                additional_roots,
                ..
            } => search_remote_ssh(self, target, root, additional_roots, req, limit).await,
        }
    }

    /// Multi-file search & replace with `dry_run` diff preview and atomic writes.
    pub async fn replace(
        &self,
        target: &ResolvedTarget,
        req: &IdeReplaceReq,
    ) -> Result<IdeReplaceResp> {
        if req.query.trim().is_empty() {
            return Err(Error::Invalid("replace query must not be empty".into()));
        }
        let matcher = build_matcher(&req.query, req.regex, req.case_sensitive, req.whole_word)?;
        match target {
            ResolvedTarget::Local {
                root,
                additional_roots,
                is_root_user,
                ..
            } => {
                let root_clone = root.clone();
                let extra_clone = additional_roots.clone();
                let restricted = !*is_root_user;
                let req_clone = req.clone();
                let lsp_pool = self.lsp_pool.clone();
                tokio::task::spawn_blocking(move || {
                    replace_local_sync(
                        &root_clone,
                        &extra_clone,
                        restricted,
                        &req_clone,
                        &matcher,
                        lsp_pool.as_deref(),
                    )
                })
                .await
                .map_err(|e| Error::Internal(format!("replace join: {e}")))?
            }
            ResolvedTarget::Ssh {
                root,
                additional_roots,
                ..
            } => replace_remote_ssh(self, target, root, additional_roots, req, &matcher).await,
        }
    }
}
