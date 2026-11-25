//! Which UI spans the daemon accepts at `POST /telemetry/ingest`: a closed
//! set of operation names and components, so a browser batch never carries
//! arbitrary UI text (project names, routes) into telemetry.
use crate::SpanRecord;
use anyhow::{bail, Result};

pub fn validate_ui_batch(spans: &[SpanRecord]) -> Result<()> {
    if spans.len() > 100 {
        bail!("A telemetry batch may contain at most 100 spans");
    }
    for span in spans {
        span.validate()?;
        if !matches!(
            span.name.as_str(),
            "ui.navigation"
                | "ui.chunk"
                | "ui.render"
                | "ui.long_task"
                | "ui.frame_delay"
                | "http.client"
                | "ui.request.queue"
                | "ui.response.decode"
                | "ui.thread.open"
                | "ui.thread.send"
                | "ui.highlight"
        ) || !matches!(span.kind.as_str(), "client" | "internal")
            || !UI_COMPONENTS.contains(&span.component.as_str())
            || span.attributes.contains_key("http.route")
        {
            bail!("Unsupported UI telemetry operation");
        }
    }
    Ok(())
}
const UI_COMPONENTS: &[&str] = &[
    "agents",
    "threads",
    "ide",
    "workbench",
    "connections",
    "history",
    "assistant",
    "home",
    "git",
    "database",
    "api",
    "vault",
    "mission-control",
    "rooms",
    "workflows",
    "scheduled-tasks",
    "personal-agents",
    "loops",
    "swarm",
    "product",
    "design",
    "canvas",
    "browser",
    "run-with-otto",
    "mcp",
    "brokers",
    "kubernetes",
    "containers",
    "aws",
    "insights",
    "usage",
    "skills-eval",
    "proof",
    "settings",
    "walkthroughs",
    "plugin",
    "snip",
    "shell",
    "other",
];

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn producer_contract_accepts_mixed_batch_and_rejects_private_names() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../fixtures/ui-ingest.json")).unwrap();
        let mut spans = vec![
            SpanRecord::new("ui.navigation", "git", 12.0),
            SpanRecord::new("ui.render", "git", 2.0),
            SpanRecord::new("ui.navigation", "threads", 8.0),
            SpanRecord::new("ui.navigation", "ide", 9.0),
            SpanRecord::new("ui.thread.open", "threads", 24.0),
            SpanRecord::new("ui.thread.send", "threads", 3.0),
            SpanRecord::new("ui.highlight", "ide", 4.0),
        ];
        for request in fixture["requests"].as_array().unwrap() {
            let mut span = SpanRecord::new(
                request["name"].as_str().unwrap(),
                request["component"].as_str().unwrap(),
                1.0,
            );
            span.kind = "client".into();
            spans.push(span);
        }
        assert!(validate_ui_batch(&spans).is_ok());
        for name in fixture["rejected_names"].as_array().unwrap() {
            spans.last_mut().unwrap().name = name.as_str().unwrap().into();
            assert!(validate_ui_batch(&spans).is_err(), "accepted {name}");
        }
        spans.last_mut().unwrap().name = "x".repeat(97);
        assert!(validate_ui_batch(&spans).is_err());
        spans.last_mut().unwrap().name = "http.client".into();
        spans.last_mut().unwrap().component = "private-project".into();
        assert!(validate_ui_batch(&spans).is_err());
    }
    /// Every component the UI can send — each sidebar module and each entry
    /// of its TELEMETRY_COMPONENTS — is accepted; one missing id failed the
    /// whole batch with 400 on every visit to that page.
    #[test]
    fn every_ui_module_component_is_accepted() {
        let between = |src: &'static str, from: &str, to: &str| {
            let start = src.find(from).expect(from) + from.len();
            &src[start..start + src[start..].find(to).expect(to)]
        };
        let sidebar = between(
            include_str!("../../../ui/src/lib/sidebar.ts"),
            "SIDEBAR_MODULES:",
            "\n];",
        );
        let runtime = include_str!("../../../ui/src/lib/telemetryRuntime.ts");
        let modules: Vec<&str> = sidebar
            .split("id: '")
            .skip(1)
            .filter_map(|s| s.split('\'').next())
            .collect();
        let components: Vec<&str> = between(runtime, "TELEMETRY_COMPONENTS = [", "]")
            .split('\'')
            .skip(1)
            .step_by(2)
            .collect();
        assert!(modules.len() > 15 && modules.contains(&"containers"));
        assert!(components.contains(&"usage") && components.contains(&"other"));
        for c in modules.iter().chain(&components) {
            let span = SpanRecord::new("ui.navigation", *c, 1.0);
            assert!(
                validate_ui_batch(&[span]).is_ok(),
                "UI component {c} rejected"
            );
        }
    }
    #[test]
    fn ingestion_rejects_dynamic_names_and_routes_atomically() {
        let mut span = SpanRecord::new("ui.navigation", "agents", 12.0);
        assert!(validate_ui_batch(&[span.clone()]).is_ok());
        span.name = "private-project-name".into();
        assert!(validate_ui_batch(&[span.clone()]).is_err());
        span.name = "ui.navigation".into();
        span.attributes
            .insert("http.route".into(), "/secrets".into());
        assert!(validate_ui_batch(&[span]).is_err());
        assert!(
            validate_ui_batch(&vec![SpanRecord::new("ui.navigation", "agents", 1.0); 101]).is_err()
        );
    }
}
