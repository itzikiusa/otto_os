//! OKF (Open Knowledge Format) v0.1/v0.2 conformance — the deterministic checker
//! ("never eyeball conformance") plus per-directory `index.md` generation.
//!
//! Rules follow the public validator taxonomy over SPEC.md §9:
//!   E1 no/unparseable frontmatter · E2 missing/empty `type` · E3 reserved-file
//!   structure (index.md frontmatter — only the bundle-root index may carry
//!   one, and only `okf_version`; log.md frontmatter) · W1 missing title or
//!   description · W2 broken internal link · W3 no content-change timestamp · W4 directory
//!   without index.md · W5 log `##` headings not ISO `YYYY-MM-DD`.
//! Warnings never fail a bundle (permissive consumption is intentional).

use std::collections::BTreeSet;
use std::sync::Arc;

use otto_core::Result;

use crate::engine::VaultEngine;
use crate::types::{Heading, OkfFinding, OkfReport};

fn is_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| matches!(i, 4 | 7) || c.is_ascii_digit())
}

/// Optional families remain advisory: malformed metadata never makes otherwise
/// valid knowledge unreadable. Trust declarations are not authenticated proofs.
fn optional_metadata_warnings(fm: &serde_json::Value) -> Vec<String> {
    let mut warnings = Vec::new();
    let nonempty = |v: Option<&serde_json::Value>| v.and_then(|v| v.as_str()).is_some_and(|s| !s.trim().is_empty());
    let datetime = |v: Option<&serde_json::Value>| v.and_then(|v| v.as_str())
        .is_some_and(|s| chrono::DateTime::parse_from_rfc3339(s).is_ok());
    if let Some(generated) = fm.get("generated") {
        if !nonempty(generated.get("by")) || !datetime(generated.get("at")) {
            warnings.push("`generated` should carry nonempty `by` and an offset-bearing ISO datetime `at`".into());
        }
    }
    if let Some(verified) = fm.get("verified") {
        let events: Vec<&serde_json::Value> = match verified {
            serde_json::Value::Array(events) => events.iter().collect(),
            serde_json::Value::Object(_) => vec![verified],
            _ => vec![],
        };
        if (!verified.is_array() && !verified.is_object()) || events.iter().any(|event| !nonempty(event.get("by")) || !datetime(event.get("at"))) {
            warnings.push("`verified` should be a {by, at} mapping or list of verification mappings".into());
        }
    }
    if let Some(sources) = fm.get("sources") {
        if sources.as_array().is_none_or(|entries| entries.iter().any(|entry| !nonempty(entry.get("resource")))) {
            warnings.push("`sources` should be a list of mappings with a nonempty `resource` (path, URI or scope descriptor)".into());
        }
    }
    if let Some(status) = fm.get("status") {
        if !matches!(status.as_str(), Some("draft" | "stable" | "deprecated")) {
            warnings.push("recommended lifecycle `status` values are draft, stable or deprecated".into());
        }
    }
    if fm.get("stale_after").is_some() && !datetime(fm.get("stale_after")) {
        warnings.push("`stale_after` should be an absolute ISO datetime with a UTC offset".into());
    }
    warnings
}

impl VaultEngine {
    pub async fn okf_validate(self: &Arc<Self>, ws: &str, id: i64) -> Result<OkfReport> {
        self.get_scoped(ws, id).await?;
        self.ensure_fresh(id);
        let rows = self.store().okf_rows(id).await?;
        let mut errors: Vec<OkfFinding> = Vec::new();
        let mut warnings: Vec<OkfFinding> = Vec::new();
        let mut dirs_with_notes: BTreeSet<String> = BTreeSet::new();
        let mut dirs_with_index: BTreeSet<String> = BTreeSet::new();

        for r in &rows {
            let dir = r
                .path
                .rsplit_once('/')
                .map(|(d, _)| d.to_string())
                .unwrap_or_default();
            let fm: serde_json::Value =
                serde_json::from_str(&r.frontmatter_json).unwrap_or(serde_json::Value::Null);
            let base = r
                .path
                .rsplit('/')
                .next()
                .unwrap_or(&r.path)
                .to_ascii_lowercase();

            if r.reserved {
                if base == "index.md" {
                    dirs_with_index.insert(dir.clone());
                    if r.has_frontmatter {
                        let root_index = !r.path.contains('/');
                        let only_okf_version = fm
                            .as_object()
                            .map(|o| o.keys().all(|k| k == "okf_version"))
                            .unwrap_or(false);
                        if !root_index || !only_okf_version || r.parse_error {
                            errors.push(OkfFinding {
                                rule: "E3".into(),
                                path: r.path.clone(),
                                message: if root_index {
                                    "root index.md frontmatter may only carry okf_version".into()
                                } else {
                                    "index.md must not have frontmatter".into()
                                },
                            });
                        }
                    }
                } else {
                    // log.md
                    if r.has_frontmatter {
                        errors.push(OkfFinding {
                            rule: "E3".into(),
                            path: r.path.clone(),
                            message: "log.md must not have frontmatter".into(),
                        });
                    }
                    let headings: Vec<Heading> =
                        serde_json::from_str(&r.headings_json).unwrap_or_default();
                    for h in headings.iter().filter(|h| h.level == 2) {
                        if !is_iso_date(h.text.trim()) {
                            warnings.push(OkfFinding {
                                rule: "W5".into(),
                                path: r.path.clone(),
                                message: format!(
                                    "log heading `## {}` is not an ISO date (YYYY-MM-DD)",
                                    h.text.trim()
                                ),
                            });
                        }
                    }
                }
                continue;
            }

            dirs_with_notes.insert(dir);
            // E1 — parseable YAML frontmatter mapping is REQUIRED on concepts.
            if !r.has_frontmatter || r.parse_error {
                errors.push(OkfFinding {
                    rule: "E1".into(),
                    path: r.path.clone(),
                    message: if r.has_frontmatter {
                        "frontmatter is not parseable YAML".into()
                    } else {
                        "missing YAML frontmatter block".into()
                    },
                });
                continue; // E2/W* are noise on a file that already fails E1
            }
            // E2 — non-empty `type`.
            if r.okf_type
                .as_deref()
                .map(str::trim)
                .unwrap_or("")
                .is_empty()
            {
                errors.push(OkfFinding {
                    rule: "E2".into(),
                    path: r.path.clone(),
                    message: "missing required frontmatter field `type`".into(),
                });
            }
            // W1 — recommended title/description (never `resource`).
            let has = |k: &str| {
                fm.get(k)
                    .and_then(|v| v.as_str())
                    .map(|s| !s.trim().is_empty())
                    .unwrap_or(false)
            };
            if !has("title") || !has("description") {
                warnings.push(OkfFinding {
                    rule: "W1".into(),
                    path: r.path.clone(),
                    message: "missing recommended `title` and/or `description`".into(),
                });
            }
            // v0.2 generated.at takes precedence; timestamp is a v0.1 fallback.
            let changed_at = if fm.get("generated").is_some() {
                fm.get("generated").and_then(|v| v.get("at"))
            } else { fm.get("timestamp") };
            if changed_at.and_then(serde_json::Value::as_str).is_none_or(|s| s.trim().is_empty()) {
                warnings.push(OkfFinding {
                    rule: "W3".into(),
                    path: r.path.clone(),
                    message: "missing `generated.at` (or legacy `timestamp`) for the last content change".into(),
                });
            }
            for message in optional_metadata_warnings(&fm) {
                warnings.push(OkfFinding { rule: "W6".into(), path: r.path.clone(), message });
            }
        }

        // W2 — broken internal links (permitted, worth noting).
        for (src, raw) in self.store().all_ghost_edges(id).await? {
            warnings.push(OkfFinding {
                rule: "W2".into(),
                path: src,
                message: format!("broken link → `{raw}` (may be not-yet-written knowledge)"),
            });
        }

        // W4 — every directory holding concepts should have an index.md.
        for d in &dirs_with_notes {
            if !dirs_with_index.contains(d) {
                warnings.push(OkfFinding {
                    rule: "W4".into(),
                    path: if d.is_empty() { "/".into() } else { d.clone() },
                    message: "directory has no index.md (progressive disclosure)".into(),
                });
            }
        }

        Ok(OkfReport {
            conformant: errors.is_empty(),
            errors,
            warnings,
            checked_notes: rows.len() as i64,
        })
    }

    /// Regenerate `index.md` in every directory that contains concepts:
    /// heading-grouped bullet lists carrying each concept's one-sentence
    /// frontmatter description (SPEC §6). Root index keeps `okf_version`.
    /// Returns the number of index files written.
    pub async fn okf_indexes(self: &Arc<Self>, ws: &str, id: i64) -> Result<usize> {
        let v = self.get_scoped(ws, id).await?;
        let notes = self.store().all_notes(id).await?;
        // Existing bundles keep their declared version, including future ones.
        // Regenerating an index is not consent to migrate or downgrade a bundle.
        let root = tokio::fs::read_to_string(std::path::Path::new(&v.root_path).join("index.md")).await.ok();
        let version = root.as_deref().map(crate::parse::parse_note)
            .and_then(|n| n.frontmatter.get("okf_version").cloned())
            .filter(|v| v.is_string() || v.is_number())
            .unwrap_or_else(|| serde_json::json!("0.2"));
        let mut dirs: BTreeSet<String> = BTreeSet::new();
        for (p, _, _, reserved) in &notes {
            if *reserved {
                continue;
            }
            dirs.insert(
                p.rsplit_once('/')
                    .map(|(d, _)| d.to_string())
                    .unwrap_or_default(),
            );
        }
        // Parent dirs of concept dirs also get an index (subdirectory bullets).
        let all_dirs: BTreeSet<String> = dirs
            .iter()
            .flat_map(|d| {
                let mut acc = vec![String::new()];
                let mut cur = String::new();
                for seg in d.split('/').filter(|s| !s.is_empty()) {
                    cur = if cur.is_empty() {
                        seg.to_string()
                    } else {
                        format!("{cur}/{seg}")
                    };
                    acc.push(cur.clone());
                }
                acc
            })
            .collect();

        let mut written = 0usize;
        for dir in &all_dirs {
            let child_notes = self.store().dir_notes(id, dir).await?;
            let child_dirs: BTreeSet<String> = all_dirs
                .iter()
                .filter(|d| {
                    if dir.is_empty() {
                        !d.is_empty() && !d.contains('/')
                    } else {
                        d.strip_prefix(&format!("{dir}/"))
                            .is_some_and(|rest| !rest.is_empty() && !rest.contains('/'))
                    }
                })
                .cloned()
                .collect();
            if child_notes.is_empty() && child_dirs.is_empty() {
                continue;
            }
            let mut md = String::new();
            let is_root = dir.is_empty();
            if is_root {
                md.push_str(&format!("---\nokf_version: {version}\n---\n\n"));
            }
            let dir_title = if is_root {
                v.name.clone()
            } else {
                dir.rsplit('/').next().unwrap_or(dir).to_string()
            };
            md.push_str(&format!("# {dir_title}\n"));
            if !child_notes.is_empty() {
                md.push('\n');
                for (p, title, desc) in &child_notes {
                    let fname = p.rsplit('/').next().unwrap_or(p);
                    let d = desc.clone().unwrap_or_default();
                    let tail = if d.is_empty() {
                        String::new()
                    } else {
                        format!(" - {d}")
                    };
                    md.push_str(&format!(
                        "* [{title}]({}){tail}\n",
                        crate::parse::percent_encode_spaces(fname)
                    ));
                }
            }
            if !child_dirs.is_empty() {
                md.push_str("\n# Subdirectories\n\n");
                for d in &child_dirs {
                    let name = d.rsplit('/').next().unwrap_or(d);
                    md.push_str(&format!("* [{name}]({name}/index.md)\n"));
                }
            }
            let rel = if is_root {
                "index.md".to_string()
            } else {
                format!("{dir}/index.md")
            };
            let abs = std::path::Path::new(&v.root_path).join(&rel);
            tokio::fs::write(&abs, md)
                .await
                .map_err(|e| otto_core::Error::Internal(format!("write {rel}: {e}")))?;
            written += 1;
        }
        self.scan(id).await?;
        Ok(written)
    }
}
