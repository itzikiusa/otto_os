//! Local and remote SSH path normalization, validation, and error mapping
//! helpers for [`super::TargetManager`].

use std::path::{Path, PathBuf};

use otto_core::{Error, Result};
use otto_ssh::{valid_proxy_jump, valid_ssh_host, valid_ssh_user};

use crate::types::{IdeSshSpec, IdeTargetKind, IdeTargetSpec};

use super::fs_ops::{enforce_non_root_path, resolve_local_existing_in_roots};
use super::sdk_paths::{
    collect_readonly_sdk_roots, discover_local_go_replace_roots, is_virtual_archive_uri,
    resolve_local_readable, strip_file_uri,
};
use super::{ResolvedTarget, TargetManager};

pub(crate) fn canonicalize_local_root_dir(raw_root: &str, is_root_user: bool) -> Result<PathBuf> {
    let decoded = strip_file_uri(raw_root);
    let root_input = if !decoded.is_empty() && Path::new(&decoded).exists() {
        decoded.as_str()
    } else {
        decoded.trim()
    };
    let expanded = otto_core::paths::expand_home_strict(root_input).map_err(Error::Invalid)?;
    let canonical = expanded
        .canonicalize()
        .map_err(|e| map_io_err("resolve workspace root", &expanded, e))?;
    let meta = std::fs::metadata(&canonical)
        .map_err(|e| map_io_err("stat workspace root", &canonical, e))?;
    if !meta.is_dir() {
        return Err(Error::Invalid(format!(
            "target root '{}' is not a directory",
            canonical.display()
        )));
    }
    if otto_core::secret_paths::is_denied_dir(&canonical) {
        return Err(Error::Forbidden(format!(
            "{} holds protected credentials; access denied",
            canonical.display()
        )));
    }
    if !is_root_user {
        enforce_non_root_path(&canonical)?;
    }
    Ok(canonical)
}

pub(crate) fn validate_ssh_spec(spec: &IdeSshSpec) -> Result<()> {
    if !valid_ssh_host(&spec.host) {
        return Err(Error::Invalid(format!(
            "SSH host '{}' is not a valid hostname or IP",
            spec.host
        )));
    }
    if !valid_ssh_user(&spec.user) {
        return Err(Error::Invalid(format!(
            "SSH user '{}' is not a valid username",
            spec.user
        )));
    }
    if let Some(id_file) = spec
        .identity_file
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if id_file.starts_with('-') || id_file.chars().any(char::is_control) {
            return Err(Error::Invalid(
                "SSH identity_file contains unsafe characters".into(),
            ));
        }
        let _ = otto_core::paths::expand_home_strict(id_file).map_err(Error::Invalid)?;
    }
    if let Some(jump) = spec.proxy_jump.as_deref().filter(|s| !s.trim().is_empty()) {
        if !valid_proxy_jump(jump) {
            return Err(Error::Invalid(format!(
                "SSH proxy_jump '{jump}' is invalid"
            )));
        }
    }
    Ok(())
}

pub(crate) fn valid_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub(crate) fn posix_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

pub(crate) async fn resolve_local_prospective_in_roots(
    root: &Path,
    additional_roots: &[PathBuf],
    is_root_user: bool,
    raw: &str,
    create_dirs: bool,
) -> Result<(PathBuf, PathBuf)> {
    let decoded = strip_file_uri(raw);
    if decoded.is_empty() || decoded == "." {
        return Err(Error::Invalid("file path must not be empty".into()));
    }
    let abs = if decoded.starts_with('~') {
        otto_core::paths::expand_home_strict(&decoded).map_err(Error::Invalid)?
    } else {
        let p = Path::new(&decoded);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            root.join(p)
        }
    };
    let mut prospective = otto_core::secret_paths::canonicalize_prospective(&abs)
        .ok_or_else(|| Error::Invalid(format!("invalid target path '{}'", abs.display())))?;

    let matched_root = std::iter::once(root)
        .chain(additional_roots.iter().map(PathBuf::as_path))
        .find(|r| prospective.starts_with(r))
        .ok_or_else(|| {
            Error::Forbidden(format!(
                "path '{}' escapes workspace root '{}'",
                prospective.display(),
                root.display()
            ))
        })?
        .to_path_buf();

    if !is_root_user {
        enforce_non_root_path(&prospective)?;
    }
    if create_dirs {
        if let Some(parent) = prospective.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| map_io_err("create parent directories", parent, e))?;
            if let Ok(canon_parent) = parent.canonicalize() {
                if !canon_parent.starts_with(&matched_root) {
                    return Err(Error::Forbidden(
                        "parent directory escapes workspace root".into(),
                    ));
                }
                if !is_root_user {
                    enforce_non_root_path(&canon_parent)?;
                }
                if let Some(name) = prospective.file_name() {
                    prospective = canon_parent.join(name);
                }
            }
        }
    }
    Ok((prospective, matched_root))
}

pub(crate) fn normalize_posix_clean_root(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    lexical_normalize_remote(trimmed).unwrap_or_else(|_| {
        let stripped = trimmed.trim_end_matches('/');
        if stripped.is_empty() && trimmed.starts_with('/') {
            "/".into()
        } else {
            stripped.into()
        }
    })
}

pub(crate) fn normalize_remote_root(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.chars().any(char::is_control) {
        return Err(Error::Invalid("invalid remote root path".into()));
    }
    if !trimmed.starts_with('/') && !trimmed.starts_with('~') {
        return Err(Error::Invalid(
            "remote SSH root path must start with '/' or '~'".into(),
        ));
    }
    lexical_normalize_remote(trimmed)
}

pub(crate) fn resolve_remote_in_roots(
    root: &str,
    additional_roots: &[String],
    raw: &str,
) -> Result<(String, String)> {
    let decoded = strip_file_uri(raw);
    if decoded.chars().any(char::is_control) {
        return Err(Error::Invalid(
            "remote path contains control characters".into(),
        ));
    }
    if decoded.is_empty() || decoded == "." {
        return Ok((root.to_string(), root.to_string()));
    }
    let combined = if decoded.starts_with('/') || decoded.starts_with('~') {
        decoded
    } else {
        join_remote(root, &decoded)
    };
    let normalized = lexical_normalize_remote(&combined)?;
    for candidate_root in std::iter::once(root).chain(additional_roots.iter().map(String::as_str)) {
        let clean_root = normalize_posix_clean_root(candidate_root);
        if clean_root == "/"
            || normalized == clean_root
            || normalized.starts_with(&format!("{clean_root}/"))
        {
            return Ok((normalized, clean_root));
        }
    }
    Err(Error::Forbidden(format!(
        "remote path '{normalized}' is outside workspace root '{root}'"
    )))
}

pub(crate) fn resolve_remote_in_roots_or_sdk(
    root: &str,
    additional_roots: &[String],
    raw: &str,
) -> Result<(String, Option<String>)> {
    if let Ok((p, r)) = resolve_remote_in_roots(root, additional_roots, raw) {
        return Ok((p, Some(r)));
    }
    let decoded = strip_file_uri(raw);
    if decoded.is_empty() {
        return Err(Error::Invalid("remote path must not be empty".into()));
    }
    let normalized = lexical_normalize_remote(&decoded)?;
    let is_remote_sdk = [
        "/usr/local/go/",
        "/usr/lib/go",
        "/usr/share/go",
        "/usr/lib/python",
        "/usr/lib/jvm/",
        "/.otto/toolchains/",
        "/go/pkg/mod/",
        "/.cargo/registry/",
        "/.rustup/toolchains/",
    ]
    .iter()
    .any(|prefix| normalized.contains(prefix));
    if is_remote_sdk {
        return Ok((normalized, None));
    }
    Err(Error::Forbidden(format!(
        "remote path '{normalized}' is outside workspace root '{root}'"
    )))
}

fn lexical_normalize_remote(path: &str) -> Result<String> {
    let is_tilde = path.starts_with('~');
    let mut stack: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if stack.is_empty() || (is_tilde && stack.len() == 1) {
                    return Err(Error::Forbidden("path traversal above root refused".into()));
                }
                stack.pop();
            }
            other => stack.push(other),
        }
    }
    if is_tilde {
        Ok(stack.join("/"))
    } else {
        Ok(format!("/{}", stack.join("/")))
    }
}

pub(crate) fn join_remote(base: &str, rel: &str) -> String {
    let b = base.trim_end_matches('/');
    let r = rel.trim_start_matches('/');
    if b.is_empty() {
        format!("/{r}")
    } else {
        format!("{b}/{r}")
    }
}

pub(crate) fn remote_parent(path: &str) -> Option<String> {
    let clean = path.trim_end_matches('/');
    if clean.is_empty() || clean == "/" || clean == "~" {
        return None;
    }
    match clean.rsplit_once('/') {
        Some(("", _)) => Some("/".into()),
        Some((parent, _)) => Some(parent.into()),
        None => None,
    }
}

pub(crate) fn map_io_err(action: &str, path: &Path, err: std::io::Error) -> Error {
    let msg = format!("cannot {action} '{}': {err}", path.display());
    match err.kind() {
        std::io::ErrorKind::NotFound => Error::NotFound(msg),
        std::io::ErrorKind::PermissionDenied => Error::Forbidden(msg),
        std::io::ErrorKind::AlreadyExists => Error::Conflict(msg),
        _ => Error::Invalid(msg),
    }
}

impl TargetManager {
    /// Resolve a local target directory, expanding `~`, canonicalizing symlinks,
    /// and enforcing `otto_core::secret_paths` for non-root callers.
    pub fn resolve_local(
        &self,
        spec: &IdeTargetSpec,
        is_root_user: bool,
    ) -> Result<ResolvedTarget> {
        self.resolve_local_with_sdks(spec, is_root_user, &[])
    }

    pub fn resolve_local_with_sdks(
        &self,
        spec: &IdeTargetSpec,
        is_root_user: bool,
        extra_sdk_dirs: &[PathBuf],
    ) -> Result<ResolvedTarget> {
        self.resolve_local_with_sdks_for_actor(spec, is_root_user, "local", extra_sdk_dirs)
    }

    pub fn resolve_local_with_sdks_for_actor(
        &self,
        spec: &IdeTargetSpec,
        is_root_user: bool,
        actor_id: &str,
        extra_sdk_dirs: &[PathBuf],
    ) -> Result<ResolvedTarget> {
        spec.validate()?;
        let canonical = canonicalize_local_root_dir(&spec.root, is_root_user)?;
        let extra_list = spec.merged_extra_roots();
        let mut additional_roots = Vec::with_capacity(extra_list.len());
        for raw_extra in &extra_list {
            let trimmed = raw_extra.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(canon_extra) = canonicalize_local_root_dir(trimmed, is_root_user) {
                if canon_extra != canonical && !additional_roots.contains(&canon_extra) {
                    additional_roots.push(canon_extra);
                }
            }
        }
        let mut sdk_inputs = extra_sdk_dirs.to_vec();
        if is_root_user {
            for go_rep in discover_local_go_replace_roots(&canonical, &additional_roots, true) {
                if go_rep != canonical
                    && !additional_roots.contains(&go_rep)
                    && !sdk_inputs.contains(&go_rep)
                {
                    sdk_inputs.push(go_rep);
                }
            }
        }
        let readonly_roots = collect_readonly_sdk_roots(&sdk_inputs);
        Ok(ResolvedTarget::Local {
            root: canonical,
            additional_roots,
            readonly_roots,
            is_root_user,
            actor_id: actor_id.to_string(),
        })
    }

    /// Resolve a remote SSH target with full hostname/user/jump validation.
    pub fn resolve_with_ssh(
        &self,
        spec: &IdeTargetSpec,
        actor_id: &str,
        ssh: IdeSshSpec,
    ) -> Result<ResolvedTarget> {
        spec.validate()?;
        validate_ssh_spec(&ssh)?;
        let norm_root = normalize_remote_root(&spec.root)?;
        let mut additional_roots = Vec::new();
        for extra in &spec.merged_extra_roots() {
            if let Ok(nr) = normalize_remote_root(extra) {
                if nr != norm_root && !additional_roots.contains(&nr) {
                    additional_roots.push(nr);
                }
            }
        }
        Ok(ResolvedTarget::Ssh {
            spec: ssh,
            root: norm_root,
            additional_roots,
            actor_id: actor_id.to_string(),
        })
    }

    /// Validate that `raw_path` resolves inside `target`'s primary or attached roots
    /// (and passes non-root secret-path checks when `!is_root_user`).
    pub async fn validate_subpath(
        &self,
        target: &IdeTargetSpec,
        raw_path: &str,
        is_root_user: bool,
    ) -> Result<()> {
        match target.kind {
            IdeTargetKind::Local => {
                let mgr = self.clone();
                let spec_clone = target.clone();
                let raw_owned = raw_path.to_string();
                let existing_check = tokio::task::spawn_blocking(
                    move || -> Result<Option<(PathBuf, Vec<PathBuf>)>> {
                        let resolved = mgr.resolve_local(&spec_clone, is_root_user)?;
                        let ResolvedTarget::Local {
                            root,
                            additional_roots,
                            ..
                        } = resolved
                        else {
                            return Err(Error::Invalid("expected local target".into()));
                        };
                        match resolve_local_existing_in_roots(
                            &root,
                            &additional_roots,
                            is_root_user,
                            &raw_owned,
                        ) {
                            Ok(_) => Ok(None),
                            Err(Error::NotFound(_)) => Ok(Some((root, additional_roots))),
                            Err(err) => Err(err),
                        }
                    },
                )
                .await
                .map_err(|e| Error::Internal(format!("validate_subpath join: {e}")))??;

                if let Some((root, additional_roots)) = existing_check {
                    let _ = resolve_local_prospective_in_roots(
                        &root,
                        &additional_roots,
                        is_root_user,
                        raw_path,
                        false,
                    )
                    .await?;
                }
                Ok(())
            }
            IdeTargetKind::Ssh => {
                let root = normalize_remote_root(&target.root)?;
                let mut extra = Vec::new();
                for raw_extra in target.merged_extra_roots() {
                    if !raw_extra.trim().is_empty() {
                        extra.push(normalize_remote_root(&raw_extra)?);
                    }
                }
                let _ = resolve_remote_in_roots(&root, &extra, raw_path)?;
                Ok(())
            }
        }
    }

    /// Resolve and confine `raw_path` within an already-authorized [`ResolvedTarget`].
    /// Existing files are canonicalized (enforcing symlink + secret-path confinement);
    /// prospective (not-yet-saved) paths are validated via prospective canonicalization.
    pub async fn resolve_path(
        &self,
        target: &ResolvedTarget,
        raw_path: &str,
        create_dirs: bool,
    ) -> Result<String> {
        match target {
            ResolvedTarget::Local {
                root,
                additional_roots,
                is_root_user,
                ..
            } => {
                let root_c = root.clone();
                let extra_c = additional_roots.clone();
                let is_root = *is_root_user;
                let raw_c = raw_path.to_string();
                let existing = tokio::task::spawn_blocking(move || {
                    resolve_local_existing_in_roots(&root_c, &extra_c, is_root, &raw_c)
                })
                .await
                .map_err(|e| Error::Internal(format!("resolve_path join: {e}")))?;
                match existing {
                    Ok((p, _)) => Ok(p.to_string_lossy().into_owned()),
                    Err(Error::NotFound(_)) => {
                        let (p, _) = resolve_local_prospective_in_roots(
                            root,
                            additional_roots,
                            *is_root_user,
                            raw_path,
                            create_dirs,
                        )
                        .await?;
                        Ok(p.to_string_lossy().into_owned())
                    }
                    Err(err) => Err(err),
                }
            }
            ResolvedTarget::Ssh {
                root,
                additional_roots,
                ..
            } => {
                let (p, _) = resolve_remote_in_roots(root, additional_roots, raw_path)?;
                Ok(p)
            }
        }
    }

    /// Resolve and confine `raw_path` for read-only LSP buffer operations (`diagnostics`,
    /// `hover`, `navigation`, `code_actions`), allowing existing workspace files,
    /// allowlisted read-only SDK files, and unsaved prospective workspace buffers.
    pub async fn resolve_readable_or_prospective_path(
        &self,
        target: &ResolvedTarget,
        raw_path: &str,
    ) -> Result<String> {
        match target {
            ResolvedTarget::Local {
                root,
                additional_roots,
                readonly_roots,
                is_root_user,
                ..
            } => {
                if is_virtual_archive_uri(raw_path) {
                    return Ok(raw_path.trim().to_string());
                }
                let root_c = root.clone();
                let extra_c = additional_roots.clone();
                let ro_c = readonly_roots.clone();
                let is_root = *is_root_user;
                let raw_c = raw_path.to_string();
                let readable = tokio::task::spawn_blocking(move || {
                    resolve_local_readable(&root_c, &extra_c, &ro_c, is_root, &raw_c)
                })
                .await
                .map_err(|e| Error::Internal(format!("resolve_readable_path join: {e}")))?;
                match readable {
                    Ok((p, _)) => Ok(p.to_string_lossy().into_owned()),
                    Err(Error::NotFound(_)) => {
                        let (p, _) = resolve_local_prospective_in_roots(
                            root,
                            additional_roots,
                            *is_root_user,
                            raw_path,
                            false,
                        )
                        .await?;
                        Ok(p.to_string_lossy().into_owned())
                    }
                    Err(err) => Err(err),
                }
            }
            ResolvedTarget::Ssh {
                root,
                additional_roots,
                ..
            } => {
                let (p, _) = resolve_remote_in_roots_or_sdk(root, additional_roots, raw_path)?;
                Ok(p)
            }
        }
    }
}
