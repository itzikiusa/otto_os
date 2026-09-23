//! Brand kit exports (`GET /design/artifacts/{id}/brand/export?format=`):
//!
//! | format     | output                                                          |
//! |------------|-----------------------------------------------------------------|
//! | `css`      | `:root { --brand-color-primary: #5B3DF5; … }` (the contract names) |
//! | `tailwind` | a Tailwind v4 `@theme { --color-primary: …; --text-display: …; }` block |
//! | `dtcg`     | W3C Design Tokens (DTCG) JSON: typed groups, dimensions as `{value, unit}`, logos/voice/imagery under `$extensions` |
//!
//! Every value comes from [`BrandModel`], which already dropped anything that
//! could break out of a declaration. Pure; unit-tested.

use serde_json::{json, Map, Value};

use super::doc::{css_ident, fmt_num, BrandModel, SCHEMA};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Css,
    Tailwind,
    Dtcg,
}

impl ExportFormat {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "css" => Some(Self::Css),
            "tailwind" => Some(Self::Tailwind),
            "dtcg" | "json" => Some(Self::Dtcg),
            _ => None,
        }
    }

    pub fn mime(self) -> &'static str {
        match self {
            Self::Css | Self::Tailwind => "text/css; charset=utf-8",
            Self::Dtcg => "application/json",
        }
    }

    /// Download file name for a kit slug (`acme-brand.tokens.css` …).
    pub fn file_name(self, slug: &str) -> String {
        match self {
            Self::Css => format!("{slug}.tokens.css"),
            Self::Tailwind => format!("{slug}.theme.css"),
            Self::Dtcg => format!("{slug}.tokens.json"),
        }
    }
}

/// `Acme Brand Kit!` → `acme-brand-kit` (ASCII, ≤ 60 chars, never empty).
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.len() >= 60 {
            break;
        }
    }
    let t = out.trim_matches('-').to_string();
    if t.is_empty() {
        "brand".into()
    } else {
        t
    }
}

pub fn export(m: &BrandModel, f: ExportFormat) -> String {
    match f {
        ExportFormat::Css => css(m),
        ExportFormat::Tailwind => tailwind(m),
        ExportFormat::Dtcg => {
            let mut s = serde_json::to_string_pretty(&dtcg(m)).unwrap_or_else(|_| "{}".into());
            s.push('\n');
            s
        }
    }
}

/// A comment-safe kit name.
fn comment_name(m: &BrandModel) -> String {
    let n = if m.name.trim().is_empty() {
        "Brand kit"
    } else {
        m.name.trim()
    };
    n.replace("*/", "* /").replace(['\n', '\r'], " ")
}

/// `:root { --brand-<group>-<name>: value; … }`.
pub fn css(m: &BrandModel) -> String {
    let mut out = format!(
        "/* {} — {SCHEMA} tokens, exported from Otto Design Hall */\n:root {{\n",
        comment_name(m)
    );
    for (k, v) in m.css_vars() {
        out.push_str(&format!("  {k}: {v};\n"));
    }
    out.push_str("}\n");
    out
}

/// Tailwind v4 theme variables (`@theme`): colours → `--color-*`, fonts →
/// `--font-*`, text styles → `--text-*` (+ `--line-height` / `--font-weight`
/// sub-properties), radius → `--radius-*`, space → `--spacing-*`.
pub fn tailwind(m: &BrandModel) -> String {
    let mut out = format!(
        "/* {} — Tailwind v4 theme from {SCHEMA}, exported from Otto Design Hall */\n@theme {{\n",
        comment_name(m)
    );
    let mut line = |k: String, v: String| out.push_str(&format!("  {k}: {v};\n"));
    for c in &m.colors {
        line(format!("--color-{}", css_ident(&c.name)), c.hex.clone());
    }
    for f in &m.fonts {
        line(format!("--font-{}", css_ident(&f.name)), f.stack.clone());
    }
    for t in &m.types {
        let n = css_ident(&t.name);
        line(format!("--text-{n}"), format!("{}px", fmt_num(t.size)));
        line(
            format!("--text-{n}--line-height"),
            format!("{}px", fmt_num(t.line)),
        );
        line(format!("--text-{n}--font-weight"), t.weight.to_string());
    }
    for r in &m.radius {
        line(
            format!("--radius-{}", css_ident(&r.name)),
            format!("{}px", fmt_num(r.px)),
        );
    }
    for s in &m.space {
        line(
            format!("--spacing-{}", css_ident(&s.name)),
            format!("{}px", fmt_num(s.px)),
        );
    }
    out.push_str("}\n");
    out
}

/// `"Inter", system-ui, sans-serif` → `["Inter", "system-ui", "sans-serif"]`.
pub fn split_stack(stack: &str) -> Vec<String> {
    stack
        .split(',')
        .map(|p| p.trim().trim_matches(['"', '\'']).trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

fn dimension(px: f64) -> Value {
    // Keep whole numbers integral in the JSON (`14`, not `14.0`).
    let value = if px.fract() == 0.0 && px.abs() < 1e15 {
        json!(px as i64)
    } else {
        json!((px * 1000.0).round() / 1000.0)
    };
    json!({ "value": value, "unit": "px" })
}

fn with_description(mut tok: Map<String, Value>, d: &Option<String>) -> Value {
    if let Some(d) = d.as_ref().filter(|d| !d.trim().is_empty()) {
        tok.insert("$description".into(), json!(d));
    }
    Value::Object(tok)
}

/// W3C DTCG (Design Tokens Community Group) JSON.
pub fn dtcg(m: &BrandModel) -> Value {
    let mut root = Map::new();
    if !m.name.trim().is_empty() {
        root.insert("$description".into(), json!(m.name));
    }
    if !m.colors.is_empty() {
        let mut g = Map::new();
        g.insert("$type".into(), json!("color"));
        for c in &m.colors {
            let mut t = Map::new();
            t.insert("$value".into(), json!(c.hex));
            g.insert(c.name.clone(), with_description(t, &c.description));
        }
        root.insert("color".into(), Value::Object(g));
    }
    if !m.fonts.is_empty() {
        let mut g = Map::new();
        g.insert("$type".into(), json!("fontFamily"));
        for f in &m.fonts {
            let mut t = Map::new();
            t.insert("$value".into(), json!(split_stack(&f.stack)));
            if !f.weights.is_empty() {
                t.insert(
                    "$extensions".into(),
                    json!({ "dev.otto.brand": { "weights": f.weights } }),
                );
            }
            g.insert(f.name.clone(), Value::Object(t));
        }
        root.insert("font".into(), Value::Object(g));
    }
    if !m.types.is_empty() {
        let mut g = Map::new();
        g.insert("$type".into(), json!("typography"));
        for t in &m.types {
            let mut v = Map::new();
            if let Some(f) = m.font_for_style(&t.name) {
                v.insert("fontFamily".into(), json!(format!("{{font.{}}}", f.name)));
            }
            v.insert("fontSize".into(), dimension(t.size));
            v.insert("fontWeight".into(), json!(t.weight));
            // DTCG line height is a unitless multiple of the font size.
            let lh = ((t.line / t.size) * 1000.0).round() / 1000.0;
            v.insert("lineHeight".into(), json!(lh));
            v.insert("letterSpacing".into(), dimension(0.0));
            g.insert(t.name.clone(), json!({ "$value": Value::Object(v) }));
        }
        root.insert("type".into(), Value::Object(g));
    }
    for (name, toks) in [("radius", &m.radius), ("space", &m.space)] {
        if toks.is_empty() {
            continue;
        }
        let mut g = Map::new();
        g.insert("$type".into(), json!("dimension"));
        for r in toks.iter() {
            let mut t = Map::new();
            t.insert("$value".into(), dimension(r.px));
            g.insert(r.name.clone(), with_description(t, &r.description));
        }
        root.insert(name.into(), Value::Object(g));
    }
    let mut ext = Map::new();
    ext.insert("schema".into(), json!(SCHEMA));
    ext.insert("name".into(), json!(m.name));
    if !m.logos.is_empty() {
        let logos: Vec<Value> = m
            .logos
            .iter()
            .map(|l| json!({ "name": l.name, "kind": l.kind, "asset": l.asset }))
            .collect();
        ext.insert("logos".into(), Value::Array(logos));
    }
    if let Some(v) = &m.voice {
        ext.insert("voice".into(), v.clone());
    }
    if let Some(v) = &m.imagery {
        ext.insert("imagery".into(), v.clone());
    }
    root.insert(
        "$extensions".into(),
        json!({ "dev.otto.brand": Value::Object(ext) }),
    );
    Value::Object(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kit() -> BrandModel {
        BrandModel::from_value(&json!({
            "$schema": "otto-brand/1",
            "name": "Acme */ Brand",
            "color": { "primary": { "$value": "#5B3DF5", "$description": "CTAs" }, "surfaceAlt": { "$value": "#F6F4FF" } },
            "font": { "display": { "$value": "\"Inter\", system-ui, sans-serif", "weights": [800] }, "body": { "$value": "system-ui" } },
            "type": { "display": { "size": 64, "line": 72, "weight": 800 }, "body": { "size": 17, "line": 28, "weight": 400 } },
            "radius": { "card": { "$value": 14 } },
            "space": { "md": { "$value": 16 }, "half": { "$value": 2.5 } },
            "logos": [{ "name": "Acme", "kind": "full", "asset": "otto://design/L1" }],
            "voice": { "summary": "Warm." }
        }))
    }

    #[test]
    fn css_uses_the_contract_names() {
        let s = css(&kit());
        assert!(
            s.starts_with("/* Acme * / Brand"),
            "comment can't be closed early: {s}"
        );
        assert!(s.contains(":root {"));
        for line in [
            "  --brand-color-primary: #5B3DF5;",
            "  --brand-color-surface-alt: #F6F4FF;",
            "  --brand-font-display: \"Inter\", system-ui, sans-serif;",
            "  --brand-type-display-size: 64px;",
            "  --brand-type-display-line: 72px;",
            "  --brand-type-display-weight: 800;",
            "  --brand-radius-card: 14px;",
            "  --brand-space-md: 16px;",
            "  --brand-space-half: 2.5px;",
        ] {
            assert!(s.contains(line), "missing {line:?} in\n{s}");
        }
        assert!(s.trim_end().ends_with('}'));
    }

    #[test]
    fn tailwind_uses_theme_namespaces() {
        let s = tailwind(&kit());
        assert!(s.contains("@theme {"));
        for line in [
            "  --color-primary: #5B3DF5;",
            "  --color-surface-alt: #F6F4FF;",
            "  --font-display: \"Inter\", system-ui, sans-serif;",
            "  --text-display: 64px;",
            "  --text-display--line-height: 72px;",
            "  --text-display--font-weight: 800;",
            "  --radius-card: 14px;",
            "  --spacing-md: 16px;",
        ] {
            assert!(s.contains(line), "missing {line:?} in\n{s}");
        }
    }

    #[test]
    fn dtcg_is_typed_and_carries_the_rest_as_extensions() {
        let v = dtcg(&kit());
        assert_eq!(v["color"]["$type"], "color");
        assert_eq!(v["color"]["primary"]["$value"], "#5B3DF5");
        assert_eq!(v["color"]["primary"]["$description"], "CTAs");
        assert_eq!(v["font"]["$type"], "fontFamily");
        assert_eq!(
            v["font"]["display"]["$value"],
            json!(["Inter", "system-ui", "sans-serif"])
        );
        assert_eq!(
            v["type"]["display"]["$value"]["fontFamily"],
            "{font.display}"
        );
        assert_eq!(v["type"]["body"]["$value"]["fontFamily"], "{font.body}");
        assert_eq!(
            v["type"]["display"]["$value"]["fontSize"],
            json!({ "value": 64, "unit": "px" })
        );
        assert_eq!(v["type"]["display"]["$value"]["lineHeight"], json!(1.125));
        assert_eq!(v["radius"]["$type"], "dimension");
        assert_eq!(
            v["radius"]["card"]["$value"],
            json!({ "value": 14, "unit": "px" })
        );
        assert_eq!(v["space"]["half"]["$value"]["value"], json!(2.5));
        let ext = &v["$extensions"]["dev.otto.brand"];
        assert_eq!(ext["schema"], "otto-brand/1");
        assert_eq!(ext["logos"][0]["asset"], "otto://design/L1");
        assert_eq!(ext["voice"]["summary"], "Warm.");
        // Round-trips as JSON text.
        let text = export(&kit(), ExportFormat::Dtcg);
        assert!(serde_json::from_str::<Value>(&text).is_ok());
    }

    #[test]
    fn an_empty_kit_exports_valid_empty_blocks() {
        let m = BrandModel::default();
        assert!(css(&m).contains(":root {\n}"));
        assert!(tailwind(&m).contains("@theme {\n}"));
        let v = dtcg(&m);
        assert!(v.get("color").is_none());
        assert_eq!(v["$extensions"]["dev.otto.brand"]["schema"], "otto-brand/1");
    }

    #[test]
    fn formats_and_slugs() {
        assert_eq!(ExportFormat::parse("CSS"), Some(ExportFormat::Css));
        assert_eq!(ExportFormat::parse("json"), Some(ExportFormat::Dtcg));
        assert_eq!(ExportFormat::parse("scss"), None);
        assert_eq!(ExportFormat::Dtcg.mime(), "application/json");
        assert_eq!(slug("Acme Brand Kit!"), "acme-brand-kit");
        assert_eq!(slug("  —  "), "brand");
        assert_eq!(ExportFormat::Tailwind.file_name("acme"), "acme.theme.css");
        assert_eq!(
            split_stack("'SF Pro', , system-ui"),
            vec!["SF Pro", "system-ui"]
        );
    }
}
