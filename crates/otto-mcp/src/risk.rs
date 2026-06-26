//! Tool risk labeling (control-plane requirement 7).
//!
//! Each discovered tool gets a **risk label** (read | write | dangerous) and a
//! **prompt-injection risk** (low | medium | high), derived from the MCP
//! `toolAnnotations` (when the server provides them) refined by name/description
//! keyword heuristics. Unlabeled tools fail closed (write + medium). A human can
//! pin labels afterwards (`risk_overridden`), and rediscovery never lowers a
//! pinned label.

use serde_json::Value;

/// The labels produced for one tool.
pub struct Labels {
    pub risk_label: String,    // "read" | "write" | "dangerous"
    pub injection_risk: String, // "low" | "medium" | "high"
    pub mutating: bool,
    pub supports_dry_run: bool,
}

const DANGEROUS_KW: &[&str] = &[
    "delete", "drop", "remove", "destroy", "exec", "execute", "deploy", "kill",
    "terminate", "purge", "wipe", "truncate", "shutdown", "revoke", "payment",
    "charge", "transfer", "send_money", "rm_",
];
const WRITE_KW: &[&str] = &[
    "create", "update", "write", "set_", "put_", "post_", "send", "edit", "modify",
    "insert", "upload", "patch", "rename", "move", "add_", "publish", "merge",
];
const INJECTION_KW: &[&str] = &[
    "fetch", "browse", "web", "url", "http", "search", "read_url", "scrape",
    "crawl", "download", "open_url", "request", "email", "inbox", "rss", "feed",
];

fn ann_bool(annotations: &Value, key: &str) -> Option<bool> {
    annotations.get(key).and_then(Value::as_bool)
}

/// Label a tool from its annotations + name + description.
pub fn label_tool(name: &str, description: Option<&str>, annotations: &Value) -> Labels {
    let hay = format!(
        "{} {}",
        name.to_lowercase(),
        description.unwrap_or("").to_lowercase()
    );

    // --- risk label / mutating --------------------------------------------
    let read_only_hint = ann_bool(annotations, "readOnlyHint");
    let destructive_hint = ann_bool(annotations, "destructiveHint");
    let kw_dangerous = DANGEROUS_KW.iter().any(|k| hay.contains(k));
    let kw_write = WRITE_KW.iter().any(|k| hay.contains(k));

    let (risk_label, mutating) = if destructive_hint == Some(true) || kw_dangerous {
        ("dangerous", true)
    } else if read_only_hint == Some(true) {
        // Server says read-only; trust it for read but keyword can still bump.
        if kw_dangerous {
            ("dangerous", true)
        } else {
            ("read", false)
        }
    } else if kw_write || read_only_hint == Some(false) {
        ("write", true)
    } else {
        // No signal at all → fail closed (write + medium injection below).
        ("write", true)
    };

    // --- injection risk ----------------------------------------------------
    // openWorldHint=true => the tool reaches untrusted external content; that is
    // the classic prompt-injection vector → at least high.
    let open_world = ann_bool(annotations, "openWorldHint") == Some(true);
    let kw_injection = INJECTION_KW.iter().any(|k| hay.contains(k));
    let injection_risk = if open_world {
        "high"
    } else if kw_injection {
        "medium"
    } else if read_only_hint == Some(true) && !kw_injection {
        "low"
    } else {
        "medium"
    };

    Labels {
        risk_label: risk_label.to_string(),
        injection_risk: injection_risk.to_string(),
        mutating,
        // MCP has no standard machine-checked dry-run affordance; we never trust a
        // server's claim to one (a lie would let a "dry-run" execute a mutation).
        // Dry-run is always a pure local simulation. See design §14 F4.
        supports_dry_run: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn read_only_hint_is_read_low() {
        let l = label_tool("get_weather", Some("returns the forecast"), &json!({"readOnlyHint": true}));
        assert_eq!(l.risk_label, "read");
        assert!(!l.mutating);
        assert_eq!(l.injection_risk, "low");
    }

    #[test]
    fn destructive_hint_is_dangerous() {
        let l = label_tool("cleanup", None, &json!({"destructiveHint": true}));
        assert_eq!(l.risk_label, "dangerous");
        assert!(l.mutating);
    }

    #[test]
    fn delete_keyword_is_dangerous_even_without_annotations() {
        let l = label_tool("delete_record", None, &json!({}));
        assert_eq!(l.risk_label, "dangerous");
        assert!(l.mutating);
    }

    #[test]
    fn open_world_hint_is_high_injection() {
        let l = label_tool("call_api", None, &json!({"openWorldHint": true, "readOnlyHint": true}));
        assert_eq!(l.injection_risk, "high");
        assert_eq!(l.risk_label, "read");
    }

    #[test]
    fn fetch_keyword_is_medium_injection() {
        let l = label_tool("fetch_url", Some("download a web page"), &json!({"readOnlyHint": true}));
        assert_eq!(l.injection_risk, "medium");
    }

    #[test]
    fn unlabeled_fails_closed_write_medium() {
        let l = label_tool("frobnicate", None, &json!({}));
        assert_eq!(l.risk_label, "write");
        assert!(l.mutating);
        assert_eq!(l.injection_risk, "medium");
        assert!(!l.supports_dry_run);
    }
}
