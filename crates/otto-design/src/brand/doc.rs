//! The `otto-brand/1` document: the schema validator every save runs
//! (`format::validate` → [`validate`]) and the lenient [`BrandModel`] the
//! exporters, the contrast report and the impact diff read.
//!
//! ```json
//! { "$schema": "otto-brand/1", "name": "Acme",
//!   "color":  { "primary": { "$value": "#5B3DF5", "$description": "CTAs" } },
//!   "font":   { "display": { "$value": "\"Inter\", system-ui, sans-serif", "weights": [700, 800] } },
//!   "type":   { "display": { "size": 64, "line": 72, "weight": 800 } },
//!   "radius": { "card": { "$value": 14 } },
//!   "space":  { "md": { "$value": 16 } },
//!   "logos":  [ { "name": "Acme", "kind": "full", "asset": "otto://design/<id>" } ],
//!   "imagery": { "summary": "…", "do": ["…"], "dont": ["…"] },
//!   "voice":  { "summary": "…", "do": ["…"], "dont": ["…"] } }
//! ```
//!
//! Every group is optional; unknown top-level keys are kept (forward
//! compatible), `$`-prefixed DTCG keys inside a token (`$type`, `$extensions`
//! …) are ignored. Two Phase 0 scaffold markers stay accepted so an old kit can
//! be re-saved: a top-level `"type": "otto-brand"` string and `"version": 1`.
//! Pure; unit-tested.

use std::collections::BTreeMap;

use otto_core::{Error, Result};
use serde_json::{json, Map, Value};

use crate::uri::{DesignUri, PREFIX};

/// The `$schema` of a v1 kit.
pub const SCHEMA: &str = "otto-brand/1";
/// Token groups, in export order. `type` holds composite text styles.
pub const GROUPS: &[&str] = &["color", "font", "type", "radius", "space"];
/// The font roles a kit may define.
pub const FONT_ROLES: &[&str] = &["display", "body", "mono"];
/// Logo variants.
pub const LOGO_KINDS: &[&str] = &["full", "mark", "mono"];

pub const MAX_TOKENS_PER_GROUP: usize = 64;
pub const MAX_LOGOS: usize = 16;
pub const MAX_LIST_ITEMS: usize = 20;
const MAX_NAME: usize = 64;
const MAX_KIT_NAME: usize = 200;
const MAX_TEXT: usize = 2_000;
const MAX_STACK: usize = 300;
const MAX_PX: f64 = 10_000.0;
/// Issues reported per failed validation (the rest are summarized).
const MAX_ISSUES: usize = 12;

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Reject a document that isn't a well-formed `otto-brand/1` kit. The error
/// lists the first problems by JSON path (`color.primary.$value: …`).
pub fn validate(v: &Value) -> Result<()> {
    let issues = issues(v);
    if issues.is_empty() {
        return Ok(());
    }
    let more = issues.len().saturating_sub(MAX_ISSUES);
    let mut msg = issues
        .into_iter()
        .take(MAX_ISSUES)
        .collect::<Vec<_>>()
        .join("; ");
    if more > 0 {
        msg.push_str(&format!("; … and {more} more"));
    }
    Err(Error::Invalid(format!(
        "invalid otto-brand document: {msg}"
    )))
}

/// Every problem with `v`, as `path: what is wrong` lines (empty = valid).
pub fn issues(v: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let Some(root) = v.as_object() else {
        out.push("the document must be a JSON object".into());
        return out;
    };
    match root.get("$schema") {
        None => {}
        Some(Value::String(s)) if s == SCHEMA => {}
        Some(other) => out.push(format!("$schema: expected \"{SCHEMA}\", got {other}")),
    }
    if let Some(n) = root.get("name") {
        match n.as_str() {
            Some(s) if s.chars().count() <= MAX_KIT_NAME => {}
            Some(_) => out.push(format!("name: longer than {MAX_KIT_NAME} characters")),
            None => out.push("name: must be a string".into()),
        }
    }
    if let Some(c) = root.get("color") {
        tokens(c, "color", &mut out, |path, tok, out| {
            match tok.get("$value").and_then(Value::as_str) {
                Some(s) if normalize_hex(s).is_some() => {}
                Some(s) => out.push(format!(
                    "{path}.$value: {s:?} is not a #RGB, #RRGGBB or #RRGGBBAA colour"
                )),
                None => out.push(format!("{path}.$value: a hex colour string is required")),
            }
        });
    }
    if let Some(f) = root.get("font") {
        tokens(f, "font", &mut out, |path, tok, out| {
            let role = path.trim_start_matches("font.");
            if !FONT_ROLES.contains(&role) {
                out.push(format!("{path}: font roles are {}", FONT_ROLES.join(" | ")));
            }
            match tok.get("$value").and_then(Value::as_str) {
                Some(s) if s.trim().is_empty() => {
                    out.push(format!("{path}.$value: the family stack is empty"))
                }
                Some(s) if s.chars().count() > MAX_STACK => {
                    out.push(format!("{path}.$value: longer than {MAX_STACK} characters"))
                }
                Some(s) if !safe_css_value(s) => out.push(format!(
                    "{path}.$value: may not contain ; {{ }} < > \\ or line breaks"
                )),
                Some(_) => {}
                None => out.push(format!("{path}.$value: a font family stack is required")),
            }
            if let Some(w) = tok.get("weights") {
                match w.as_array() {
                    Some(ws) if ws.len() <= 9 => {
                        if !ws.iter().all(|x| weight(x).is_some()) {
                            out.push(format!("{path}.weights: integers from 1 to 1000"));
                        }
                    }
                    Some(_) => out.push(format!("{path}.weights: at most 9 weights")),
                    None => out.push(format!("{path}.weights: must be an array")),
                }
            }
        });
    }
    if let Some(t) = root.get("type") {
        match t {
            // Phase 0 scaffold marker (`"type": "otto-brand"`), not a type scale.
            Value::String(s) if s == "otto-brand" => {}
            Value::Object(_) => tokens(t, "type", &mut out, |path, tok, out| {
                for k in ["size", "line"] {
                    if px(tok.get(k)).is_none_or(|n| n <= 0.0) {
                        out.push(format!("{path}.{k}: a positive px number is required"));
                    }
                }
                if tok.get("weight").and_then(weight).is_none() {
                    out.push(format!(
                        "{path}.weight: an integer from 1 to 1000 is required"
                    ));
                }
            }),
            _ => out.push("type: must be an object of text styles".into()),
        }
    }
    for g in ["radius", "space"] {
        if let Some(x) = root.get(g) {
            tokens(x, g, &mut out, |path, tok, out| {
                if px(tok.get("$value")).is_none() {
                    out.push(format!(
                        "{path}.$value: a px number from 0 to {MAX_PX} is required"
                    ));
                }
            });
        }
    }
    if let Some(l) = root.get("logos") {
        match l.as_array() {
            None => out.push("logos: must be an array".into()),
            Some(items) => {
                if items.len() > MAX_LOGOS {
                    out.push(format!("logos: at most {MAX_LOGOS} logos"));
                }
                for (i, item) in items.iter().enumerate().take(MAX_LOGOS) {
                    logo_issues(item, i, &mut out);
                }
            }
        }
    }
    for g in ["voice", "imagery"] {
        if let Some(x) = root.get(g) {
            prose_issues(x, g, &mut out);
        }
    }
    out
}

/// Walk one token group: an object of `name → token object`.
fn tokens(
    group: &Value,
    name: &str,
    out: &mut Vec<String>,
    check: impl Fn(&str, &Map<String, Value>, &mut Vec<String>),
) {
    let Some(map) = group.as_object() else {
        out.push(format!("{name}: must be an object of tokens"));
        return;
    };
    let real: Vec<_> = map.iter().filter(|(k, _)| !k.starts_with('$')).collect();
    if real.len() > MAX_TOKENS_PER_GROUP {
        out.push(format!("{name}: at most {MAX_TOKENS_PER_GROUP} tokens"));
    }
    for (k, tok) in real.into_iter().take(MAX_TOKENS_PER_GROUP) {
        let path = format!("{name}.{k}");
        if !valid_token_name(k) {
            out.push(format!(
                "{path}: token names are 1-{MAX_NAME} letters, digits, - or _ (starting with a letter or digit)"
            ));
        }
        let Some(obj) = tok.as_object() else {
            out.push(format!("{path}: must be an object"));
            continue;
        };
        if let Some(d) = obj.get("$description") {
            match d.as_str() {
                Some(s) if s.chars().count() <= MAX_TEXT => {}
                Some(_) => out.push(format!(
                    "{path}.$description: longer than {MAX_TEXT} characters"
                )),
                None => out.push(format!("{path}.$description: must be a string")),
            }
        }
        check(&path, obj, out);
    }
}

fn logo_issues(item: &Value, i: usize, out: &mut Vec<String>) {
    let path = format!("logos[{i}]");
    let Some(o) = item.as_object() else {
        out.push(format!("{path}: must be an object"));
        return;
    };
    match o.get("name").and_then(Value::as_str) {
        Some(s) if !s.trim().is_empty() && s.chars().count() <= MAX_KIT_NAME => {}
        _ => out.push(format!("{path}.name: a non-empty name is required")),
    }
    match o.get("kind").and_then(Value::as_str) {
        Some(k) if LOGO_KINDS.contains(&k) => {}
        _ => out.push(format!("{path}.kind: one of {}", LOGO_KINDS.join(" | "))),
    }
    // An empty/omitted asset is a placeholder slot ("not uploaded yet").
    match o.get("asset") {
        None | Some(Value::Null) => {}
        Some(Value::String(s)) if s.is_empty() || valid_asset(s) => {}
        Some(_) => out.push(format!(
            "{path}.asset: an otto://design/<id> reference or blob:<sha256>"
        )),
    }
}

/// `voice` / `imagery`: `{summary?, do?: [..], dont?: [..], …}`.
fn prose_issues(v: &Value, name: &str, out: &mut Vec<String>) {
    let Some(o) = v.as_object() else {
        out.push(format!("{name}: must be an object"));
        return;
    };
    if let Some(s) = o.get("summary") {
        match s.as_str() {
            Some(t) if t.chars().count() <= MAX_TEXT => {}
            _ => out.push(format!(
                "{name}.summary: a string up to {MAX_TEXT} characters"
            )),
        }
    }
    for k in ["do", "dont"] {
        if let Some(l) = o.get(k) {
            let ok = l.as_array().is_some_and(|items| {
                items.len() <= MAX_LIST_ITEMS
                    && items
                        .iter()
                        .all(|x| x.as_str().is_some_and(|t| t.chars().count() <= MAX_TEXT))
            });
            if !ok {
                out.push(format!(
                    "{name}.{k}: up to {MAX_LIST_ITEMS} strings of at most {MAX_TEXT} characters"
                ));
            }
        }
    }
}

/// `[A-Za-z0-9][A-Za-z0-9_-]{0,63}`.
pub fn valid_token_name(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    s.len() <= MAX_NAME && chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// A logo asset: a well-formed design reference or a content-addressed blob.
pub fn valid_asset(s: &str) -> bool {
    if s.starts_with(PREFIX) {
        return DesignUri::parse(s).is_some();
    }
    s.strip_prefix("blob:").is_some_and(crate::blobs::is_sha)
}

/// Nothing that could close a declaration or a tag when the value is pasted
/// into a stylesheet or a `style` attribute.
fn safe_css_value(s: &str) -> bool {
    !s.chars()
        .any(|c| matches!(c, ';' | '{' | '}' | '<' | '>' | '\\' | '\n' | '\r'))
}

fn px(v: Option<&Value>) -> Option<f64> {
    let n = v?.as_f64()?;
    (n.is_finite() && (0.0..=MAX_PX).contains(&n)).then_some(n)
}

fn weight(v: &Value) -> Option<u16> {
    let n = v.as_u64()?;
    (1..=1000).contains(&n).then_some(n as u16)
}

/// `#abc` → `#AABBCC`; `#aabbcc` / `#aabbccdd` → upper case. `None` for
/// anything else (named colours, `rgb()` …).
pub fn normalize_hex(s: &str) -> Option<String> {
    let h = s.trim().strip_prefix('#')?;
    if !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    match h.len() {
        3 => Some(format!(
            "#{}",
            h.chars()
                .flat_map(|c| [c, c])
                .collect::<String>()
                .to_ascii_uppercase()
        )),
        6 | 8 => Some(format!("#{}", h.to_ascii_uppercase())),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// The lenient model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct ColorTok {
    pub name: String,
    /// Normalized `#RRGGBB` / `#RRGGBBAA`.
    pub hex: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FontTok {
    /// `display` | `body` | `mono`.
    pub name: String,
    pub stack: String,
    pub weights: Vec<u16>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeTok {
    pub name: String,
    pub size: f64,
    pub line: f64,
    pub weight: u16,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PxTok {
    pub name: String,
    pub px: f64,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Logo {
    pub name: String,
    pub kind: String,
    pub asset: Option<String>,
}

/// What the exporters / contrast / impact read. Built leniently: entries that
/// don't validate are skipped, never an error (an old kit still exports).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BrandModel {
    pub name: String,
    pub colors: Vec<ColorTok>,
    pub fonts: Vec<FontTok>,
    pub types: Vec<TypeTok>,
    pub radius: Vec<PxTok>,
    pub space: Vec<PxTok>,
    pub logos: Vec<Logo>,
    pub voice: Option<Value>,
    pub imagery: Option<Value>,
}

fn entries<'a>(
    root: &'a Map<String, Value>,
    group: &str,
) -> Vec<(&'a String, &'a Map<String, Value>)> {
    root.get(group)
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter(|(k, _)| valid_token_name(k))
                .filter_map(|(k, v)| v.as_object().map(|o| (k, o)))
                .take(MAX_TOKENS_PER_GROUP)
                .collect()
        })
        .unwrap_or_default()
}

fn description(o: &Map<String, Value>) -> Option<String> {
    o.get("$description")
        .and_then(Value::as_str)
        .map(|s| s.chars().take(MAX_TEXT).collect())
}

impl BrandModel {
    /// Parse bytes (a stored version) — an unparseable document is empty.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        serde_json::from_slice::<Value>(bytes)
            .map(|v| Self::from_value(&v))
            .unwrap_or_default()
    }

    pub fn from_value(v: &Value) -> Self {
        let Some(root) = v.as_object() else {
            return Self::default();
        };
        let mut m = BrandModel {
            name: root
                .get("name")
                .and_then(Value::as_str)
                .map(|s| s.chars().take(MAX_KIT_NAME).collect())
                .unwrap_or_default(),
            ..Default::default()
        };
        for (k, o) in entries(root, "color") {
            if let Some(hex) = o
                .get("$value")
                .and_then(Value::as_str)
                .and_then(normalize_hex)
            {
                m.colors.push(ColorTok {
                    name: k.clone(),
                    hex,
                    description: description(o),
                });
            }
        }
        for (k, o) in entries(root, "font") {
            if !FONT_ROLES.contains(&k.as_str()) {
                continue;
            }
            if let Some(s) = o.get("$value").and_then(Value::as_str) {
                let stack = clean_css_value(s);
                if stack.is_empty() {
                    continue;
                }
                let weights: Vec<u16> = o
                    .get("weights")
                    .and_then(Value::as_array)
                    .map(|ws| ws.iter().filter_map(weight).take(9).collect())
                    .unwrap_or_default();
                m.fonts.push(FontTok {
                    name: k.clone(),
                    stack,
                    weights,
                });
            }
        }
        if root.get("type").is_some_and(Value::is_object) {
            for (k, o) in entries(root, "type") {
                let (Some(size), Some(line), Some(w)) = (
                    px(o.get("size")),
                    px(o.get("line")),
                    o.get("weight").and_then(weight),
                ) else {
                    continue;
                };
                if size > 0.0 && line > 0.0 {
                    m.types.push(TypeTok {
                        name: k.clone(),
                        size,
                        line,
                        weight: w,
                    });
                }
            }
        }
        for (group, dst) in [("radius", &mut m.radius), ("space", &mut m.space)] {
            for (k, o) in entries(root, group) {
                if let Some(n) = px(o.get("$value")) {
                    dst.push(PxTok {
                        name: k.clone(),
                        px: n,
                        description: description(o),
                    });
                }
            }
        }
        if let Some(items) = root.get("logos").and_then(Value::as_array) {
            for o in items.iter().filter_map(Value::as_object).take(MAX_LOGOS) {
                let (Some(name), Some(kind)) = (
                    o.get("name").and_then(Value::as_str),
                    o.get("kind").and_then(Value::as_str),
                ) else {
                    continue;
                };
                if !LOGO_KINDS.contains(&kind) {
                    continue;
                }
                m.logos.push(Logo {
                    name: name.chars().take(MAX_KIT_NAME).collect(),
                    kind: kind.to_string(),
                    asset: o
                        .get("asset")
                        .and_then(Value::as_str)
                        .filter(|s| valid_asset(s))
                        .map(str::to_string),
                });
            }
        }
        m.voice = root.get("voice").filter(|v| v.is_object()).cloned();
        m.imagery = root.get("imagery").filter(|v| v.is_object()).cloned();
        m
    }

    /// Every token as `group.name → canonical value` — what the impact diff
    /// compares. Type styles flatten to `size/line/weight`; a font's weights
    /// are not part of its value (they don't change what consumers render).
    pub fn flat(&self) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        for c in &self.colors {
            out.insert(format!("color.{}", c.name), c.hex.clone());
        }
        for f in &self.fonts {
            out.insert(format!("font.{}", f.name), f.stack.clone());
        }
        for t in &self.types {
            out.insert(
                format!("type.{}", t.name),
                format!("{}/{}/{}", fmt_num(t.size), fmt_num(t.line), t.weight),
            );
        }
        for r in &self.radius {
            out.insert(format!("radius.{}", r.name), format!("{}px", fmt_num(r.px)));
        }
        for s in &self.space {
            out.insert(format!("space.{}", s.name), format!("{}px", fmt_num(s.px)));
        }
        out
    }

    /// The CSS custom properties of the kit, in group order:
    /// `--brand-<group>-<name>` (type styles: `-size` / `-line` / `-weight`).
    /// Mirrored by `brandCssVars` in `ui/src/modules/design-hall/brand/tokens.ts`.
    pub fn css_vars(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for c in &self.colors {
            out.push((css_var("color", &c.name), c.hex.clone()));
        }
        for f in &self.fonts {
            out.push((css_var("font", &f.name), f.stack.clone()));
        }
        for t in &self.types {
            let base = css_var("type", &t.name);
            out.push((format!("{base}-size"), format!("{}px", fmt_num(t.size))));
            out.push((format!("{base}-line"), format!("{}px", fmt_num(t.line))));
            out.push((format!("{base}-weight"), t.weight.to_string()));
        }
        for r in &self.radius {
            out.push((css_var("radius", &r.name), format!("{}px", fmt_num(r.px))));
        }
        for s in &self.space {
            out.push((css_var("space", &s.name), format!("{}px", fmt_num(s.px))));
        }
        out
    }

    /// The font role a text style uses: `display` for display/headline/title/
    /// h1-h6 styles, `mono` for code/mono, else `body` — falling back to the
    /// first defined font. Mirrored by `fontRoleForStyle` in tokens.ts.
    pub fn font_for_style(&self, style: &str) -> Option<&FontTok> {
        let want = font_role_for_style(style);
        self.fonts
            .iter()
            .find(|f| f.name == want)
            .or_else(|| self.fonts.first())
    }
}

/// See [`BrandModel::font_for_style`].
pub fn font_role_for_style(style: &str) -> &'static str {
    let s = style.to_ascii_lowercase();
    let heading = s.len() == 2 && s.starts_with('h') && s.as_bytes()[1].is_ascii_digit();
    if s.contains("mono") || s.contains("code") {
        "mono"
    } else if heading
        || s.contains("display")
        || s.contains("headline")
        || s.contains("title")
        || s.contains("hero")
    {
        "display"
    } else {
        "body"
    }
}

/// `primaryColor` / `surface_alt` → `primary-color` / `surface-alt`: the CSS
/// spelling of a token name (lower-case kebab). Mirrored by `cssIdent` in
/// tokens.ts — keep the two identical.
pub fn css_ident(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    let mut prev: Option<char> = None;
    for c in name.chars() {
        if c.is_ascii_uppercase() {
            if prev.is_some_and(|p| p.is_ascii_lowercase() || p.is_ascii_digit()) {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
        } else if c == '_' {
            out.push('-');
        } else {
            out.push(c);
        }
        prev = Some(c);
    }
    out
}

/// `--brand-<group>-<css_ident(name)>`.
pub fn css_var(group: &str, name: &str) -> String {
    format!("--brand-{group}-{}", css_ident(name))
}

/// A px/number in its shortest form: `16`, `1.5`, `0.125`.
pub fn fmt_num(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        let r = (n * 1000.0).round() / 1000.0;
        format!("{r}")
    }
}

/// Drop the characters [`safe_css_value`] forbids (older / hand-edited kits).
pub fn clean_css_value(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, ';' | '{' | '}' | '<' | '>' | '\\' | '\n' | '\r'))
        .collect::<String>()
        .trim()
        .to_string()
}

/// The document a brand-new `otto-brand` artifact starts with when the caller
/// sent no content (the UI always sends one of its starter kits).
pub fn default_doc(name: &str) -> Value {
    json!({
        "$schema": SCHEMA,
        "name": name,
        "color": {
            "primary": { "$value": "#4F46E5" },
            "accent": { "$value": "#F59E0B" },
            "ink": { "$value": "#111827" },
            "surface": { "$value": "#FFFFFF" }
        },
        "font": {
            "display": { "$value": "-apple-system, system-ui, sans-serif", "weights": [700, 800] },
            "body": { "$value": "-apple-system, system-ui, sans-serif", "weights": [400, 600] },
            "mono": { "$value": "ui-monospace, Menlo, monospace" }
        },
        "type": {
            "display": { "size": 64, "line": 72, "weight": 800 },
            "h2": { "size": 40, "line": 48, "weight": 700 },
            "body": { "size": 17, "line": 28, "weight": 400 }
        },
        "radius": { "sm": { "$value": 8 }, "card": { "$value": 14 }, "pill": { "$value": 999 } },
        "space": { "xs": { "$value": 4 }, "sm": { "$value": 8 }, "md": { "$value": 16 }, "lg": { "$value": 32 } },
        "logos": [],
        "voice": { "summary": "", "do": [], "dont": [] }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acme() -> Value {
        json!({
            "$schema": "otto-brand/1",
            "name": "Acme",
            "color": {
                "primary": { "$value": "#5b3df5", "$description": "CTAs" },
                "accent": { "$value": "#FFB547" },
                "ink": { "$value": "#14122B" },
                "surfaceAlt": { "$value": "#fff", "$type": "color" }
            },
            "font": { "display": { "$value": "\"Inter\", system-ui, sans-serif", "weights": [700, 800] } },
            "type": { "display": { "size": 64, "line": 72, "weight": 800 }, "body": { "size": 17, "line": 28, "weight": 400 } },
            "radius": { "card": { "$value": 14 }, "pill": { "$value": 999 } },
            "space": { "4": { "$value": 16 } },
            "logos": [
                { "name": "Acme", "kind": "full", "asset": "otto://design/LOGO1" },
                { "name": "Mark", "kind": "mark" }
            ],
            "voice": { "summary": "Warm, confident, never salesy.", "do": ["Join free"], "dont": ["Submit"] },
            "imagery": { "summary": "Product renders over stock photos" }
        })
    }

    #[test]
    fn a_full_kit_and_the_default_doc_validate() {
        assert_eq!(issues(&acme()), Vec::<String>::new());
        assert!(validate(&default_doc("New kit")).is_ok());
        // Phase 0 scaffold markers stay accepted.
        assert!(validate(&json!({ "type": "otto-brand", "version": 1 })).is_ok());
        assert!(validate(&json!({})).is_ok(), "every group is optional");
    }

    #[test]
    fn bad_tokens_are_reported_by_path() {
        let doc = json!({
            "$schema": "otto-brand/2",
            "color": { "primary": { "$value": "blue" }, "bad name": { "$value": "#fff" }, "x": "#fff" },
            "font": { "heading": { "$value": "Inter" }, "body": { "$value": "Inter; } body { color: red" } },
            "type": { "display": { "size": -1, "line": 72, "weight": 1200 } },
            "radius": { "card": { "$value": "14px" } },
            "logos": [{ "name": "", "kind": "wordmark", "asset": "https://example.com/logo.svg" }],
            "voice": { "do": "be nice" }
        });
        let msgs = issues(&doc);
        let has = |needle: &str| msgs.iter().any(|m| m.contains(needle));
        assert!(has("$schema"), "{msgs:?}");
        assert!(has("color.primary.$value"), "{msgs:?}");
        assert!(has("color.bad name"), "{msgs:?}");
        assert!(has("color.x: must be an object"), "{msgs:?}");
        assert!(has("font.heading: font roles"), "{msgs:?}");
        assert!(has("font.body.$value: may not contain"), "{msgs:?}");
        assert!(has("type.display.size"), "{msgs:?}");
        assert!(has("type.display.weight"), "{msgs:?}");
        assert!(has("radius.card.$value"), "{msgs:?}");
        assert!(
            has("logos[0].name") && has("logos[0].kind") && has("logos[0].asset"),
            "{msgs:?}"
        );
        assert!(has("voice.do"), "{msgs:?}");
        let err = validate(&doc).unwrap_err().to_string();
        assert!(err.contains("invalid otto-brand document"), "{err}");
        assert!(
            err.contains("more"),
            "long issue lists are summarized: {err}"
        );
        assert!(validate(&json!([1])).is_err());
        assert!(validate(&json!({ "type": 3 })).is_err());
    }

    #[test]
    fn too_many_tokens_are_refused() {
        let mut color = Map::new();
        for i in 0..=MAX_TOKENS_PER_GROUP {
            color.insert(format!("c{i}"), json!({ "$value": "#000" }));
        }
        let msgs = issues(&json!({ "color": color }));
        assert!(
            msgs.iter().any(|m| m.contains("at most 64 tokens")),
            "{msgs:?}"
        );
    }

    #[test]
    fn model_normalizes_and_flattens() {
        let m = BrandModel::from_value(&acme());
        assert_eq!(m.name, "Acme");
        let primary = m.colors.iter().find(|c| c.name == "primary").unwrap();
        assert_eq!(primary.hex, "#5B3DF5");
        assert_eq!(primary.description.as_deref(), Some("CTAs"));
        assert_eq!(
            m.colors
                .iter()
                .find(|c| c.name == "surfaceAlt")
                .unwrap()
                .hex,
            "#FFFFFF"
        );
        let flat = m.flat();
        assert_eq!(flat["type.display"], "64/72/800");
        assert_eq!(flat["radius.card"], "14px");
        assert_eq!(flat["space.4"], "16px");
        assert_eq!(flat["font.display"], "\"Inter\", system-ui, sans-serif");
        assert_eq!(m.logos.len(), 2);
        assert_eq!(m.logos[0].asset.as_deref(), Some("otto://design/LOGO1"));
        assert_eq!(m.logos[1].asset, None);
        assert!(m.voice.is_some() && m.imagery.is_some());
    }

    #[test]
    fn model_skips_what_does_not_validate() {
        let m = BrandModel::from_value(&json!({
            "color": { "ok": { "$value": "#000" }, "bad": { "$value": "red" } },
            "font": { "heading": { "$value": "Inter" }, "body": { "$value": "Inter;}" } },
            "type": "otto-brand",
            "radius": { "r": { "$value": "8px" } }
        }));
        assert_eq!(m.colors.len(), 1);
        assert_eq!(m.fonts.len(), 1, "unknown font roles are skipped");
        assert_eq!(m.fonts[0].stack, "Inter", "unsafe characters are dropped");
        assert!(m.types.is_empty() && m.radius.is_empty());
        assert_eq!(BrandModel::from_bytes(b"not json"), BrandModel::default());
    }

    #[test]
    fn css_names_are_kebab_and_type_styles_expand() {
        assert_eq!(css_ident("surfaceAlt"), "surface-alt");
        assert_eq!(css_ident("surface_alt"), "surface-alt");
        assert_eq!(css_ident("h2"), "h2");
        assert_eq!(css_ident("XL"), "xl");
        assert_eq!(css_ident("brandBG2"), "brand-bg2");
        assert_eq!(css_var("color", "primary"), "--brand-color-primary");
        let vars = BrandModel::from_value(&acme()).css_vars();
        let get = |k: &str| vars.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("--brand-color-primary"), Some("#5B3DF5"));
        assert_eq!(get("--brand-color-surface-alt"), Some("#FFFFFF"));
        assert_eq!(get("--brand-type-display-size"), Some("64px"));
        assert_eq!(get("--brand-type-display-line"), Some("72px"));
        assert_eq!(get("--brand-type-display-weight"), Some("800"));
        assert_eq!(get("--brand-radius-card"), Some("14px"));
        assert_eq!(get("--brand-space-4"), Some("16px"));
    }

    #[test]
    fn helpers() {
        assert_eq!(normalize_hex("#abc").as_deref(), Some("#AABBCC"));
        assert_eq!(normalize_hex("#11223344").as_deref(), Some("#11223344"));
        assert_eq!(normalize_hex("#12345"), None);
        assert_eq!(normalize_hex("red"), None);
        assert_eq!(fmt_num(16.0), "16");
        assert_eq!(fmt_num(1.5), "1.5");
        assert_eq!(fmt_num(1.0 / 3.0), "0.333");
        assert!(valid_token_name("4") && valid_token_name("surface-alt"));
        assert!(!valid_token_name("-x") && !valid_token_name("") && !valid_token_name("a.b"));
        assert!(valid_asset(&format!("blob:{}", "a".repeat(64))));
        assert!(!valid_asset("blob:xyz") && !valid_asset("otto://design/@bad"));
        assert_eq!(font_role_for_style("display"), "display");
        assert_eq!(font_role_for_style("H2"), "display");
        assert_eq!(font_role_for_style("codeBlock"), "mono");
        assert_eq!(font_role_for_style("body"), "body");
        assert_eq!(font_role_for_style("h10"), "body");
    }
}
