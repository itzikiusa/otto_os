//! Machine-readable classification of debug-session start failures.
//!
//! A start that fails (no debugger installed, no interpreter/JDK, a build
//! error) used to reach the UI as one raw `Display` string such as
//! `not found: Go debugger 'dlv' (Delve) not found; …`. The session snapshot
//! now carries the human message without the error-variant prefix plus a
//! small `error_kind` the UI can turn into a next step.

use otto_core::Error;

use crate::toolchains_catalog::EcosystemKind;

/// The language's debug adapter (Delve, CodeLLDB/lldb) is not installed.
pub(crate) const KIND_ADAPTER_MISSING: &str = "adapter_missing";
/// The language runtime/SDK itself (Python, Node.js, a JDK, the shell) is missing.
pub(crate) const KIND_TOOLCHAIN_MISSING: &str = "toolchain_missing";
/// Compiling the target with debug symbols failed (`go build`, `cargo`, `javac`).
pub(crate) const KIND_BUILD_FAILED: &str = "build_failed";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StartFailure {
    pub(crate) message: String,
    pub(crate) kind: Option<&'static str>,
    pub(crate) ecosystem: Option<&'static str>,
}

/// Classify a start error for session `language` (the canonical debug language).
pub(crate) fn classify_start_error(err: &Error, language: &str) -> StartFailure {
    let message = human_message(err);
    let lower = message.to_ascii_lowercase();
    let kind = match err {
        Error::NotFound(_) if lower.contains("debugger") => Some(KIND_ADAPTER_MISSING),
        Error::NotFound(_)
            if ["interpreter", "runtime", "jdk", "compiler", "shell ("]
                .iter()
                .any(|needle| lower.contains(needle)) =>
        {
            Some(KIND_TOOLCHAIN_MISSING)
        }
        _ if lower.contains("build failed") || lower.contains("compile failed") => {
            Some(KIND_BUILD_FAILED)
        }
        _ => None,
    };
    let ecosystem = kind
        .and_then(|_| EcosystemKind::parse(language))
        .map(EcosystemKind::as_str);
    StartFailure {
        message,
        kind,
        ecosystem,
    }
}

/// The error's own text, without the `not found:` / `upstream:` variant prefix
/// its `Display` adds (that prefix is for logs, not for people).
fn human_message(err: &Error) -> String {
    match err {
        Error::NotFound(m)
        | Error::Forbidden(m)
        | Error::Conflict(m)
        | Error::Invalid(m)
        | Error::Upstream(m)
        | Error::Internal(m) => m.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_delve_is_an_adapter_problem_for_go_without_the_prefix() {
        let err = Error::NotFound(
            "Go debugger 'dlv' (Delve) not found; install it from IDE → Toolchains & SDKs".into(),
        );
        let f = classify_start_error(&err, "go");
        assert_eq!(f.kind, Some(KIND_ADAPTER_MISSING));
        assert_eq!(f.ecosystem, Some("go"));
        assert!(f.message.starts_with("Go debugger 'dlv'"), "{}", f.message);
        assert!(!f.message.starts_with("not found:"));
    }

    #[test]
    fn missing_runtimes_are_toolchain_problems_mapped_to_their_ecosystem() {
        let cases = [
            (
                "Python 3 interpreter not found; install Python via IDE → Toolchains",
                "python",
                "python",
            ),
            (
                "Node.js runtime not found; install Node.js via IDE → Toolchains",
                "typescript",
                "node",
            ),
            (
                "JDK 'java' not found; install Temurin JDK via IDE → Toolchains",
                "java",
                "java",
            ),
            ("Zsh shell ('zsh') not found on target", "shell", "shell"),
        ];
        for (msg, lang, eco) in cases {
            let f = classify_start_error(&Error::NotFound(msg.into()), lang);
            assert_eq!(f.kind, Some(KIND_TOOLCHAIN_MISSING), "{msg}");
            assert_eq!(f.ecosystem, Some(eco), "{msg}");
            assert_eq!(f.message, msg);
        }
    }

    #[test]
    fn build_failures_are_classified_whatever_the_variant() {
        for err in [
            Error::Invalid("go build failed:\n./main.go:3: undefined: x".into()),
            Error::Upstream("cargo debug build failed: error[E0425]".into()),
            Error::Upstream("javac debug compile failed: Main.java:1".into()),
        ] {
            let f = classify_start_error(&err, "go");
            assert_eq!(f.kind, Some(KIND_BUILD_FAILED), "{err}");
            assert!(!f.message.starts_with("invalid:") && !f.message.starts_with("upstream:"));
        }
    }

    #[test]
    fn other_failures_carry_no_kind_and_no_ecosystem() {
        let f = classify_start_error(&Error::Upstream("DAP 'initialize' timed out".into()), "go");
        assert_eq!(f.kind, None);
        assert_eq!(f.ecosystem, None);
        assert_eq!(f.message, "DAP 'initialize' timed out");
        let f = classify_start_error(&Error::NotFound("program file missing".into()), "go");
        assert_eq!(f.kind, None);
    }
}
