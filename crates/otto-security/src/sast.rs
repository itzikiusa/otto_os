use std::collections::BTreeMap;
use std::path::Path;

use otto_core::{Error, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use crate::download::{io_error, upstream};
use crate::engine::{fresh, read_json, write_json, BASE_EXCLUDED_DIRS};
use crate::incremental::Selection;
use crate::report::{relative_path, text, MAX_FINDINGS};
use crate::{Check, Finding, ScanSettings, Severity};

const REGISTRY: &str = "https://semgrep.dev/c/p/default";

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Rules {
    pub(crate) digest: String,
    checked_at: String,
    config: Value,
    descriptions: BTreeMap<String, Description>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Description {
    message: String,
    fix: String,
}

pub(crate) async fn refresh_rules(
    client: &Client,
    dir: &Path,
    cancel: &CancellationToken,
) -> Result<Rules> {
    let path = dir.join("community-rules.json");
    let current: Option<Rules> = read_json(&path).await;
    if current
        .as_ref()
        .is_some_and(|rules| fresh(&rules.checked_at, 24))
    {
        return Ok(current.expect("cached rules"));
    }
    let fetch = async {
        let mut response = client
            .get(REGISTRY)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|_| upstream("The community security rules could not be downloaded."))?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| upstream("Security rule download failed."))?
        {
            if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
                return Err(upstream(
                    "The community security rules exceeded their size cap.",
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        tokio::task::spawn_blocking(move || compile_rules(&bytes))
            .await
            .map_err(|_| Error::Internal("security rules worker stopped".into()))?
    };
    let result = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err(crate::governor::cancelled()),
        result = fetch => result,
    };
    match result {
        Ok(rules) => {
            write_json(&path, &rules).await?;
            Ok(rules)
        }
        Err(error) => current.ok_or(error),
    }
}

fn compile_rules(bytes: &[u8]) -> Result<Rules> {
    #[cfg(target_os = "macos")]
    let _background = crate::download::BackgroundThread::enter();

    let pack: Value = serde_yaml::from_slice(bytes)
        .map_err(|_| upstream("The community security rules were unreadable."))?;
    let rules = pack["rules"]
        .as_array()
        .ok_or_else(|| upstream("No community security rules were returned."))?;
    let mut retained: Vec<_> = rules
        .iter()
        .filter(|rule| {
            let category = rule["metadata"]["category"].as_str();
            let confidence = rule["metadata"]["confidence"].as_str().unwrap_or_default();
            category == Some("security")
                && rule["languages"].as_array().is_some_and(|languages| {
                    languages.iter().any(|lang| {
                        if matches!(lang.as_str(), Some("c" | "cpp")) {
                            return !confidence.eq_ignore_ascii_case("low");
                        }
                        confidence.eq_ignore_ascii_case("high")
                            && matches!(
                                lang.as_str(),
                                Some(
                                    "c" | "cpp"
                                        | "csharp"
                                        | "C#"
                                        | "java"
                                        | "javascript"
                                        | "typescript"
                                        | "go"
                                        | "golang"
                                        | "python"
                                        | "rust"
                                        | "kotlin"
                                        | "kt"
                                        | "js"
                                        | "ts"
                                )
                            )
                    })
                })
        })
        .cloned()
        .collect();
    if retained.is_empty() {
        return Err(upstream(
            "No matching community security rules were returned.",
        ));
    }
    retained.push(json!({
        "id": "otto.zig.runtime-safety-disabled",
        "languages": ["generic"],
        "severity": "WARNING",
        "message": "This Zig code explicitly disables runtime safety checks.",
        "paths": {"include": ["*.zig"]},
        "pattern": "@setRuntimeSafety(false)"
    }));
    retained.push(json!({
        "id": "otto.cpp.unbounded-input",
        "languages": ["cpp"],
        "severity": "ERROR",
        "message": "gets reads unchecked input into a fixed-size buffer.",
        "pattern-either": [{"pattern": "gets(...)"}, {"pattern": "std::gets(...)"}],
        "fix": "Use std::getline for strings or a bounded input API."
    }));
    let mut descriptions = BTreeMap::new();
    for rule in &mut retained {
        if let Some(obj) = rule.as_object_mut() {
            // This product runs per-file analysis. Never enable custom or interfile options
            // from a remotely maintained ruleset implicitly.
            obj.remove("options");
        }
        if let Some(id) = rule["id"].as_str() {
            descriptions.insert(
                id.to_owned(),
                Description {
                    message: text(
                        rule["message"]
                            .as_str()
                            .unwrap_or("Review the reported code."),
                        900,
                    ),
                    fix: text(
                        rule["fix"]
                            .as_str()
                            .unwrap_or("Review the matched line and follow the rule guidance."),
                        600,
                    ),
                },
            );
        }
    }
    let config = json!({"rules": retained});
    Ok(Rules {
        digest: hex::encode(Sha256::digest(crate::store::encode(&config)?.as_bytes())),
        checked_at: crate::store::now(),
        config,
        descriptions,
    })
}

pub(crate) async fn args(
    root: &Path,
    report: &Path,
    output: &Path,
    rules: &Rules,
    settings: &ScanSettings,
    selection: &Selection,
) -> Result<Vec<String>> {
    let config = output.join("rules.json");
    tokio::fs::write(&config, crate::store::encode(&rules.config)?)
        .await
        .map_err(io_error)?;
    let mut args = vec![
        "scan".into(),
        format!("--project-root={}", root.display()),
        format!("--config={}", config.display()),
        "--no-rewrite-rule-ids".into(),
        "--no-git-ignore".into(),
        "--no-progress-bar".into(),
        "--quiet".into(),
        "--json".into(),
        format!("--output={}", report.display()),
        format!("--jobs={}", settings.parallelism),
        format!("--max-memory={}", settings.memory_mib),
        "--timeout=5".into(),
        "--max-target-bytes=262144".into(),
        "--disable-nosem".into(),
    ];
    for excluded in BASE_EXCLUDED_DIRS
        .iter()
        .map(|s| s.to_string())
        .chain(settings.excludes.iter().cloned())
    {
        args.push(format!("--exclude={excluded}"));
    }
    if let Selection::Files(files) = selection {
        args.extend(files.iter().map(|path| format!("--include=/{path}")));
    }
    args.push(root.to_string_lossy().into_owned());
    Ok(args)
}

pub(crate) fn parse(bytes: &[u8], root: &Path, rules: &Rules) -> Result<Vec<Finding>> {
    let report: Value = serde_json::from_slice(bytes)
        .map_err(|_| Error::internal_public("Opengrep returned an unreadable report."))?;
    let matches = report["results"]
        .as_array()
        .ok_or_else(|| Error::internal_public("Opengrep did not return findings."))?;
    if matches.len() > MAX_FINDINGS {
        return Err(Error::internal_public(
            "This scan produced too many findings. Narrow its scope and retry.",
        ));
    }
    Ok(matches
        .iter()
        .filter_map(|item| {
            let path = relative_path(item["path"].as_str()?, root)?;
            let rule = item["check_id"].as_str()?;
            let description = rules.descriptions.get(rule)?;
            let severity = match item["extra"]["severity"].as_str() {
                Some("ERROR" | "HIGH") => Severity::High,
                Some("WARNING" | "MEDIUM") => Severity::Medium,
                Some("INFO" | "LOW") => Severity::Low,
                _ => Severity::Unknown,
            };
            let line = item["start"]["line"].as_u64().filter(|line| *line > 0);
            let root = root.to_string_lossy().into_owned();
            let identity = format!(
                "opengrep\\0{root}\\0{path}\\0{rule}\\0{line:?}\\0{}",
                item["start"]["col"]
            );
            Some(Finding {
                fingerprint: hex::encode(Sha256::digest(identity.as_bytes())),
                engine: "opengrep".into(),
                check: Check::Sast,
                severity,
                root,
                path,
                line,
                rule: rule.into(),
                title: text(&description.message, 240),
                description: description.message.clone(),
                fix: description.fix.clone(),
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiled_rules_remove_options_and_findings_use_only_static_descriptions() {
        let pack = json!({"rules": [
            {"id":"known", "languages":["rust"], "metadata":{"category":"security","confidence":"HIGH"},
                "message":"Static guidance", "fix":"Static fix", "options":{"interfile":true}},
            {"id":"style", "languages":["rust"], "metadata":{"category":"style","confidence":"HIGH"}}
        ]});
        let rules = compile_rules(&serde_json::to_vec(&pack).unwrap()).unwrap();
        let selected = rules.config["rules"].as_array().unwrap();
        assert!(selected.iter().all(|rule| rule.get("options").is_none()));
        assert!(!rules.descriptions.contains_key("style"));

        let report = json!({"results":[
            {"check_id":"known", "path":"src/lib.rs", "start":{"line":4,"col":2},
                "extra":{"message":"Matched input", "fix":"Matched replacement", "severity":"ERROR"}},
            {"check_id":"style", "path":"src/lib.rs"},
            {"check_id":"known", "path":"../outside.rs"}
        ]});
        let findings = parse(
            &serde_json::to_vec(&report).unwrap(),
            Path::new("/project"),
            &rules,
        )
        .unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, Some(4));
        assert_eq!(findings[0].severity, Severity::High);
        assert_eq!(findings[0].description, "Static guidance");
        assert_eq!(findings[0].fix, "Static fix");
    }
}
