//! Router v1 — which provider + model + account answers a turn. Pure (no I/O,
//! no LLM call): local keyword rules classify the text as
//! `chat | code | hard | voice`, and the user's `targets[kind]` picks the route.
//!
//! Precedence (plan §2.4): an explicit thread **pin** wins; else a leading
//! `@claude` / `@codex` **mention** routes that one turn (and is stripped from
//! the pasted text); else the **rules**. Rules never flip a thread that already
//! has a provider on a single weak hint — a switch needs two keyword hits (or a
//! code fence / file name), so a chat thread that mentions "regex" once stays
//! put. Nothing here switches silently: the caller posts a `route` turn for
//! every provider change, and limit-driven failover happens only when the user
//! turned `auto_failover` on or answered "continue on X" for the thread.

use serde::{Deserialize, Serialize};

/// The request classes, in the order the settings UI lists them.
pub const KINDS: [&str; 4] = ["chat", "code", "hard", "voice"];

/// A provider + model + named account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteTarget {
    pub provider: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
}

impl RouteTarget {
    pub fn new(provider: &str, model: Option<&str>) -> Self {
        Self {
            provider: provider.into(),
            model: model.map(str::to_string),
            account_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Targets {
    pub chat: RouteTarget,
    pub code: RouteTarget,
    pub hard: RouteTarget,
    pub voice: RouteTarget,
}

impl Default for Targets {
    /// Decision 2: Claude for conversation / research / browser chores, Codex
    /// for code, shell and data; Opus-class for hard work; the fastest model
    /// for voice conversations.
    fn default() -> Self {
        Self {
            chat: RouteTarget::new("claude", Some("sonnet")),
            code: RouteTarget::new("codex", None),
            hard: RouteTarget::new("claude", Some("opus")),
            voice: RouteTarget::new("claude", Some("haiku")),
        }
    }
}

impl Targets {
    pub fn get(&self, kind: &str) -> &RouteTarget {
        match kind {
            "code" => &self.code,
            "hard" => &self.hard,
            "voice" => &self.voice,
            _ => &self.chat,
        }
    }

    fn all(&self) -> [&RouteTarget; 4] {
        [&self.chat, &self.code, &self.hard, &self.voice]
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtraKeywords {
    #[serde(default)]
    pub code: Vec<String>,
    #[serde(default)]
    pub hard: Vec<String>,
}

/// `AssistantRoutingSettings` on the wire. Default: the Decision-2 targets,
/// auto-failover and memory approval both OFF.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RoutingSettings {
    #[serde(default)]
    pub targets: Targets,
    #[serde(default)]
    pub extra_keywords: ExtraKeywords,
    #[serde(default)]
    pub auto_failover: bool,
    #[serde(default)]
    pub memory_approval: bool,
    #[serde(default)]
    pub updated_at: Option<String>,
}

/// `AssistantRouteDecision` on the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouteDecision {
    pub provider: String,
    pub model: Option<String>,
    pub account_id: Option<String>,
    pub kind: String,
    pub reason: String,
    pub matched: Vec<String>,
    /// The text to paste (a leading mention stripped).
    pub text: String,
}

impl RouteDecision {
    pub fn target(&self) -> RouteTarget {
        RouteTarget {
            provider: self.provider.clone(),
            model: self.model.clone(),
            account_id: self.account_id.clone(),
        }
    }
}

/// Providers a mention may name.
const MENTIONS: [&str; 2] = ["claude", "codex"];

/// Built-in "hard" phrases (Opus-class / high effort).
const HARD_PHRASES: &[&str] = &[
    "think hard",
    "think harder",
    "ultrathink",
    "think deeply",
    "think carefully",
    "deep dive",
    "in depth",
    "in-depth",
    "take your time",
    "multi-hour",
    "for hours",
    "thoroughly",
    "comprehensive plan",
    "research deeply",
];

/// Built-in "code" words / phrases (Codex: code, shell, data wrangling).
const CODE_WORDS: &[&str] = &[
    "code",
    "script",
    "bug",
    "debug",
    "stack trace",
    "traceback",
    "compile",
    "compiler",
    "function",
    "refactor",
    "regex",
    "sql",
    "query",
    "shell",
    "bash",
    "zsh",
    "terminal",
    "command line",
    "cli",
    "python",
    "javascript",
    "typescript",
    "rust",
    "golang",
    "java",
    "csv",
    "spreadsheet",
    "excel",
    "json",
    "yaml",
    "dataframe",
    "pandas",
    "git",
    "commit",
    "pull request",
    "unit test",
    "exception",
    "segfault",
    "api endpoint",
    "dockerfile",
    "kubernetes",
];

/// File extensions that mark a code / data request ("fix run.sh").
const CODE_EXTS: &[&str] = &[
    ".py", ".rs", ".ts", ".tsx", ".js", ".jsx", ".go", ".java", ".sh", ".sql", ".csv", ".xlsx",
    ".json", ".yaml", ".yml", ".toml", ".rb", ".swift", ".kt", ".c", ".cpp", ".h",
];

/// A request longer than this is "hard" (long, multi-part work).
pub const LONG_REQUEST_CHARS: usize = 6000;

/// Is `needle` in `hay` (both lower-case) on word boundaries?
fn has_phrase(hay: &str, needle: &str) -> bool {
    let bytes = hay.as_bytes();
    let mut from = 0;
    while let Some(pos) = hay[from..].find(needle) {
        let start = from + pos;
        let end = start + needle.len();
        let before_ok = start == 0 || !bytes[start - 1].is_ascii_alphanumeric();
        let after_ok = end >= bytes.len() || !bytes[end].is_ascii_alphanumeric();
        if before_ok && after_ok {
            return true;
        }
        from = start + needle.len().max(1);
        if from >= hay.len() {
            break;
        }
    }
    false
}

/// The first token that looks like a file name with a code/data extension.
fn file_hint(lower: &str) -> Option<String> {
    lower
        .split_whitespace()
        .map(|t| t.trim_matches(|c: char| "`'\"()[]{},;:!?".contains(c)))
        .find(|t| {
            CODE_EXTS
                .iter()
                .any(|e| t.len() > e.len() && t.ends_with(e) && !t.starts_with("http"))
        })
        .map(str::to_string)
}

/// Split a leading `@claude` / `@codex` mention off `text`. Returns the named
/// provider and the remaining text (trimmed; a `:`/`,` after the mention is
/// dropped too). Case-insensitive; `@claudeX` is not a mention.
pub fn parse_mention(text: &str) -> (Option<&'static str>, String) {
    let trimmed = text.trim_start();
    let Some(rest) = trimmed.strip_prefix('@') else {
        return (None, text.trim().to_string());
    };
    for p in MENTIONS {
        if rest
            .get(..p.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(p))
        {
            let after = &rest[p.len()..];
            let boundary = after
                .chars()
                .next()
                .is_none_or(|c| c.is_whitespace() || c == ':' || c == ',');
            if boundary {
                let body = after.trim_start_matches([':', ',']).trim();
                return (Some(p), body.to_string());
            }
        }
    }
    (None, text.trim().to_string())
}

/// Classify a request. Returns the kind and what matched (for the badge
/// tooltip and the "strong enough to switch" rule). `hard` beats `code`
/// ("think hard about this bug" wants the Opus-class model).
pub fn classify(text: &str, extra: &ExtraKeywords, voice: bool) -> (&'static str, Vec<String>) {
    if voice {
        return ("voice", vec!["voice".into()]);
    }
    let lower = text.to_lowercase();
    let mut hard: Vec<String> = HARD_PHRASES
        .iter()
        .map(|s| s.to_string())
        .chain(extra.hard.iter().map(|s| s.trim().to_lowercase()))
        .filter(|p| !p.is_empty() && has_phrase(&lower, p))
        .collect();
    if text.chars().count() > LONG_REQUEST_CHARS {
        hard.push("long request".into());
    }
    if !hard.is_empty() {
        return ("hard", hard);
    }
    let mut code: Vec<String> = CODE_WORDS
        .iter()
        .map(|s| s.to_string())
        .chain(extra.code.iter().map(|s| s.trim().to_lowercase()))
        .filter(|w| !w.is_empty() && has_phrase(&lower, w))
        .collect();
    if text.contains("```") {
        code.push("code block".into());
    }
    if let Some(f) = file_hint(&lower) {
        code.push(f);
    }
    code.dedup();
    if !code.is_empty() {
        return ("code", code);
    }
    ("chat", Vec::new())
}

/// A classification strong enough to move a thread off its current provider:
/// two hits, or a code fence / file name (unambiguous on its own).
fn strong(matched: &[String]) -> bool {
    matched.len() >= 2
        || matched
            .iter()
            .any(|m| m == "code block" || m == "long request" || m.contains('.'))
}

/// Everything [`decide`] looks at for one turn.
pub struct RouteInput<'a> {
    pub text: &'a str,
    pub voice: bool,
    pub settings: &'a RoutingSettings,
    /// The thread's explicit pin, if any.
    pub pin: Option<RouteTarget>,
    /// The thread's current route (its backing session), if it has one.
    pub current: Option<RouteTarget>,
    /// Providers currently known to be over their usage limit.
    pub limited: &'a [String],
    /// Failover allowed for this turn (`auto_failover` on, or the user
    /// answered "continue on X" for this thread).
    pub failover: bool,
}

/// Decide the route for one turn.
pub fn decide(input: RouteInput<'_>) -> RouteDecision {
    let settings = input.settings;
    let (mention, body) = parse_mention(input.text);
    let (kind, matched) = classify(&body, &settings.extra_keywords, input.voice);

    // 1. A mention routes this one turn — to the configured target that uses
    // that provider (keeps its model/account), else the provider default.
    if let Some(p) = mention {
        let own = settings.targets.get(kind);
        let t = if own.provider == p {
            // The kind's own target, when it is on the named provider.
            own.clone()
        } else {
            settings
                .targets
                .all()
                .into_iter()
                .find(|t| t.provider == p)
                .cloned()
                .unwrap_or_else(|| RouteTarget::new(p, None))
        };
        return build(t, kind, "mention", vec![format!("@{p}")], body);
    }

    // 2. An explicit pin always wins (and is never failed over).
    if let Some(pin) = input.pin {
        return build(pin, kind, "pin", matched, body);
    }

    // 3. Rules, sticky on a weak hint.
    let ruled = settings.targets.get(kind).clone();
    let (mut target, mut reason) = if matched.is_empty() {
        (ruled, "default")
    } else {
        (ruled, "rule")
    };
    if let Some(cur) = &input.current {
        if cur.provider != target.provider && !strong(&matched) && kind != "voice" {
            target = cur.clone();
            reason = "default";
        }
    }

    // 4. Limit-aware failover — only when allowed; never silent (the caller
    // posts a `route` turn for the switch).
    if input.failover && input.limited.contains(&target.provider) {
        if let Some(alt) = settings
            .targets
            .all()
            .into_iter()
            .find(|t| !input.limited.contains(&t.provider))
        {
            target = alt.clone();
            reason = "failover";
        }
    }
    build(target, kind, reason, matched, body)
}

fn build(
    t: RouteTarget,
    kind: &str,
    reason: &str,
    matched: Vec<String>,
    text: String,
) -> RouteDecision {
    RouteDecision {
        provider: t.provider,
        model: t.model,
        account_id: t.account_id,
        kind: kind.to_string(),
        reason: reason.to_string(),
        matched,
        text,
    }
}

/// The first configured target on a provider other than `provider` — the
/// "continue on X?" suggestion when `provider` hits its limit.
pub fn alternative(settings: &RoutingSettings, provider: &str) -> Option<RouteTarget> {
    settings
        .targets
        .all()
        .into_iter()
        .find(|t| t.provider != provider)
        .cloned()
}

/// Merge a partial `PUT /assistant/routing` body over `current`. Unknown keys
/// are ignored; a malformed `targets` entry is a 400 at the caller.
pub fn merge_settings(
    current: &RoutingSettings,
    patch: &serde_json::Value,
) -> Result<RoutingSettings, String> {
    let mut out = current.clone();
    if let Some(t) = patch.get("targets") {
        let mut targets = serde_json::to_value(&out.targets).map_err(|e| e.to_string())?;
        if let (Some(obj), Some(src)) = (targets.as_object_mut(), t.as_object()) {
            for (k, v) in src {
                if !KINDS.contains(&k.as_str()) {
                    return Err(format!("unknown route kind '{k}'"));
                }
                obj.insert(k.clone(), v.clone());
            }
        }
        out.targets = serde_json::from_value(targets).map_err(|e| format!("targets: {e}"))?;
        for t in out.targets.all() {
            if !valid_provider(&t.provider) {
                return Err(format!(
                    "provider '{}' is not a valid provider name",
                    t.provider
                ));
            }
        }
    }
    if let Some(k) = patch.get("extra_keywords") {
        let mut kw: ExtraKeywords =
            serde_json::from_value(k.clone()).map_err(|e| format!("extra_keywords: {e}"))?;
        for list in [&mut kw.code, &mut kw.hard] {
            list.retain(|w| !w.trim().is_empty());
            list.truncate(100);
        }
        out.extra_keywords = kw;
    }
    if let Some(b) = patch
        .get("auto_failover")
        .and_then(serde_json::Value::as_bool)
    {
        out.auto_failover = b;
    }
    if let Some(b) = patch
        .get("memory_approval")
        .and_then(serde_json::Value::as_bool)
    {
        out.memory_approval = b;
    }
    Ok(out)
}

/// Same slug rule as Personal Agents: non-empty ASCII alphanumerics, `-`, `_`.
pub fn valid_provider(p: &str) -> bool {
    !p.is_empty()
        && p.len() <= 40
        && p.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn input<'a>(text: &'a str, s: &'a RoutingSettings) -> RouteInput<'a> {
        RouteInput {
            text,
            voice: false,
            settings: s,
            pin: None,
            current: None,
            limited: &[],
            failover: false,
        }
    }

    #[test]
    fn plain_conversation_goes_to_the_chat_default() {
        let s = RoutingSettings::default();
        let d = decide(input("Plan my trip to Lisbon next month", &s));
        assert_eq!(d.kind, "chat");
        assert_eq!(d.reason, "default");
        assert_eq!(d.provider, "claude");
        assert_eq!(d.model.as_deref(), Some("sonnet"));
        assert!(d.matched.is_empty());
    }

    #[test]
    fn code_and_data_go_to_codex() {
        let s = RoutingSettings::default();
        for text in [
            "fix this python script, it throws an exception",
            "clean up data.csv and merge the duplicate rows",
            "```\nfn main() {}\n```",
        ] {
            let d = decide(input(text, &s));
            assert_eq!(d.kind, "code", "{text}");
            assert_eq!(d.provider, "codex", "{text}");
            assert_eq!(d.reason, "rule", "{text}");
        }
    }

    #[test]
    fn hard_beats_code_and_long_requests_are_hard() {
        let s = RoutingSettings::default();
        let d = decide(input("think hard about this bug in the parser", &s));
        assert_eq!(d.kind, "hard");
        assert_eq!(d.model.as_deref(), Some("opus"));
        let long = "a".repeat(LONG_REQUEST_CHARS + 1);
        assert_eq!(decide(input(&long, &s)).kind, "hard");
    }

    #[test]
    fn keywords_match_whole_words_only() {
        let e = ExtraKeywords::default();
        // "codename" / "digit" must not trip "code" / "git".
        assert_eq!(
            classify("what's the codename of the digital launch?", &e, false).0,
            "chat"
        );
        assert_eq!(classify("git status please", &e, false).0, "code");
        // Extra user keywords extend the rules.
        let extra = ExtraKeywords {
            code: vec!["terraform".into()],
            hard: vec!["full audit".into()],
        };
        assert_eq!(classify("write terraform for it", &extra, false).0, "code");
        assert_eq!(classify("do a full audit", &extra, false).0, "hard");
    }

    #[test]
    fn voice_mode_uses_the_voice_target() {
        let s = RoutingSettings::default();
        let mut i = input("what's the weather", &s);
        i.voice = true;
        let d = decide(i);
        assert_eq!(d.kind, "voice");
        assert_eq!(d.model.as_deref(), Some("haiku"));
    }

    #[test]
    fn mention_routes_one_turn_and_is_stripped() {
        let s = RoutingSettings::default();
        let mut i = input("@codex: summarize this email", &s);
        i.pin = Some(RouteTarget::new("claude", Some("opus")));
        let d = decide(i);
        assert_eq!(d.provider, "codex");
        assert_eq!(d.reason, "mention");
        assert_eq!(d.text, "summarize this email");
        assert_eq!(d.matched, vec!["@codex".to_string()]);
        let d = decide(input("@Claude write a regex for emails", &s));
        assert_eq!(d.provider, "claude");
        assert_eq!(d.text, "write a regex for emails");
        // Not a mention: glued suffix, or not at the start.
        assert_eq!(parse_mention("@claudette hi").0, None);
        assert_eq!(parse_mention("ask @codex later").0, None);
        assert_eq!(parse_mention("@codex").0, Some("codex"));
    }

    #[test]
    fn pin_beats_rules() {
        let s = RoutingSettings::default();
        let mut i = input("fix this python script and the sql query", &s);
        i.pin = Some(RouteTarget::new("claude", Some("opus")));
        let d = decide(i);
        assert_eq!(d.reason, "pin");
        assert_eq!(d.provider, "claude");
        assert_eq!(d.kind, "code");
    }

    #[test]
    fn a_weak_hint_does_not_flip_an_existing_thread() {
        let s = RoutingSettings::default();
        let mut i = input("any good regex tutorials?", &s);
        i.current = Some(RouteTarget::new("claude", Some("sonnet")));
        let d = decide(i);
        assert_eq!(d.provider, "claude");
        assert_eq!(d.reason, "default");
        // Two hits (or a file name) are strong enough.
        let mut i = input("debug this python traceback", &s);
        i.current = Some(RouteTarget::new("claude", None));
        assert_eq!(decide(i).provider, "codex");
        let mut i = input("look at deploy.sh", &s);
        i.current = Some(RouteTarget::new("claude", None));
        assert_eq!(decide(i).provider, "codex");
    }

    #[test]
    fn failover_only_when_allowed() {
        let s = RoutingSettings::default();
        let limited = vec!["claude".to_string()];
        let mut i = input("plan my week", &s);
        i.limited = &limited;
        // No failover allowed: stays on the limited provider (the caller asks).
        assert_eq!(decide(i).provider, "claude");
        let mut i = input("plan my week", &s);
        i.limited = &limited;
        i.failover = true;
        let d = decide(i);
        assert_eq!(d.provider, "codex");
        assert_eq!(d.reason, "failover");
        // A pin is never failed over.
        let mut i = input("plan my week", &s);
        i.limited = &limited;
        i.failover = true;
        i.pin = Some(RouteTarget::new("claude", None));
        assert_eq!(decide(i).provider, "claude");
    }

    #[test]
    fn alternative_picks_another_provider() {
        let s = RoutingSettings::default();
        assert_eq!(alternative(&s, "claude").unwrap().provider, "codex");
        assert_eq!(alternative(&s, "codex").unwrap().provider, "claude");
    }

    #[test]
    fn merge_settings_is_partial_and_validated() {
        let cur = RoutingSettings::default();
        let out = merge_settings(
            &cur,
            &json!({"auto_failover": true, "targets": {"code": {"provider":"claude","model":"opus"}}}),
        )
        .unwrap();
        assert!(out.auto_failover);
        assert!(!out.memory_approval);
        assert_eq!(out.targets.code.provider, "claude");
        assert_eq!(out.targets.chat, cur.targets.chat);
        assert!(merge_settings(&cur, &json!({"targets": {"bogus": {"provider":"x"}}})).is_err());
        assert!(merge_settings(
            &cur,
            &json!({"targets": {"chat": {"provider":"bad name!"}}})
        )
        .is_err());
        let kw = merge_settings(
            &cur,
            &json!({"extra_keywords": {"code": ["", "terraform"]}}),
        )
        .unwrap();
        assert_eq!(kw.extra_keywords.code, vec!["terraform".to_string()]);
    }

    #[test]
    fn settings_round_trip_the_wire_shape() {
        let v = serde_json::to_value(RoutingSettings::default()).unwrap();
        for k in KINDS {
            assert!(v["targets"][k]["provider"].is_string(), "{k}");
            assert!(v["targets"][k].get("account_id").is_some(), "{k}");
        }
        assert_eq!(v["auto_failover"], false);
        assert_eq!(v["memory_approval"], false);
        assert!(v["extra_keywords"]["code"].is_array());
        let back: RoutingSettings = serde_json::from_value(json!({})).unwrap();
        assert_eq!(back, RoutingSettings::default());
    }
}
