//! Bind a force-push decision to immutable source and destination identities.
//! Only local reads happen before confirmation; the final command cannot follow
//! a later checkout, commit, upstream edit or background tracking-ref update.

use crate::local::{push_err, AskPass, LocalGit, SpawnClass};
use otto_core::{Error, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushTarget {
    pub branch: String,
    pub source_sha: String,
    pub remote: String,
    pub destination_ref: String,
    /// Hash instead of a URL: Git URLs can contain credentials.
    pub destination_hash: String,
    pub remote_sha: Option<String>,
}

fn changed() -> Error {
    Error::Conflict(
        "push target changed — refresh the repository and confirm the new source and destination"
            .into(),
    )
}

impl LocalGit {
    /// Publish only the revision covered by evidence. Branch movement while
    /// credentials are loaded cannot substitute an unverified commit. Explicit
    /// destination and flags suppress configured extra refs/tags/submodules;
    /// ordinary fast-forward rules apply (this never forces remote history).
    pub async fn push_revision(
        &self,
        token: Option<String>,
        revision: &str,
        branch: &str,
    ) -> Result<()> {
        if !matches!(revision.len(), 40 | 64)
            || !revision.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(Error::Invalid(
                "publication requires a full commit ID".into(),
            ));
        }
        let destination = format!("refs/heads/{branch}");
        self.run_read(&["check-ref-format", &destination]).await?;
        let refspec = format!("{revision}:{destination}");
        let askpass = token.as_deref().map(AskPass::new).transpose()?;
        let envs = askpass.as_ref().map(AskPass::envs).unwrap_or_default();
        let (ok, out, err, code) = self
            .run_raw_class(
                &[
                    "push",
                    "--no-follow-tags",
                    "--no-mirror",
                    "--recurse-submodules=no",
                    "--",
                    "origin",
                    &refspec,
                ],
                &envs,
                SpawnClass::Remote,
            )
            .await?;
        if ok {
            Ok(())
        } else {
            Err(push_err(&err, &out, code))
        }
    }

    async fn push_config(&self, key: &str) -> Result<Option<String>> {
        let (ok, out, err, code) = self
            .run_raw_class(&["config", "--get", key], &[], SpawnClass::LocalRead)
            .await?;
        if ok {
            Ok(Some(out.trim().to_string()))
        } else if code == Some(1) {
            Ok(None)
        } else {
            Err(Error::Upstream(format!(
                "cannot read push configuration: {err}"
            )))
        }
    }

    /// Read one branch's ordinary single-ref push destination. Ambiguous or
    /// multi-ref modes can still use normal push, but never the force dialog.
    pub async fn push_target(&self) -> Result<PushTarget> {
        Ok(self.push_target_with_url().await?.0)
    }

    async fn push_target_with_url(&self) -> Result<(PushTarget, String)> {
        // A resolved URL handed back to `git push` can be rewritten a second
        // time. Do not offer this force flow when URL rewriting is configured.
        let (rewrites, _, _, code) = self
            .run_raw_class(
                &[
                    "config",
                    "--get-regexp",
                    r"^url\..*\.(insteadof|pushinsteadof)$",
                ],
                &[],
                SpawnClass::LocalRead,
            )
            .await?;
        if rewrites || code != Some(1) {
            return Err(Error::Conflict("force retry cannot pin a URL with Git URL rewriting configured — review and push with your Git client".into()));
        }
        let branch = self.current_branch().await?;
        if branch == "HEAD" {
            return Err(Error::Conflict(
                "check out a branch before force pushing".into(),
            ));
        }
        let source_ref = format!("refs/heads/{branch}");
        let source_sha = self
            .run_read(&["rev-parse", "--verify", &source_ref])
            .await?
            .trim()
            .to_string();
        let upstream_remote = self.push_config(&format!("branch.{branch}.remote")).await?;
        let remote = self
            .push_config(&format!("branch.{branch}.pushRemote"))
            .await?
            .or(self.push_config("remote.pushDefault").await?)
            .or(upstream_remote.clone())
            .unwrap_or_else(|| "origin".into());
        let mode = self
            .push_config("push.default")
            .await?
            .unwrap_or_else(|| "simple".into());
        if self
            .push_config(&format!("remote.{remote}.push"))
            .await?
            .is_some()
            || !matches!(mode.as_str(), "simple" | "current" | "upstream")
            || self
                .push_config(&format!("remote.{remote}.mirror"))
                .await?
                .is_some_and(|s| s != "false")
        {
            return Err(Error::Conflict("force retry requires a single branch push destination; review the remote push configuration".into()));
        }
        let merge = self.push_config(&format!("branch.{branch}.merge")).await?;
        let destination_ref = match mode.as_str() {
            "upstream" if upstream_remote.as_deref() == Some(&remote) => {
                merge.clone().ok_or_else(changed)?
            }
            "upstream" => return Err(changed()),
            "simple"
                if upstream_remote.as_deref() == Some(&remote)
                    && merge.as_deref().is_some_and(|r| r != source_ref) =>
            {
                return Err(changed())
            }
            _ => source_ref.clone(),
        };
        if !destination_ref.starts_with("refs/heads/") {
            return Err(changed());
        }
        self.run_read(&["check-ref-format", &destination_ref])
            .await?;
        let urls = self
            .run_read(&["remote", "get-url", "--push", "--all", &remote])
            .await?;
        let urls: Vec<_> = urls.lines().filter(|s| !s.is_empty()).collect();
        if urls.len() != 1 {
            return Err(Error::Conflict(
                "force retry requires exactly one push URL".into(),
            ));
        }
        let url = urls[0].to_string();
        let destination_hash = hex::encode(Sha256::digest(url.as_bytes()));
        // Reverse the fetch mapping to locate this destination's tracking ref.
        // Custom namespaces are supported; missing/negative mappings fail closed
        // when force is requested because no observed remote OID is available.
        let (ok, mappings, _, _) = self
            .run_raw_class(
                &["config", "--get-all", &format!("remote.{remote}.fetch")],
                &[],
                SpawnClass::LocalRead,
            )
            .await?;
        let mut tracking = None;
        if ok {
            for mapping in mappings.lines() {
                let Some((src, dst)) = mapping.trim_start_matches('+').split_once(':') else {
                    continue;
                };
                if src == destination_ref {
                    tracking = Some(dst.to_string());
                    break;
                }
                if let (Some((sp, ss)), Some((dp, ds))) = (src.split_once('*'), dst.split_once('*'))
                {
                    if let Some(middle) = destination_ref
                        .strip_prefix(sp)
                        .and_then(|v| v.strip_suffix(ss))
                    {
                        tracking = Some(format!("{dp}{middle}{ds}"));
                        break;
                    }
                }
            }
        }
        let remote_sha = match tracking {
            Some(r) => {
                let (ok, out, _, _) = self
                    .run_raw_class(
                        &["rev-parse", "-q", "--verify", &r],
                        &[],
                        SpawnClass::LocalRead,
                    )
                    .await?;
                ok.then(|| out.trim().to_string())
            }
            None => None,
        };
        Ok((
            PushTarget {
                branch,
                source_sha,
                remote,
                destination_ref,
                destination_hash,
                remote_sha,
            },
            url,
        ))
    }

    /// Prepare exact argv without sending anything. An explicit lease binds the
    /// remote OID; SHA:ref and a resolved URL bind the source and destination even
    /// if an external CLI changes refs/config after this method returns.
    pub(crate) async fn force_push_args(&self, expected: &PushTarget) -> Result<Vec<String>> {
        let (fresh, url) = self.push_target_with_url().await.map_err(|_| changed())?;
        if &fresh != expected {
            return Err(changed());
        }
        let remote_sha = fresh.remote_sha.as_deref().ok_or_else(|| {
            Error::Conflict("fetch and review the destination branch before force pushing".into())
        })?;
        // Git ignores --force-if-includes with an explicit OID lease. Preserve
        // its integration protection ourselves: the observed remote commit must
        // be reachable from this branch's recent reflog (including pre-rewrite
        // tips), not merely fetched by a background worker. Bound argv and fail
        // closed if the relevant integration is older than the retained window.
        let reflog = self
            .run_read(&[
                "reflog",
                "show",
                "-n",
                "256",
                "--format=%H",
                &format!("refs/heads/{}", fresh.branch),
            ])
            .await?;
        let mut args = vec![
            "rev-list",
            "--max-count=1",
            remote_sha,
            "--not",
            &fresh.source_sha,
        ];
        args.extend(reflog.lines().filter(|s| !s.is_empty()));
        if !self.run_read(&args).await?.trim().is_empty() {
            return Err(Error::Conflict("force push refused: the fetched remote commits were never integrated into this branch — review and integrate them first".into()));
        }
        Ok(vec![
            "push".into(),
            "--no-follow-tags".into(),
            "--no-mirror".into(),
            "--recurse-submodules=no".into(),
            format!(
                "--force-with-lease={}:{}",
                fresh.destination_ref, remote_sha
            ),
            "--".into(),
            url,
            format!("{}:{}", fresh.source_sha, fresh.destination_ref),
        ])
    }

    pub async fn push_confirmed(&self, token: Option<String>, expected: &PushTarget) -> Result<()> {
        let args = self.force_push_args(expected).await?;
        let askpass = token.as_deref().map(AskPass::new).transpose()?;
        let envs = askpass.as_ref().map(AskPass::envs).unwrap_or_default();
        let args: Vec<_> = args.iter().map(String::as_str).collect();
        let (ok, out, err, code) = self.run_raw_class(&args, &envs, SpawnClass::Remote).await?;
        if ok {
            Ok(())
        } else {
            Err(push_err(&err, &out, code))
        }
    }
}

#[cfg(test)]
#[path = "push_tests.rs"]
mod tests;
