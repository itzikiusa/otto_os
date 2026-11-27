use std::path::{Path, PathBuf};

use otto_core::{Error, Result};
use otto_state::DbPool;
use percent_encoding::{percent_decode_str, utf8_percent_encode, NON_ALPHANUMERIC};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::report::{relative_path, text, MAX_FINDINGS};
use crate::store::{db_error, encode, now};
use crate::{Check, Finding, ScanDetail, Severity};

pub fn export(scan: &ScanDetail, roots: &[PathBuf]) -> Value {
    let results: Vec<_> = scan.findings.iter().filter_map(|finding| {
        let root = roots.iter().position(|root| root == Path::new(&finding.root))?;
        let uri = finding.path.split('/').map(|part| utf8_percent_encode(part, NON_ALPHANUMERIC).to_string())
            .collect::<Vec<_>>().join("/");
        let artifact = if finding.check == Check::Dast {
            json!({"uri": finding.path})
        } else {
            json!({"uri": uri, "uriBaseId": format!("ROOT_{root}")})
        };
        let mut physical = json!({"artifactLocation": artifact});
        if let Some(line) = finding.line {
            physical["region"] = json!({"startLine": line});
        }
        let level = match finding.severity {
            Severity::Critical | Severity::High => "error",
            Severity::Medium => "warning",
            Severity::Low | Severity::Unknown => "note",
        };
        let mut item = json!({
            "ruleId": finding.rule, "level": level,
            "message": {"text": finding.description},
            "locations": [{"physicalLocation": physical}],
            "partialFingerprints": {"otto/v1": finding.fingerprint},
            "properties": {"check": finding.check, "engine": finding.engine, "fix": finding.fix},
        });
        if let Some(reason) = scan.annotations.suppressions.get(&finding.fingerprint) {
            item["suppressions"] = json!([{"kind": "external", "justification": reason}]);
        }
        Some(item)
    }).collect();
    json!({
        "version": "2.1.0",
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "runs": [{"tool": {"driver": {"name": "Otto"}}, "results": results}],
    })
}

pub async fn import(
    pool: &DbPool,
    project: &str,
    roots: &[PathBuf],
    payload: Value,
) -> Result<String> {
    let findings = parse(&payload, roots)?;
    let id = otto_core::new_id();
    let timestamp = now();
    sqlx::query(
        "INSERT INTO ide_security_scans (id, project_id, status, created_at, finished_at, findings, count)
         VALUES (?, ?, 'completed', ?, ?, ?, ?)",
    ).bind(&id).bind(project).bind(&timestamp).bind(&timestamp).bind(encode(&findings)?)
        .bind(findings.len() as u32).execute(pool).await.map_err(db_error)?;
    Ok(id)
}

fn parse(data: &Value, roots: &[PathBuf]) -> Result<Vec<Finding>> {
    if data["version"].as_str() != Some("2.1.0") {
        return Err(Error::Invalid("Import a SARIF 2.1.0 report.".into()));
    }
    let runs = data["runs"]
        .as_array()
        .ok_or_else(|| Error::Invalid("The SARIF report has no runs.".into()))?;
    let mut findings = Vec::new();
    for item in runs
        .iter()
        .filter_map(|run| run["results"].as_array())
        .flatten()
    {
        if findings.len() >= MAX_FINDINGS {
            return Err(Error::PayloadTooLarge(
                "The SARIF report exceeds 10,000 findings.".into(),
            ));
        }
        if let Some(finding) = import_finding(item, roots) {
            findings.push(finding);
        }
    }
    findings.sort_unstable_by(|a, b| {
        a.severity
            .cmp(&b.severity)
            .then_with(|| a.fingerprint.cmp(&b.fingerprint))
    });
    findings.dedup_by(|a, b| a.fingerprint == b.fingerprint);
    Ok(findings)
}

fn import_finding(item: &Value, roots: &[PathBuf]) -> Option<Finding> {
    let physical = &item["locations"][0]["physicalLocation"];
    let artifact = &physical["artifactLocation"];
    let base = artifact["uriBaseId"]
        .as_str()
        .and_then(|value| value.strip_prefix("ROOT_"))
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let root = roots.get(base)?;
    let read_check = serde_json::from_value::<Check>(item["properties"]["check"].clone()).ok();
    let raw_uri = artifact["uri"].as_str()?;
    let path = if read_check == Some(Check::Dast) {
        crate::passive::parse_url(raw_uri).ok()?.0.to_string()
    } else {
        let uri = percent_decode_str(raw_uri).decode_utf8().ok()?;
        if Path::new(uri.as_ref()).is_absolute()
            || uri.contains([':', '\\'])
            || uri.chars().any(char::is_control)
        {
            return None;
        }
        relative_path(&uri, root)?
    };
    let root = root.to_string_lossy().into_owned();
    let rule = text(item["ruleId"].as_str().unwrap_or("sarif"), 180);
    let secret = read_check == Some(Check::Secret)
        || ["secret", "credential", "token", "password"]
            .iter()
            .any(|word| rule.to_lowercase().contains(word));
    let check = if secret {
        Check::Secret
    } else {
        read_check.unwrap_or(Check::Sast)
    };
    let line = physical["region"]["startLine"]
        .as_u64()
        .filter(|line| *line > 0);
    let identity = format!("sarif\\0{root}\\0{path}\\0{rule}\\0{line:?}");
    let severity = match item["level"].as_str() {
        Some("error") => Severity::High,
        Some("warning") => Severity::Medium,
        Some("note") => Severity::Low,
        _ => Severity::Unknown,
    };
    let (description, fix) = if secret {
        (
            "Potential secret. The matched text is masked.".into(),
            "Remove the credential, rotate it if real, and use a secret store.".into(),
        )
    } else {
        (
            text(
                item["message"]["text"]
                    .as_str()
                    .unwrap_or("Review the reported location."),
                900,
            ),
            text(
                item["properties"]["fix"]
                    .as_str()
                    .unwrap_or("Review the rule guidance."),
                600,
            ),
        )
    };
    Some(Finding {
        fingerprint: hex::encode(Sha256::digest(identity.as_bytes())),
        engine: "sarif".into(),
        check,
        severity,
        root,
        path,
        line,
        title: rule.clone(),
        rule,
        description,
        fix,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_filters_outside_locations_and_discards_secret_text() {
        let data = json!({"version":"2.1.0", "runs":[{"results":[
            {"ruleId":"secret", "message":{"text":"raw credential"}, "locations":[
                {"physicalLocation":{"artifactLocation":{"uri":"src%20config.env"}, "region":{"startLine":8}}}]},
            {"ruleId":"escape", "locations":[{"physicalLocation":{"artifactLocation":{"uri":"%2e%2e/private"}}}]}
        ]}]});
        let findings = parse(&data, &[PathBuf::from("/project")]).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].path, "src config.env");
        assert_eq!(findings[0].line, Some(8));
        assert!(!encode(&findings).unwrap().contains("raw credential"));
    }
}
