use std::path::{Component, Path};

use otto_core::{Error, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::{Check, Finding, Severity};

pub(crate) const MAX_FINDINGS: usize = 10_000;

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Report {
    #[serde(default)]
    results: Vec<Target>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Target {
    target: String,
    #[serde(default)]
    vulnerabilities: Vec<Issue>,
    #[serde(default)]
    misconfigurations: Vec<Issue>,
    #[serde(default)]
    secrets: Vec<Issue>,
    #[serde(default)]
    licenses: Vec<Issue>,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
struct Issue {
    #[serde(rename = "VulnerabilityID", alias = "ID", alias = "RuleID")]
    rule: String,
    severity: String,
    title: String,
    description: String,
    pkg_name: String,
    installed_version: String,
    fixed_version: String,
    start_line: u64,
    cause_metadata: Location,
    resolution: String,
    name: String,
    file_path: String,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
struct Location {
    start_line: u64,
}

pub(crate) fn parse_report(data: &[u8], root: &Path) -> Result<Vec<Finding>> {
    let report: Report = serde_json::from_slice(data)
        .map_err(|_| Error::internal_public("Trivy returned an unreadable report."))?;
    let mut findings = Vec::new();
    for target in report.results {
        for (check, issues) in [
            (Check::Vulnerability, target.vulnerabilities),
            (Check::Misconfiguration, target.misconfigurations),
            (Check::Secret, target.secrets),
            (Check::License, target.licenses),
        ] {
            for issue in issues {
                let source = if issue.file_path.is_empty() {
                    &target.target
                } else {
                    &issue.file_path
                };
                if let Some(path) = relative_path(source, root) {
                    if findings.len() == MAX_FINDINGS {
                        return Err(Error::internal_public(
                            "This scan produced too many findings. Narrow its scope and retry.",
                        ));
                    }
                    findings.push(normalize(root, &path, check, issue));
                }
            }
        }
    }
    Ok(findings)
}

pub(crate) fn relative_path(raw: &str, root: &Path) -> Option<String> {
    let path = Path::new(raw);
    let path = if path.is_absolute() {
        path.strip_prefix(root).ok()?
    } else {
        path
    };
    let mut segments = Vec::new();
    for part in path.components() {
        match part {
            Component::Normal(name) => segments.push(name.to_str()?),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!segments.is_empty()).then(|| segments.join("/"))
}

fn normalize(root: &Path, path: &str, check: Check, issue: Issue) -> Finding {
    let severity = match issue.severity.as_str() {
        "CRITICAL" => Severity::Critical,
        "HIGH" => Severity::High,
        "MEDIUM" => Severity::Medium,
        "LOW" => Severity::Low,
        _ => Severity::Unknown,
    };
    let line = Some(issue.start_line.max(issue.cause_metadata.start_line)).filter(|n| *n > 0);
    let rule = text(
        if check == Check::License {
            &issue.name
        } else {
            &issue.rule
        },
        180,
    );
    let root = root.to_string_lossy().into_owned();
    let identity = format!(
        "{root}\0{path}\0{check:?}\0{rule}\0{}\0{line:?}",
        issue.pkg_name
    );
    let fingerprint = hex::encode(Sha256::digest(identity.as_bytes()));
    let (title, description, fix) = match check {
        Check::Secret => (
            "Potential secret".into(),
            "Trivy matched a credential pattern. The matched text is masked.".into(),
            "Remove the credential from the file, rotate it if real, and use a secret store."
                .into(),
        ),
        Check::Vulnerability => (
            text(&issue.title, 240),
            text(
                &format!(
                    "{} {}. {}",
                    issue.pkg_name, issue.installed_version, issue.description
                ),
                900,
            ),
            if issue.fixed_version.is_empty() {
                "Check for a patched version, a mitigation, or a replacement dependency.".into()
            } else {
                format!(
                    "Update {} to {}.",
                    text(&issue.pkg_name, 160),
                    text(&issue.fixed_version, 160)
                )
            },
        ),
        Check::Misconfiguration => (
            text(&issue.title, 240),
            text(&issue.description, 900),
            text(&issue.resolution, 600),
        ),
        Check::Sast | Check::Dast => unreachable!("Trivy reports its own scan findings"),
        Check::License => (
            format!("{} license", text(&issue.name, 160)),
            format!(
                "Trivy detected this license for {}.",
                text(&issue.pkg_name, 160)
            ),
            "Check the license requirements against your project's distribution policy.".into(),
        ),
    };
    Finding {
        fingerprint,
        engine: "trivy".into(),
        check,
        severity,
        root,
        path: path.into(),
        line,
        title: if title.is_empty() {
            rule.clone()
        } else {
            title
        },
        rule,
        description,
        fix,
    }
}

pub(crate) fn text(raw: &str, max: usize) -> String {
    raw.chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .take(max)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Check, Severity};

    #[test]
    fn normalizes_findings_and_drops_all_secret_payload_fields() {
        let data = br#"{"Results":[
          {"Target":"Cargo.lock","Vulnerabilities":[{"VulnerabilityID":"CVE-2025-10000",
            "PkgName":"library","InstalledVersion":"1.0.0","FixedVersion":"1.0.1",
            "Severity":"HIGH","Title":"Library issue","Description":"A dependency needs an update."}]},
          {"Target":"infra/Dockerfile","Misconfigurations":[{"ID":"DS001","Severity":"MEDIUM",
            "Title":"Run as non-root","Description":"Avoid running as root.",
            "Resolution":"Add a USER instruction.","CauseMetadata":{"StartLine":4}}]},
          {"Target":"src/config.js","Secrets":[{"RuleID":"sample-rule","Severity":"CRITICAL",
            "StartLine":5,"Title":"RAW_SAMPLE","Match":"RAW_SAMPLE","Code":{"Lines":[{"Content":"RAW_SAMPLE"}]}}]}
        ]}"#;
        let findings = parse_report(data, Path::new("/work/project")).unwrap();
        assert_eq!(findings.len(), 3);
        let vuln = &findings[0];
        assert_eq!(vuln.check, Check::Vulnerability);
        assert_eq!(vuln.severity, Severity::High);
        assert!(vuln.fix.contains("1.0.1"));
        assert_eq!(findings[1].line, Some(4));
        assert_eq!(findings[2].check, Check::Secret);
        assert_eq!(findings[2].line, Some(5));
        assert!(!serde_json::to_string(&findings)
            .unwrap()
            .contains("RAW_SAMPLE"));
    }

    #[test]
    fn keeps_paths_relative_to_the_project() {
        let data = br#"{"Results":[
          {"Target":"/work/project/src/conf","Secrets":[{"RuleID":"ok"}]},
          {"Target":"/work/other/src/conf","Secrets":[{"RuleID":"outside"}]},
          {"Target":"../private.txt","Secrets":[{"RuleID":"escape"}]}
        ]}"#;
        let findings = parse_report(data, Path::new("/work/project")).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].path, "src/conf");
    }
}
