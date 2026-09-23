//! Brand kit → site theme. A kit's tokens become `--brand-<group>-<name>`
//! custom properties (through [`BrandModel`], the same names the Brand Kit's
//! CSS export uses), and the site stylesheet's theme SLOTS (`--os-primary`,
//! `--os-ink`, …) are bound to the tokens that fill them, or to defaults.
//! Tone (light / dark text) and WCAG contrast are computed from the concrete
//! colours. Mirrored by `ui/src/modules/design-hall/site/engine/theme.ts` —
//! keep the slot rules, defaults and sanitizing identical. Pure; unit-tested.

use serde_json::Value;

use crate::brand::doc::{css_ident, css_var, fmt_num, BrandModel};

pub const DEFAULT_PRIMARY: &str = "#5B3DF5";
pub const DEFAULT_ACCENT: &str = "#FFB547";
pub const DEFAULT_INK: &str = "#14122B";
pub const DEFAULT_SURFACE: &str = "#FFFFFF";
pub const DEFAULT_SURFACE_ALT: &str = "#F6F4FF";
pub const DEFAULT_FONT_DISPLAY: &str =
    "\"Inter Display\", Inter, ui-sans-serif, system-ui, -apple-system, \"Segoe UI\", sans-serif";
pub const DEFAULT_FONT_BODY: &str =
    "Inter, ui-sans-serif, system-ui, -apple-system, \"Segoe UI\", sans-serif";
pub const DEFAULT_RADIUS: &str = "14px";
const MAX_TOKENS: usize = 400;

#[derive(Debug, Clone, PartialEq)]
pub struct BrandToken {
    /// `color.primary` (the kit's own name).
    pub path: String,
    /// `--brand-color-primary`.
    pub css_var: String,
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct Theme {
    pub tokens: Vec<BrandToken>,
    pub primary: String,
    pub accent: String,
    pub ink: String,
    pub surface: String,
    pub surface_alt: String,
    pub paper: String,
    pub night: String,
    pub on_primary: String,
    pub on_accent: String,
    /// slot → the CSS it is bound to (`var(--brand-…)` or a literal), in
    /// [`SLOTS`] order.
    pub slot_css: Vec<(&'static str, String)>,
}

/// Slot → the kit tokens that may fill it (first match wins).
pub const SLOTS: &[(&str, &[&str])] = &[
    ("primary", &["color.primary", "color.brand", "color.main"]),
    (
        "accent",
        &["color.accent", "color.secondary", "color.highlight"],
    ),
    (
        "ink",
        &["color.ink", "color.text", "color.foreground", "color.fg"],
    ),
    (
        "surface",
        &[
            "color.surface",
            "color.background",
            "color.bg",
            "color.paper",
        ],
    ),
    (
        "surface-alt",
        &[
            "color.surface-alt",
            "color.subtle",
            "color.muted-surface",
            "color.canvas",
        ],
    ),
    (
        "font-display",
        &[
            "font.display",
            "font.heading",
            "font.headline",
            "font.title",
        ],
    ),
    (
        "font-body",
        &["font.body", "font.text", "font.base", "font.sans"],
    ),
    (
        "radius",
        &[
            "radius.card",
            "radius.m",
            "radius.md",
            "radius.default",
            "radius.base",
        ],
    ),
];

/// Strip anything that could escape a declaration (`; { } < > \` and controls).
pub fn css_value(v: &str) -> String {
    let kept: String = v
        .chars()
        .filter(|c| (*c as u32) >= 32 && !matches!(c, ';' | '{' | '}' | '<' | '>' | '\\'))
        .collect();
    kept.trim().chars().take(200).collect()
}

/// `surfaceAlt` / `Surface Alt` → `surface-alt` (legacy kits' names).
pub fn token_slug(s: &str) -> String {
    let mut spaced = String::with_capacity(s.len() + 4);
    let mut prev: Option<char> = None;
    for c in s.chars() {
        if c.is_ascii_uppercase()
            && prev.is_some_and(|p| p.is_ascii_lowercase() || p.is_ascii_digit())
        {
            spaced.push('-');
        }
        spaced.push(c);
        prev = Some(c);
    }
    let mut out = String::with_capacity(spaced.len());
    for c in spaced.to_ascii_lowercase().chars() {
        let keep = c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-';
        let ch = if keep { c } else { '-' };
        if ch == '-' && out.ends_with('-') {
            continue;
        }
        out.push(ch);
    }
    out.trim_matches('-').to_string()
}

/// `var(<name>)`.
pub fn var(name: &str) -> String {
    format!("var({name})")
}

// ---------------------------------------------------------------------------
// Colour math (WCAG 2.x)
// ---------------------------------------------------------------------------

/// `#rgb` / `#rrggbb` / `#rrggbbaa` / `rgb(r g b)` / `rgb(r, g, b)` → [r, g, b].
pub fn parse_color(v: &str) -> Option<[f64; 3]> {
    let s = v.trim().to_ascii_lowercase();
    if let Some(h) = s.strip_prefix('#') {
        if !h.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let full: String = match h.len() {
            3 => h.chars().flat_map(|c| [c, c]).collect(),
            6 | 8 => h[..6].to_string(),
            _ => return None,
        };
        let byte = |i: usize| u8::from_str_radix(&full[i..i + 2], 16).ok().map(f64::from);
        return Some([byte(0)?, byte(2)?, byte(4)?]);
    }
    let rest = s.strip_prefix("rgba(").or_else(|| s.strip_prefix("rgb("))?;
    let nums: Vec<f64> = rest
        .split(|c: char| c.is_whitespace() || c == ',' || c == ')' || c == '/')
        .filter(|x| !x.is_empty())
        .take(3)
        .map(|x| x.parse::<f64>())
        .collect::<std::result::Result<Vec<_>, _>>()
        .ok()?;
    if nums.len() == 3
        && nums
            .iter()
            .all(|x| x.is_finite() && (0.0..=255.0).contains(x))
    {
        Some([nums[0], nums[1], nums[2]])
    } else {
        None
    }
}

pub fn luminance(v: &str) -> Option<f64> {
    let c = parse_color(v)?;
    let lin = |x: f64| {
        let n = x / 255.0;
        if n <= 0.03928 {
            n / 12.92
        } else {
            ((n + 0.055) / 1.055).powf(2.4)
        }
    };
    Some(0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2]))
}

/// WCAG contrast ratio, one decimal.
pub fn contrast(a: &str, b: &str) -> Option<f64> {
    let la = luminance(a)?;
    let lb = luminance(b)?;
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    Some(((hi + 0.05) / (lo + 0.05) * 10.0).round() / 10.0)
}

/// `#aabbcc` mix of two colours (`t` = share of `b`); `a` when unparseable.
pub fn mix(a: &str, b: &str, t: f64) -> String {
    match (parse_color(a), parse_color(b)) {
        (Some(ca), Some(cb)) => {
            let mut out = String::from("#");
            for (a, b) in ca.iter().zip(cb.iter()) {
                let n = (a + (b - a) * t).round().clamp(0.0, 255.0) as u8;
                out.push_str(&format!("{n:02x}"));
            }
            out
        }
        _ => a.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Tokens
// ---------------------------------------------------------------------------

/// The kit's tokens: `otto-brand/1` through [`BrandModel`] (colours, fonts,
/// type styles as `-size`/`-line`/`-weight`, px radii and spacing), then any
/// pre-v1 leftovers (a `typography.*.fontFamily`, string radii) the lenient
/// walker finds.
pub fn tokens_of(kit: Option<&Value>) -> Vec<BrandToken> {
    let mut out: Vec<BrandToken> = Vec::new();
    let Some(v) = kit else {
        return out;
    };
    let m = BrandModel::from_value(v);
    let push = |out: &mut Vec<BrandToken>, path: String, css_name: String, raw: &str| {
        let value = css_value(raw);
        if value.is_empty() || out.len() >= MAX_TOKENS || out.iter().any(|t| t.path == path) {
            return;
        }
        out.push(BrandToken {
            path,
            css_var: css_name,
            value,
        });
    };
    for c in &m.colors {
        push(
            &mut out,
            format!("color.{}", c.name),
            css_var("color", &c.name),
            &c.hex,
        );
    }
    for f in &m.fonts {
        push(
            &mut out,
            format!("font.{}", f.name),
            css_var("font", &f.name),
            &f.stack,
        );
    }
    for t in &m.types {
        let base = css_var("type", &t.name);
        push(
            &mut out,
            format!("type.{}.size", t.name),
            format!("{base}-size"),
            &format!("{}px", fmt_num(t.size)),
        );
        push(
            &mut out,
            format!("type.{}.line", t.name),
            format!("{base}-line"),
            &format!("{}px", fmt_num(t.line)),
        );
        push(
            &mut out,
            format!("type.{}.weight", t.name),
            format!("{base}-weight"),
            &t.weight.to_string(),
        );
    }
    for r in &m.radius {
        push(
            &mut out,
            format!("radius.{}", r.name),
            css_var("radius", &r.name),
            &format!("{}px", fmt_num(r.px)),
        );
    }
    for s in &m.space {
        push(
            &mut out,
            format!("space.{}", s.name),
            css_var("space", &s.name),
            &format!("{}px", fmt_num(s.px)),
        );
    }
    for t in legacy_tokens(v) {
        if out
            .iter()
            .any(|k| k.path == t.path || k.css_var == t.css_var)
            || out.len() >= MAX_TOKENS
        {
            continue;
        }
        out.push(t);
    }
    out
}

const SKIP_GROUPS: &[&str] = &[
    "type",
    "version",
    "name",
    "description",
    "logos",
    "voice",
    "meta",
    "notes",
];

fn scalar(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => n.as_f64().filter(|f| f.is_finite()).map(|_| n.to_string()),
        _ => None,
    }
}

/// The lenient walker for kits that predate `otto-brand/1`.
fn legacy_tokens(doc: &Value) -> Vec<BrandToken> {
    let mut out = Vec::new();
    let Some(root) = doc.as_object() else {
        return out;
    };
    for (group, node) in root {
        if group.starts_with('$') || SKIP_GROUPS.contains(&group.as_str()) {
            continue;
        }
        walk_legacy(group, node, std::slice::from_ref(group), 0, &mut out);
    }
    out
}

fn push_legacy(path: &[String], raw: &str, out: &mut Vec<BrandToken>) {
    let segs: Vec<String> = path
        .iter()
        .map(|s| token_slug(s))
        .filter(|s| !s.is_empty())
        .collect();
    if segs.len() < 2 {
        return;
    }
    let p = segs.join(".");
    let value = css_value(raw);
    if value.is_empty() || out.iter().any(|t: &BrandToken| t.path == p) || out.len() >= MAX_TOKENS {
        return;
    }
    out.push(BrandToken {
        css_var: format!("--brand-{}", segs.join("-")),
        path: p,
        value,
    });
}

fn walk_legacy(
    group: &str,
    node: &Value,
    path: &[String],
    depth: usize,
    out: &mut Vec<BrandToken>,
) {
    if depth > 4 {
        return;
    }
    let Some(map) = node.as_object() else {
        return;
    };
    for (k, v) in map {
        if k.starts_with('$') {
            continue;
        }
        let mut here = path.to_vec();
        here.push(k.clone());
        match v.as_object().and_then(|o| o.get("$value")) {
            Some(val) => {
                if let Some(s) = scalar(val) {
                    push_legacy(&here, &s, out);
                } else if group == "typography" {
                    if let Some(fam) = val.get("fontFamily").and_then(scalar) {
                        let mut p = vec!["font".to_string()];
                        p.extend(here[1..].iter().cloned());
                        push_legacy(&p, &fam, out);
                    }
                }
            }
            None => match scalar(v) {
                Some(s) => push_legacy(&here, &s, out),
                None => walk_legacy(group, v, &here, depth + 1, out),
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Theme
// ---------------------------------------------------------------------------

impl Theme {
    /// Build from a brand document (`None` → the default palette).
    pub fn from_kit(kit: Option<&Value>) -> Theme {
        let tokens = tokens_of(kit);
        let find = |slot: &str| -> Option<BrandToken> {
            let none: &[&str] = &[];
            let sources: &[&str] = SLOTS
                .iter()
                .find(|(s, _)| *s == slot)
                .map(|(_, p)| *p)
                .unwrap_or(none);
            for p in sources {
                let as_var = format!("--brand-{}", p.replacen('.', "-", 1));
                if let Some(t) = tokens.iter().find(|t| t.path == *p || t.css_var == as_var) {
                    return Some(t.clone());
                }
            }
            match slot {
                "primary" => tokens
                    .iter()
                    .find(|t| t.path.starts_with("color."))
                    .cloned(),
                "radius" => tokens
                    .iter()
                    .find(|t| t.path.starts_with("radius."))
                    .cloned(),
                _ => None,
            }
        };
        let mut slot_css: Vec<(&'static str, String)> = Vec::new();
        let mut pick = |slot: &'static str, fallback: &str| -> (String, bool) {
            match find(slot) {
                Some(t) => {
                    slot_css.push((slot, var(&t.css_var)));
                    (t.value, true)
                }
                None => {
                    slot_css.push((slot, fallback.to_string()));
                    (fallback.to_string(), false)
                }
            }
        };
        let (primary, from_primary) = pick("primary", DEFAULT_PRIMARY);
        let (accent, _) = pick("accent", DEFAULT_ACCENT);
        let (ink, _) = pick("ink", DEFAULT_INK);
        let (surface, from_surface) = pick("surface", DEFAULT_SURFACE);
        let alt_default = if from_primary || from_surface {
            mix(&surface, &primary, 0.06)
        } else {
            DEFAULT_SURFACE_ALT.to_string()
        };
        let (surface_alt, _) = pick("surface-alt", &alt_default);
        pick("font-display", DEFAULT_FONT_DISPLAY);
        pick("font-body", DEFAULT_FONT_BODY);
        pick("radius", DEFAULT_RADIUS);

        let ls = luminance(&surface).unwrap_or(1.0);
        let li = luminance(&ink).unwrap_or(0.0);
        let (paper, night) = if ls >= li {
            (surface.clone(), ink.clone())
        } else {
            (ink.clone(), surface.clone())
        };
        let best = |bg: &str| -> String {
            if contrast(&paper, bg).unwrap_or(0.0) >= contrast(&night, bg).unwrap_or(0.0) {
                paper.clone()
            } else {
                night.clone()
            }
        };
        let on_primary = best(&primary);
        let on_accent = best(&accent);
        Theme {
            tokens,
            primary,
            accent,
            ink,
            surface,
            surface_alt,
            paper,
            night,
            on_primary,
            on_accent,
            slot_css,
        }
    }

    fn slot(&self, name: &str) -> String {
        self.slot_css
            .iter()
            .find(|(s, _)| *s == name)
            .map(|(_, c)| c.clone())
            .unwrap_or_default()
    }

    /// Every kit token as `--brand-*`, then the slots bound to them.
    pub fn declarations(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = self
            .tokens
            .iter()
            .map(|t| (t.css_var.clone(), t.value.clone()))
            .collect();
        let mut slot = |k: &str, v: String| out.push((k.to_string(), v));
        slot("--os-primary", self.slot("primary"));
        slot("--os-accent", self.slot("accent"));
        slot("--os-ink", self.slot("ink"));
        slot("--os-surface", self.slot("surface"));
        slot("--os-surface-alt", self.slot("surface-alt"));
        slot(
            "--os-paper",
            if self.paper == self.surface {
                var("--os-surface")
            } else {
                var("--os-ink")
            },
        );
        slot(
            "--os-night",
            if self.night == self.ink {
                var("--os-ink")
            } else {
                var("--os-surface")
            },
        );
        slot(
            "--os-on-primary",
            if self.on_primary == self.paper {
                var("--os-paper")
            } else {
                var("--os-night")
            },
        );
        slot(
            "--os-on-accent",
            if self.on_accent == self.paper {
                var("--os-paper")
            } else {
                var("--os-night")
            },
        );
        slot("--os-font-display", self.slot("font-display"));
        slot("--os-font-body", self.slot("font-body"));
        slot("--os-radius", self.slot("radius"));
        out
    }

    /// The `.os-site { … }` block the export's stylesheet starts with.
    pub fn css(&self) -> String {
        let body: Vec<String> = self
            .declarations()
            .into_iter()
            .map(|(k, v)| format!("  {k}: {v};"))
            .collect();
        format!(".os-site {{\n{}\n}}\n", body.join("\n"))
    }

    /// Page-level rules only a standalone document needs.
    pub fn document_css(&self) -> String {
        format!(
            "html {{ -webkit-text-size-adjust: 100%; }}\nbody {{ margin: 0; background: {}; }}\n",
            css_value(&self.surface)
        )
    }

    /// Light or dark text on `color`.
    pub fn tone_for(&self, color: &str) -> &'static str {
        if contrast(&self.night, color).unwrap_or(21.0)
            >= contrast(&self.paper, color).unwrap_or(0.0)
        {
            "light"
        } else {
            "dark"
        }
    }

    /// Resolve a section background (see [`Background`]).
    pub fn background(&self, bg: &str) -> Background {
        let v = bg.trim();
        let base = |color: &str, css: Option<String>, preset: Option<&'static str>| Background {
            css,
            preset,
            tone: self.tone_for(color),
        };
        if v.is_empty() {
            return base(&self.surface, None, None);
        }
        if let Some(path) = v.strip_prefix("token:") {
            let path = path.trim();
            let (group, name) = path.split_once('.').unwrap_or((path, ""));
            let as_var = format!("--brand-{group}-{}", css_ident(name));
            if let Some(t) = self
                .tokens
                .iter()
                .find(|t| t.path == path)
                .or_else(|| self.tokens.iter().find(|t| t.css_var == as_var))
            {
                let color = if parse_color(&t.value).is_some() {
                    t.value.clone()
                } else {
                    self.surface.clone()
                };
                return base(&color, Some(var(&t.css_var)), None);
            }
            let alias: Option<(&str, &str)> = match path {
                "color.primary" => Some((self.primary.as_str(), "--os-primary")),
                "color.accent" => Some((self.accent.as_str(), "--os-accent")),
                "color.ink" => Some((self.ink.as_str(), "--os-ink")),
                "color.surface" => Some((self.surface.as_str(), "--os-surface")),
                "color.surface-alt" => Some((self.surface_alt.as_str(), "--os-surface-alt")),
                _ => None,
            };
            return match alias {
                Some((color, slot)) => base(color, Some(var(slot)), None),
                None => base(&self.surface, None, None),
            };
        }
        if let Some(p) = v.strip_prefix("gradient:") {
            let (color, css, preset): (&str, &str, &'static str) = match p {
                "soft" => (self.surface_alt.as_str(), "--os-surface-alt", "soft"),
                "ink" => (self.night.as_str(), "--os-night", "ink"),
                "primary" => (self.primary.as_str(), "--os-primary", "primary"),
                "sunset" => (self.primary.as_str(), "--os-primary", "sunset"),
                _ => return base(&self.surface, None, None),
            };
            return base(color, Some(var(css)), Some(preset));
        }
        if parse_color(v).is_some() {
            return base(v, Some(css_value(v)), None);
        }
        base(&self.surface, None, None)
    }
}

/// A resolved section background.
#[derive(Debug, Clone, PartialEq)]
pub struct Background {
    /// Value for `--os-bg` (None = the default surface).
    pub css: Option<String>,
    /// Gradient preset class suffix (`os-bg-<preset>`).
    pub preset: Option<&'static str>,
    /// `light` (dark text) | `dark` (light text).
    pub tone: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn kit() -> Value {
        json!({
            "$schema": "otto-brand/1",
            "name": "Teal",
            "color": {
                "primary": { "$value": "#0F766E" },
                "accent": { "$value": "#F59E0B" },
                "ink": { "$value": "#0B1220" },
                "surface": { "$value": "#FFFFFF" },
                "surfaceAlt": { "$value": "#ECFDF5" }
            },
            "font": { "display": { "$value": "\"Clash Display\", system-ui" } },
            "type": { "display": { "size": 64, "line": 72, "weight": 800 } },
            "radius": { "md": { "$value": 14 } },
            "space": { "m": { "$value": 16 } },
            "voice": { "summary": "Warm" }
        })
    }

    #[test]
    fn a_v1_kit_becomes_brand_vars_and_bound_slots() {
        let t = Theme::from_kit(Some(&kit()));
        let css = t.css();
        assert!(
            css.starts_with(".os-site {\n  --brand-color-primary: #0F766E;"),
            "{css}"
        );
        for want in [
            "--brand-color-surface-alt: #ECFDF5;",
            "--brand-font-display: \"Clash Display\", system-ui;",
            "--brand-type-display-size: 64px;",
            "--brand-radius-md: 14px;",
            "--brand-space-m: 16px;",
            "--os-primary: var(--brand-color-primary);",
            "--os-surface-alt: var(--brand-color-surface-alt);",
            "--os-radius: var(--brand-radius-md);",
            "--os-font-body: Inter,",
        ] {
            assert!(css.contains(want), "{want} missing in\n{css}");
        }
        assert!(!css.contains("voice"), "non-token groups never become vars");
        assert_eq!(t.primary, "#0F766E");
        assert_eq!(t.paper, "#FFFFFF");
        assert_eq!(t.night, "#0B1220");
    }

    #[test]
    fn no_kit_means_defaults_and_legacy_kits_still_read() {
        let t = Theme::from_kit(None);
        assert!(t.tokens.is_empty());
        assert_eq!(t.primary, DEFAULT_PRIMARY);
        assert_eq!(t.surface_alt, DEFAULT_SURFACE_ALT);
        assert!(t.css().contains("--os-primary: #5B3DF5;"));
        let legacy = json!({ "type": "otto-brand", "version": 1,
            "typography": { "display": { "$value": { "fontFamily": "Georgia" } } },
            "radius": { "card": { "$value": "20px" } } });
        let l = Theme::from_kit(Some(&legacy));
        let css = l.css();
        assert!(css.contains("--brand-font-display: Georgia;"), "{css}");
        assert!(
            css.contains("--os-radius: var(--brand-radius-card);"),
            "{css}"
        );
    }

    #[test]
    fn backgrounds_resolve_to_vars_with_a_tone() {
        let t = Theme::from_kit(Some(&kit()));
        let b = t.background("token:color.surfaceAlt");
        assert_eq!(b.css.as_deref(), Some("var(--brand-color-surface-alt)"));
        assert_eq!(b.tone, "light");
        assert_eq!(t.background("token:color.ink").tone, "dark");
        let g = t.background("gradient:ink");
        assert_eq!((g.preset, g.tone), (Some("ink"), "dark"));
        assert_eq!(t.background("#FFD166").css.as_deref(), Some("#FFD166"));
        assert_eq!(t.background("token:color.nope").css, None);
        assert_eq!(t.background("").css, None);
    }

    #[test]
    fn colour_math_and_sanitizing() {
        assert_eq!(contrast("#000", "#fff"), Some(21.0));
        assert_eq!(parse_color("rgb(10, 20, 30)"), Some([10.0, 20.0, 30.0]));
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(mix("#ffffff", "#000000", 0.5), "#808080");
        assert_eq!(css_value("red; } body { x: y"), "red  body  x: y");
        assert_eq!(token_slug("surfaceAlt"), "surface-alt");
        assert_eq!(token_slug("Surface  Alt!"), "surface-alt");
    }
}
