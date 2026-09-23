//! The `otto-site` HTML renderer — the export/preview twin of
//! `ui/src/modules/design-hall/site/engine/render.ts` (which also renders the
//! editor canvas). Both emit the same semantic markup and `site.css` classes
//! for the same document; change the two together.
//!
//! Everything authored is escaped; links go through [`safe_href`] (http(s),
//! mailto, tel, relative, `#anchor`, `page:<id>` — anything else is `#`),
//! images through [`safe_src`], videos through [`safe_video`]. No script is
//! ever emitted: motion presets are pure CSS. Pure; unit-tested.

use std::collections::HashMap;

use serde_json::{Map, Value};

use super::schema::{media_slot, Block, Page, Section, SiteDoc, SECTION_BLOCKS};
use super::theme::Theme;
use super::validate::is_data_image;

type Props = Map<String, Value>;

/// The feature-icon set (24×24, 2 px strokes) — `SITE_ICONS` in
/// `ui/src/modules/design-hall/site/engine/icons.ts`, verbatim.
pub const ICONS: &[(&str, &str)] = &[
    ("bolt", "M13 2 3 14h9l-1 8 10-12h-9l1-8z"),
    ("star", "m12 2 3.09 6.26L22 9.27l-5 4.87 1.18 6.88L12 17.77l-6.18 3.25L7 14.14 2 9.27l6.91-1.01L12 2z"),
    ("shield", "M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"),
    ("heart", "M20.8 4.6a5.5 5.5 0 0 0-7.8 0L12 5.7l-1-1.1a5.5 5.5 0 0 0-7.8 7.8l1 1.1 7.8 7.7 7.8-7.7 1-1.1a5.5 5.5 0 0 0 0-7.8z"),
    ("chart", "M3 3v18h18M7 14l4-4 4 4 5-6"),
    ("clock", "M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20zM12 6v6l4 2"),
    ("gift", "M20 12v10H4V12M2 7h20v5H2zM12 22V7M12 7H7.5a2.5 2.5 0 0 1 0-5C11 2 12 7 12 7zM12 7h4.5a2.5 2.5 0 0 0 0-5C13 2 12 7 12 7z"),
    ("globe", "M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20zM2 12h20M12 2a15 15 0 0 1 4 10 15 15 0 0 1-4 10 15 15 0 0 1-4-10 15 15 0 0 1 4-10z"),
    ("lock", "M5 11h14v11H5zM7 11V7a5 5 0 0 1 10 0v4"),
    ("sparkle", "M12 3l1.9 5.1L19 10l-5.1 1.9L12 17l-1.9-5.1L5 10l5.1-1.9L12 3zM19 15l.9 2.1L22 18l-2.1.9L19 21l-.9-2.1L16 18l2.1-.9L19 15z"),
    ("check", "M20 6 9 17l-5-5"),
    ("layers", "m12 2 10 5-10 5L2 7l10-5zM2 17l10 5 10-5M2 12l10 5 10-5"),
    ("users", "M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2M9 11a4 4 0 1 0 0-8 4 4 0 0 0 0 8zM23 21v-2a4 4 0 0 0-3-3.9M16 3.1a4 4 0 0 1 0 7.8"),
    ("infinity", "M12 12c-2-2.7-4-4-6-4a4 4 0 0 0 0 8c2 0 4-1.3 6-4zm0 0c2 2.7 4 4 6 4a4 4 0 0 0 0-8c-2 0-4 1.3-6 4z"),
    ("code", "m16 18 6-6-6-6M8 6l-6 6 6 6"),
    ("book", "M4 19.5A2.5 2.5 0 0 1 6.5 17H20V2H6.5A2.5 2.5 0 0 0 4 4.5v15zM20 17v5H6.5A2.5 2.5 0 0 1 4 19.5"),
    ("calendar", "M3 5h18v16H3zM16 3v4M8 3v4M3 10h18"),
    ("pin", "M12 22s7-6.2 7-12a7 7 0 0 0-14 0c0 5.8 7 12 7 12zM12 12.5a2.5 2.5 0 1 0 0-5 2.5 2.5 0 0 0 0 5z"),
    ("mail", "M3 5h18v14H3zM3 6l9 7 9-7"),
    ("rocket", "M5 15c-1.5 1.3-2 5-2 5s3.7-.5 5-2c.7-.8.7-2.1-.1-2.9a2.2 2.2 0 0 0-2.9-.1zM12 15l-3-3a22 22 0 0 1 2-4A12.9 12.9 0 0 1 22 2c0 2.7-.8 7.5-6 11a22 22 0 0 1-4 2zM9 12H4s.6-3 2-4c1.6-1.1 5 0 5 0M12 15v5s3-.6 4-2c1.1-1.6 0-5 0-5"),
];
const ARROW_PATH: &str = "M5 12h14M13 6l6 6-6 6";
const PLAY_PATH: &str = "M8 5v14l11-7z";

fn icon_path(name: &str) -> Option<&'static str> {
    ICONS.iter().find(|(n, _)| *n == name).map(|(_, p)| *p)
}

/// What a 3D embed shows (resolved by the host: title, version, poster).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EmbedInfo {
    pub title: String,
    pub seq: Option<i64>,
    pub policy: String,
    /// A poster URL (an exported asset path or a data URI).
    pub poster: Option<String>,
    pub broken: bool,
}

/// Everything the renderer needs from its host.
#[derive(Debug, Clone)]
pub struct RenderCtx<'a> {
    pub theme: &'a Theme,
    /// Page id → its URL (`page:<id>` links).
    pub pages: HashMap<String, String>,
    pub home_href: String,
    /// `otto://design/…` image → URL.
    pub assets: HashMap<String, String>,
    /// `otto://design/…` 3D artifact → what the embed shows.
    pub embeds: HashMap<String, EmbedInfo>,
}

impl RenderCtx<'_> {
    fn page_href(&self, id: &str) -> String {
        self.pages.get(id).cloned().unwrap_or_else(|| "#".into())
    }
}

// ---------------------------------------------------------------------------
// Primitives
// ---------------------------------------------------------------------------

pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Escaped text with line breaks kept (`\n` → `<br>`).
pub fn esc_lines(s: &str) -> String {
    esc(s).replace("\r\n", "<br>").replace('\n', "<br>")
}

/// Lower-cased with every control / space character removed (what a browser
/// ignores inside a scheme: `java\tscript:`).
fn probe(s: &str) -> String {
    s.chars()
        .filter(|c| (*c as u32) > 0x20)
        .collect::<String>()
        .to_ascii_lowercase()
}

/// `^[a-z][a-z0-9+.-]*:`.
fn has_scheme(p: &str) -> bool {
    let mut chars = p.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    for c in chars {
        if c == ':' {
            return true;
        }
        if !(c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-')) {
            return false;
        }
    }
    false
}

/// The link sanitizer (see the module docs).
pub fn safe_href(raw: &str, ctx: &RenderCtx) -> String {
    let s = raw.trim();
    if s.is_empty() {
        return "#".into();
    }
    if let Some(id) = s.strip_prefix("page:") {
        return ctx.page_href(id);
    }
    let p = probe(s);
    if p.starts_with("http:")
        || p.starts_with("https:")
        || p.starts_with("mailto:")
        || p.starts_with("tel:")
    {
        return s.to_string();
    }
    if has_scheme(&p) || p.starts_with("//") || p.starts_with('\\') {
        return "#".into();
    }
    s.to_string()
}

/// The image sanitizer: otto:// (resolved by the host), https, data:image, relative.
pub fn safe_src(raw: &str, ctx: &RenderCtx) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    if s.starts_with(crate::uri::PREFIX) {
        return ctx.assets.get(s).cloned();
    }
    let p = probe(s);
    if p.starts_with("https:") {
        return Some(s.to_string());
    }
    if p.starts_with("data:") {
        return is_data_image(s).then(|| s.to_string());
    }
    if has_scheme(&p) || p.starts_with("//") || p.starts_with('\\') {
        return None;
    }
    Some(s.to_string())
}

/// Video sources: https or relative only.
pub fn safe_video(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    let p = probe(s);
    if p.starts_with("https:") {
        return Some(s.to_string());
    }
    if has_scheme(&p) || p.starts_with("//") || p.starts_with('\\') {
        return None;
    }
    Some(s.to_string())
}

/// A string (or number) prop, else `""`.
pub fn text(p: &Props, key: &str) -> String {
    match p.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn flag(p: &Props, key: &str, dflt: bool) -> bool {
    p.get(key).and_then(Value::as_bool).unwrap_or(dflt)
}

/// A list prop (`["a","b"]`, or newline-separated text), empties dropped.
pub fn lines(p: &Props, key: &str) -> Vec<String> {
    match p.get(key) {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string)
            .collect(),
        Some(Value::String(s)) => s
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

/// A `[{label, href}]` prop.
pub fn links(p: &Props, key: &str) -> Vec<(String, String)> {
    let Some(Value::Array(a)) = p.get(key) else {
        return Vec::new();
    };
    a.iter()
        .filter_map(Value::as_object)
        .filter_map(|o| {
            let label = o.get("label")?.as_str()?.to_string();
            let href = o
                .get("href")
                .and_then(Value::as_str)
                .unwrap_or("#")
                .to_string();
            Some((label, href))
        })
        .collect()
}

/// "Priya Natarajan" → "PN".
pub fn initials(name: &str) -> String {
    name.split_whitespace()
        .take(2)
        .filter_map(|w| w.chars().next())
        .collect::<String>()
        .to_uppercase()
}

fn svg(path: &str, cls: &str) -> String {
    format!("<svg class=\"{cls}\" viewBox=\"0 0 24 24\" aria-hidden=\"true\"><path d=\"{path}\"/></svg>")
}

/// A feature icon: a named glyph, else up to 3 characters of text.
fn icon(name: &str) -> String {
    let key = name.trim();
    if key.is_empty() {
        return String::new();
    }
    match icon_path(key) {
        Some(p) => format!(
            "<span class=\"os-icon\" aria-hidden=\"true\">{}</span>",
            svg(p, "os-ico")
        ),
        None => {
            let short: String = key.chars().take(3).collect();
            format!(
                "<span class=\"os-icon os-icon--text\" aria-hidden=\"true\">{}</span>",
                esc(&short)
            )
        }
    }
}

fn text_el(tag: &str, cls: &str, p: &Props, key: &str, multi: bool) -> String {
    let v = text(p, key);
    if v.trim().is_empty() {
        return String::new();
    }
    let body = if multi { esc_lines(&v) } else { esc(&v) };
    format!("<{tag} class=\"{cls}\">{body}</{tag}>")
}

fn button(
    ctx: &RenderCtx,
    p: &Props,
    label_key: &str,
    href_key: &str,
    kind: &str,
    extra: &str,
) -> String {
    let label = text(p, label_key);
    if label.trim().is_empty() {
        return String::new();
    }
    let arrow = if kind == "ghost" {
        svg(ARROW_PATH, "os-ico os-ico--arrow")
    } else {
        String::new()
    };
    let cls = if extra.is_empty() {
        format!("os-btn os-btn--{kind}")
    } else {
        format!("os-btn os-btn--{kind} {extra}")
    };
    format!(
        "<a class=\"{cls}\" href=\"{}\"><span>{}</span>{arrow}</a>",
        esc(&safe_href(&text(p, href_key), ctx)),
        esc(&label)
    )
}

fn actions(ctx: &RenderCtx, p: &Props) -> String {
    let a = button(ctx, p, "primary_label", "primary_href", "primary", "");
    let b = button(ctx, p, "secondary_label", "secondary_href", "ghost", "");
    if a.is_empty() && b.is_empty() {
        String::new()
    } else {
        format!("<div class=\"os-actions\">{a}{b}</div>")
    }
}

/// Kicker + h2 + subheading (all empty → nothing).
fn head(p: &Props) -> String {
    let k = text_el("p", "os-kicker", p, "eyebrow", false);
    let t = text_el("h2", "os-h2", p, "heading", true);
    let s = text_el("p", "os-sub", p, "subheading", true);
    if k.is_empty() && t.is_empty() && s.is_empty() {
        String::new()
    } else {
        format!("<header class=\"os-head\">{k}{t}{s}</header>")
    }
}

fn items<'a>(s: &'a Section, block: &str) -> Vec<&'a Block> {
    s.blocks.iter().filter(|b| b.block == block).collect()
}

fn i_style(i: usize) -> String {
    format!(" style=\"--os-i: {i}\"")
}

fn logo(ctx: &RenderCtx, p: &Props) -> String {
    format!(
        "<a class=\"os-logo\" href=\"{}\"><span class=\"os-logo__mark\" aria-hidden=\"true\"></span><span>{}</span></a>",
        esc(&ctx.home_href),
        esc(&text(p, "logo"))
    )
}

fn checks(list: &[String], cls: &str) -> String {
    if list.is_empty() {
        return String::new();
    }
    let li: String = list
        .iter()
        .map(|x| {
            format!(
                "<li>{}<span>{}</span></li>",
                svg(icon_path("check").unwrap_or(""), "os-ico"),
                esc(x)
            )
        })
        .collect();
    format!("<ul class=\"{cls}\">{li}</ul>")
}

fn price(p: &Props) -> String {
    let period = text(p, "period");
    let per = if period.is_empty() {
        String::new()
    } else {
        format!("<span class=\"os-price__p\">{}</span>", esc(&period))
    };
    format!(
        "<p class=\"os-price\"><span class=\"os-price__v\">{}</span>{per}</p>",
        esc(&text(p, "price"))
    )
}

// ---------------------------------------------------------------------------
// Media
// ---------------------------------------------------------------------------

fn embed3d(ctx: &RenderCtx, b: &Block) -> String {
    let src = text(&b.props, "src");
    let info = if src.is_empty() {
        None
    } else {
        ctx.embeds.get(src.trim())
    };
    let title = info
        .map(|i| i.title.clone())
        .filter(|t| !t.is_empty())
        .or_else(|| Some(text(&b.props, "label")).filter(|t| !t.is_empty()))
        .unwrap_or_else(|| "Your product in 3D".into());
    let alt = Some(text(&b.props, "alt"))
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| title.clone());
    let mut cls = String::from("os-embed os-embed-3d");
    if flag(&b.props, "auto_rotate", true) {
        cls.push_str(" os-embed--spin");
    }
    if flag(&b.props, "tilt_on_hover", true) {
        cls.push_str(" os-embed--tilt");
    }
    let stage = match info.and_then(|i| i.poster.clone()) {
        Some(poster) => format!(
            "<img class=\"os-embed-3d__poster\" src=\"{}\" alt=\"{}\" loading=\"lazy\">",
            esc(&poster),
            esc(&alt)
        ),
        None => format!(
            "<div class=\"os-embed-3d__art\" role=\"img\" aria-label=\"{}\"><span class=\"os-orb\"></span><span class=\"os-ring\"></span><span class=\"os-card3d\"><span class=\"os-card3d__chip\"></span><span class=\"os-card3d__name\">{}</span></span></div>",
            esc(&alt),
            esc(&title)
        ),
    };
    let cap = text_el("figcaption", "os-caption", &b.props, "caption", false);
    format!("<figure class=\"{cls}\"><div class=\"os-embed-3d__stage\">{stage}</div>{cap}</figure>")
}

fn image_figure(ctx: &RenderCtx, b: &Block, cls: &str, i: Option<usize>) -> String {
    let alt = text(&b.props, "alt");
    let img = match safe_src(&text(&b.props, "src"), ctx) {
        Some(src) => format!(
            "<img class=\"os-img\" src=\"{}\" alt=\"{}\" loading=\"lazy\">",
            esc(&src),
            esc(&alt)
        ),
        None => format!(
            "<div class=\"os-img os-img--empty\" role=\"img\" aria-label=\"{}\"></div>",
            esc(if alt.is_empty() {
                "Image placeholder"
            } else {
                alt.as_str()
            })
        ),
    };
    let cap = text_el("figcaption", "os-caption", &b.props, "caption", false);
    let style = i.map(i_style).unwrap_or_default();
    format!("<figure class=\"{cls}\"{style}>{img}{cap}</figure>")
}

fn media(ctx: &RenderCtx, s: &Section) -> String {
    let allowed = media_slot(&s.block);
    match s
        .blocks
        .iter()
        .find(|b| allowed.contains(&b.block.as_str()))
    {
        Some(b) if b.block == "embed/3d" => embed3d(ctx, b),
        Some(b) => image_figure(ctx, b, "os-embed os-embed-img", None),
        None => String::new(),
    }
}

/// A feature's picture for rows / bento: its image, else a big icon tile.
fn feature_visual(ctx: &RenderCtx, b: &Block) -> String {
    if let Some(src) = safe_src(&text(&b.props, "image"), ctx) {
        return format!(
            "<img class=\"os-img\" src=\"{}\" alt=\"{}\" loading=\"lazy\">",
            esc(&src),
            esc(&text(&b.props, "title"))
        );
    }
    let path = icon_path(text(&b.props, "icon").trim())
        .or_else(|| icon_path("sparkle"))
        .unwrap_or("");
    format!(
        "<div class=\"os-img os-img--empty os-img--icon\" aria-hidden=\"true\">{}</div>",
        svg(path, "os-ico")
    )
}

// ---------------------------------------------------------------------------
// Section bodies
// ---------------------------------------------------------------------------

const DECO: &str = "<div class=\"os-deco\" aria-hidden=\"true\"><span class=\"os-deco__a\"></span><span class=\"os-deco__b\"></span></div>";

fn feature(b: &Block, i: usize) -> String {
    format!(
        "<article class=\"os-card os-feature os-item\"{}>{}{}{}</article>",
        i_style(i),
        icon(&text(&b.props, "icon")),
        text_el("h3", "os-h3", &b.props, "title", false),
        text_el("p", "os-text", &b.props, "body", true)
    )
}

fn tier(ctx: &RenderCtx, b: &Block, i: usize) -> String {
    let p = &b.props;
    let hl = flag(p, "highlight", false);
    format!(
        "<article class=\"os-card os-tier os-item{}\"{}>{}{}{}{}{}{}</article>",
        if hl { " os-tier--hl" } else { "" },
        i_style(i),
        text_el("span", "os-badge", p, "badge", false),
        text_el("h3", "os-tier__name", p, "name", false),
        price(p),
        text_el("p", "os-text", p, "blurb", true),
        checks(&lines(p, "features"), "os-checks"),
        button(
            ctx,
            p,
            "cta_label",
            "cta_href",
            if hl { "primary" } else { "ghost" },
            "os-btn--block"
        )
    )
}

fn person(p: &Props) -> String {
    let name = text(p, "name");
    if name.trim().is_empty() && text(p, "role").trim().is_empty() {
        return String::new();
    }
    format!(
        "<figcaption class=\"os-person\"><span class=\"os-avatar\" aria-hidden=\"true\">{}</span><span class=\"os-person__who\">{}{}</span></figcaption>",
        esc(&initials(&name)),
        text_el("strong", "os-person__name", p, "name", false),
        text_el("span", "os-person__role", p, "role", false)
    )
}

fn body(ctx: &RenderCtx, s: &Section) -> String {
    let p = &s.props;
    match s.block.as_str() {
        "nav/bar" => {
            let nav: String = items(s, "item/link")
                .iter()
                .map(|b| {
                    format!(
                        "<a class=\"os-nav__link\" href=\"{}\"><span>{}</span></a>",
                        esc(&safe_href(&text(&b.props, "href"), ctx)),
                        esc(&text(&b.props, "label"))
                    )
                })
                .collect();
            let nav = if nav.is_empty() {
                String::new()
            } else {
                format!("<nav class=\"os-nav__links\" aria-label=\"Main\">{nav}</nav>")
            };
            format!(
                "<div class=\"os-wrap os-nav__bar\">{}{nav}{}</div>",
                logo(ctx, p),
                button(ctx, p, "cta_label", "cta_href", "primary", "os-btn--sm")
            )
        }
        "hero/split" | "hero/centered" | "hero/fullbleed" | "hero/stacked" => {
            let m = media(ctx, s);
            let eyebrow = text(p, "eyebrow");
            let eyebrow = if eyebrow.trim().is_empty() {
                String::new()
            } else {
                format!("<p class=\"os-eyebrow\"><span>{}</span></p>", esc(&eyebrow))
            };
            let text_col = format!(
                "<div class=\"os-hero__text\">{eyebrow}{}{}{}{}</div>",
                text_el("h1", "os-h1", p, "headline", true),
                text_el("p", "os-lead", p, "subhead", true),
                actions(ctx, p),
                text_el("p", "os-trust", p, "trust", false)
            );
            let (grid_cls, media_col) = if m.is_empty() {
                ("", String::new())
            } else {
                (" os-hero__grid--media", format!("<div class=\"os-hero__media\">{m}</div>"))
            };
            format!("{DECO}<div class=\"os-wrap os-hero__grid{grid_cls}\">{text_col}{media_col}</div>")
        }
        "features/grid" => {
            let cards: String = items(s, "item/feature")
                .iter()
                .enumerate()
                .map(|(i, b)| feature(b, i))
                .collect();
            format!("<div class=\"os-wrap\">{}<div class=\"os-grid\">{cards}</div></div>", head(p))
        }
        "features/alternating" => {
            let rows: String = items(s, "item/feature")
                .iter()
                .enumerate()
                .map(|(i, b)| {
                    format!(
                        "<div class=\"os-row os-item\"{}><div class=\"os-row__media\">{}</div><div class=\"os-row__text\">{}{}{}</div></div>",
                        i_style(i),
                        feature_visual(ctx, b),
                        icon(&text(&b.props, "icon")),
                        text_el("h3", "os-h3", &b.props, "title", false),
                        text_el("p", "os-text", &b.props, "body", true)
                    )
                })
                .collect();
            format!("<div class=\"os-wrap\">{}<div class=\"os-rows\">{rows}</div></div>", head(p))
        }
        "features/bento" => {
            let cells: String = items(s, "item/feature")
                .iter()
                .enumerate()
                .map(|(i, b)| {
                    let lead = if i == 0 { " os-bento__lead" } else { "" };
                    let visual = if i == 0 {
                        format!("<div class=\"os-bento__visual\">{}</div>", feature_visual(ctx, b))
                    } else {
                        icon(&text(&b.props, "icon"))
                    };
                    format!(
                        "<article class=\"os-card os-bento__cell os-item{lead}\"{}>{visual}{}{}</article>",
                        i_style(i),
                        text_el("h3", "os-h3", &b.props, "title", false),
                        text_el("p", "os-text", &b.props, "body", true)
                    )
                })
                .collect();
            format!("<div class=\"os-wrap\">{}<div class=\"os-bento\">{cells}</div></div>", head(p))
        }
        "features/steps" => {
            let steps: String = items(s, "item/step")
                .iter()
                .enumerate()
                .map(|(i, b)| {
                    format!(
                        "<li class=\"os-step os-item\"{}><span class=\"os-step__n\" aria-hidden=\"true\">{}</span>{}{}</li>",
                        i_style(i),
                        i + 1,
                        text_el("h3", "os-h3", &b.props, "title", false),
                        text_el("p", "os-text", &b.props, "body", true)
                    )
                })
                .collect();
            format!("<div class=\"os-wrap\">{}<ol class=\"os-steps\">{steps}</ol></div>", head(p))
        }
        "features/stats" => {
            let stats: String = items(s, "item/stat")
                .iter()
                .enumerate()
                .map(|(i, b)| {
                    format!(
                        "<div class=\"os-stat os-item\"{}>{}{}</div>",
                        i_style(i),
                        text_el("span", "os-stat__v", &b.props, "value", false),
                        text_el("span", "os-stat__l", &b.props, "label", false)
                    )
                })
                .collect();
            format!("{DECO}<div class=\"os-wrap\">{}<div class=\"os-stats\">{stats}</div></div>", head(p))
        }
        "social/logos" => {
            let list: String = items(s, "item/logo")
                .iter()
                .enumerate()
                .map(|(i, b)| {
                    let name = text(&b.props, "name");
                    let inner = match safe_src(&text(&b.props, "image"), ctx) {
                        Some(src) => format!(
                            "<img class=\"os-logos__img\" src=\"{}\" alt=\"{}\" loading=\"lazy\">",
                            esc(&src),
                            esc(&name)
                        ),
                        None => format!("<span>{}</span>", esc(&name)),
                    };
                    format!("<li class=\"os-logos__item os-item\"{}>{inner}</li>", i_style(i))
                })
                .collect();
            format!(
                "<div class=\"os-wrap os-logos\">{}<ul class=\"os-logos__list\">{list}</ul></div>",
                text_el("p", "os-logos__label", p, "label", false)
            )
        }
        "social/testimonials" => {
            let cards: String = items(s, "item/testimonial")
                .iter()
                .enumerate()
                .map(|(i, b)| {
                    format!(
                        "<figure class=\"os-card os-quote os-item\"{}>{}{}</figure>",
                        i_style(i),
                        text_el("blockquote", "os-quote__text", &b.props, "quote", true),
                        person(&b.props)
                    )
                })
                .collect();
            format!("<div class=\"os-wrap\">{}<div class=\"os-quotes\">{cards}</div></div>", head(p))
        }
        "social/quote" => format!(
            "<div class=\"os-wrap\"><figure class=\"os-bigquote\">{}{}</figure></div>",
            text_el("blockquote", "os-bigquote__text", p, "quote", true),
            person(p)
        ),
        "pricing/tiers" => {
            let tiers: String = items(s, "item/tier")
                .iter()
                .enumerate()
                .map(|(i, b)| tier(ctx, b, i))
                .collect();
            format!("<div class=\"os-wrap\">{}<div class=\"os-tiers\">{tiers}</div></div>", head(p))
        }
        "pricing/compare" => {
            let tiers = items(s, "item/tier");
            let mut rows: Vec<String> = Vec::new();
            for t in &tiers {
                for x in lines(&t.props, "features") {
                    if !rows.contains(&x) {
                        rows.push(x);
                    }
                }
            }
            let th: String = tiers
                .iter()
                .map(|b| {
                    let period = text(&b.props, "period");
                    let small = if period.is_empty() {
                        String::new()
                    } else {
                        format!("<small>{}</small>", esc(&period))
                    };
                    format!(
                        "<th scope=\"col\"{}><span class=\"os-compare__name\">{}</span><span class=\"os-compare__price\">{}{small}</span></th>",
                        if flag(&b.props, "highlight", false) { " class=\"os-hl\"" } else { "" },
                        esc(&text(&b.props, "name")),
                        esc(&text(&b.props, "price"))
                    )
                })
                .collect();
            let body: String = rows
                .iter()
                .map(|row| {
                    let cells: String = tiers
                        .iter()
                        .map(|b| {
                            if lines(&b.props, "features").contains(row) {
                                format!(
                                    "<td>{}<span class=\"os-sr\">Included</span></td>",
                                    svg(icon_path("check").unwrap_or(""), "os-ico os-ico--yes")
                                )
                            } else {
                                "<td><span class=\"os-dash\" aria-hidden=\"true\">—</span><span class=\"os-sr\">Not included</span></td>".to_string()
                            }
                        })
                        .collect();
                    format!("<tr><th scope=\"row\">{}</th>{cells}</tr>", esc(row))
                })
                .collect();
            let foot: String = tiers
                .iter()
                .map(|b| {
                    let kind = if flag(&b.props, "highlight", false) { "primary" } else { "ghost" };
                    format!("<td>{}</td>", button(ctx, &b.props, "cta_label", "cta_href", kind, "os-btn--sm"))
                })
                .collect();
            format!(
                "<div class=\"os-wrap\">{}<div class=\"os-table-wrap\"><table class=\"os-compare\"><thead><tr><th scope=\"col\"><span class=\"os-sr\">Feature</span></th>{th}</tr></thead><tbody>{body}</tbody><tfoot><tr><td></td>{foot}</tr></tfoot></table></div></div>",
                head(p)
            )
        }
        "pricing/single" => format!(
            "<div class=\"os-wrap\">{}<article class=\"os-card os-single\"><div class=\"os-single__top\">{}{}</div>{}{}{}</article></div>",
            head(p),
            text_el("h3", "os-tier__name", p, "name", false),
            text_el("span", "os-badge", p, "badge", false),
            price(p),
            checks(&lines(p, "features"), "os-checks os-checks--2"),
            button(ctx, p, "cta_label", "cta_href", "primary", "os-btn--block")
        ),
        "faq/accordion" => {
            let qa: String = items(s, "item/faq")
                .iter()
                .enumerate()
                .map(|(i, b)| {
                    format!(
                        "<details class=\"os-qa os-item\"{}><summary class=\"os-qa__q\"><span>{}</span><span class=\"os-qa__icon\" aria-hidden=\"true\"></span></summary>{}</details>",
                        i_style(i),
                        esc(&text(&b.props, "question")),
                        text_el("p", "os-qa__a", &b.props, "answer", true)
                    )
                })
                .collect();
            format!(
                "<div class=\"os-wrap os-faq__grid\"><div class=\"os-faq__head\">{}</div><div class=\"os-faq__list\">{qa}</div></div>",
                head(p)
            )
        }
        "faq/grid" => {
            let qa: String = items(s, "item/faq")
                .iter()
                .enumerate()
                .map(|(i, b)| {
                    format!(
                        "<div class=\"os-faqgrid__item os-item\"{}>{}{}</div>",
                        i_style(i),
                        text_el("dt", "os-h3", &b.props, "question", false),
                        text_el("dd", "os-text", &b.props, "answer", true)
                    )
                })
                .collect();
            format!("<div class=\"os-wrap\">{}<dl class=\"os-faqgrid\">{qa}</dl></div>", head(p))
        }
        "cta/band" => format!(
            "{DECO}<div class=\"os-wrap os-ctaband\">{}{}{}</div>",
            text_el("h2", "os-h2 os-h2--xl", p, "headline", true),
            text_el("p", "os-lead", p, "subhead", true),
            actions(ctx, p)
        ),
        "cta/split" => {
            let action = text(p, "form_action");
            let action = action.trim();
            let ok = action
                .get(..8)
                .is_some_and(|p| p.eq_ignore_ascii_case("https://"));
            let fid = format!("{}-email", s.id);
            let label = Some(text(p, "button_label")).filter(|l| !l.is_empty()).unwrap_or_else(|| "Submit".into());
            let form = format!(
                "<form class=\"os-form\"{}><label class=\"os-sr\" for=\"{fid}\">Email address</label><input class=\"os-input\" id=\"{fid}\" type=\"email\" name=\"email\" autocomplete=\"email\" placeholder=\"{}\"><button class=\"os-btn os-btn--primary\" type=\"{}\"><span>{}</span></button>{}</form>",
                if ok { format!(" method=\"post\" action=\"{}\"", esc(action)) } else { String::new() },
                esc(&text(p, "placeholder")),
                if ok { "submit" } else { "button" },
                esc(&label),
                text_el("p", "os-note", p, "note", false),
                fid = esc(&fid)
            );
            format!(
                "{DECO}<div class=\"os-wrap os-ctasplit\"><div class=\"os-ctasplit__text\">{}{}</div>{form}</div>",
                text_el("h2", "os-h2", p, "headline", true),
                text_el("p", "os-lead", p, "subhead", true)
            )
        }
        "cta/card" => format!(
            "<div class=\"os-wrap\"><div class=\"os-card os-ctacard\">{DECO}{}{}{}<div class=\"os-actions\">{}</div></div></div>",
            text_el("p", "os-kicker", p, "eyebrow", false),
            text_el("h2", "os-h2", p, "headline", true),
            text_el("p", "os-lead", p, "subhead", true),
            button(ctx, p, "primary_label", "primary_href", "primary", "")
        ),
        "content/text" => {
            let raw = text(p, "body");
            let paras: String = split_paragraphs(&raw)
                .iter()
                .map(|x| format!("<p>{}</p>", esc_lines(x)))
                .collect();
            let body = if paras.is_empty() {
                String::new()
            } else {
                format!("<div class=\"os-prose__body\">{paras}</div>")
            };
            format!(
                "<div class=\"os-wrap os-prose\">{}{}{body}</div>",
                text_el("p", "os-kicker", p, "eyebrow", false),
                text_el("h2", "os-h2", p, "heading", true)
            )
        }
        "media/3d-embed" => {
            let e = items(s, "embed/3d")
                .first()
                .map(|b| embed3d(ctx, b))
                .unwrap_or_default();
            format!("<div class=\"os-wrap\">{}<div class=\"os-showcase\">{e}</div></div>", head(p))
        }
        "media/video" => {
            let poster = safe_src(&text(p, "poster"), ctx);
            let frame = match safe_video(&text(p, "src")) {
                Some(src) => format!(
                    "<video class=\"os-video__el\" controls preload=\"metadata\" playsinline src=\"{}\"{}></video>",
                    esc(&src),
                    poster.as_ref().map(|u| format!(" poster=\"{}\"", esc(u))).unwrap_or_default()
                ),
                None => format!(
                    "<div class=\"os-video__empty\" role=\"img\" aria-label=\"Video placeholder\">{}<span class=\"os-play\" aria-hidden=\"true\">{}</span></div>",
                    poster
                        .as_ref()
                        .map(|u| format!("<img class=\"os-video__poster\" src=\"{}\" alt=\"\" loading=\"lazy\">", esc(u)))
                        .unwrap_or_default(),
                    svg(PLAY_PATH, "os-ico")
                ),
            };
            let cap = text_el("figcaption", "os-caption", p, "caption", false);
            let h = text_el("h2", "os-h2", p, "heading", true);
            let sub = text_el("p", "os-sub", p, "subheading", true);
            let header = if h.is_empty() && sub.is_empty() {
                String::new()
            } else {
                format!("<header class=\"os-head\">{h}{sub}</header>")
            };
            format!("<div class=\"os-wrap\">{header}<figure class=\"os-video\"><div class=\"os-video__frame\">{frame}</div>{cap}</figure></div>")
        }
        "media/gallery" => {
            let figs: String = items(s, "embed/image")
                .iter()
                .enumerate()
                .map(|(i, b)| image_figure(ctx, b, "os-gallery__item os-item", Some(i)))
                .collect();
            format!("<div class=\"os-wrap\">{}<div class=\"os-gallery\">{figs}</div></div>", head(p))
        }
        "footer/columns" => {
            let cols: String = items(s, "item/column")
                .iter()
                .enumerate()
                .map(|(i, b)| {
                    let title = text(&b.props, "title");
                    let li: String = links(&b.props, "links")
                        .iter()
                        .map(|(l, h)| format!("<li><a href=\"{}\">{}</a></li>", esc(&safe_href(h, ctx)), esc(l)))
                        .collect();
                    format!(
                        "<nav class=\"os-footer__col os-item\" aria-label=\"{}\"{}>{}<ul>{li}</ul></nav>",
                        esc(if title.is_empty() { "Links" } else { title.as_str() }),
                        i_style(i),
                        text_el("h3", "os-footer__title", &b.props, "title", false)
                    )
                })
                .collect();
            format!(
                "<div class=\"os-wrap os-footer__grid\"><div class=\"os-footer__brand\">{}{}</div><div class=\"os-footer__cols\">{cols}</div>{}</div>",
                logo(ctx, p),
                text_el("p", "os-text", p, "tagline", true),
                text_el("p", "os-legal", p, "legal", false)
            )
        }
        "footer/simple" => {
            let nav: String = links(p, "links")
                .iter()
                .map(|(l, h)| format!("<a href=\"{}\">{}</a>", esc(&safe_href(h, ctx)), esc(l)))
                .collect();
            format!(
                "<div class=\"os-wrap os-footer__row\">{}<nav class=\"os-footer__inline\" aria-label=\"Footer\">{nav}</nav>{}</div>",
                logo(ctx, p),
                text_el("p", "os-legal", p, "legal", false)
            )
        }
        _ => String::new(),
    }
}

/// Paragraphs of a body text: split on blank lines (`\n\s*\n`), trimmed.
fn split_paragraphs(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for line in raw.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.trim().is_empty() {
            if !cur.trim().is_empty() {
                out.push(cur.trim().to_string());
            }
            cur.clear();
        } else {
            if !cur.is_empty() {
                cur.push('\n');
            }
            cur.push_str(line);
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

// ---------------------------------------------------------------------------
// Sections, pages, documents
// ---------------------------------------------------------------------------

/// The `<section>` classes (tone, spacing, motion, responsive…) and `--os-bg`.
pub fn section_classes(s: &Section, theme: &Theme) -> (Vec<String>, Option<String>) {
    let (fam, variant) = s.block.split_once('/').unwrap_or((s.block.as_str(), ""));
    let st = &s.style;
    let bg = theme.background(&st.background);
    let or = |v: &str, d: &str| {
        if v.is_empty() {
            d.to_string()
        } else {
            v.to_string()
        }
    };
    let mut c = vec![
        "os-sec".to_string(),
        format!("os-{fam}"),
        format!("os-{fam}--{variant}"),
        format!("os-tone-{}", bg.tone),
        format!("os-space-{}", or(&st.spacing, "m")),
        format!("os-align-{}", or(&st.align, "left")),
    ];
    if !st.motion.is_empty() && st.motion != "none" {
        c.push(format!("os-motion-{}", st.motion));
    }
    if let Some(p) = bg.preset {
        c.push(format!("os-bg-{p}"));
    }
    match st.min_height.as_str() {
        "80vh" => c.push("os-minh-80".into()),
        "100vh" => c.push("os-minh-100".into()),
        _ => {}
    }
    for bp in &s.responsive.hide {
        c.push(format!("os-hide-{bp}"));
    }
    if s.responsive.stack == "media-first" {
        c.push("os-stack-first".into());
    }
    if !s.responsive.mobile_align.is_empty() {
        c.push(format!("os-malign-{}", s.responsive.mobile_align));
    }
    (c, bg.css)
}

/// One section (`""` when hidden or not a known block — never published).
pub fn render_section(s: &Section, ctx: &RenderCtx) -> String {
    if s.hidden || !SECTION_BLOCKS.contains(&s.block.as_str()) {
        return String::new();
    }
    let (classes, bg) = section_classes(s, ctx.theme);
    let style = bg
        .map(|css| format!(" style=\"--os-bg: {}\"", esc(&css)))
        .unwrap_or_default();
    let tag = if s.block.starts_with("nav/") {
        "header"
    } else if s.block.starts_with("footer/") {
        "footer"
    } else {
        "section"
    };
    format!(
        "<{tag} id=\"{}\" class=\"{}\"{style}>{}</{tag}>",
        esc(&s.id),
        esc(&classes.join(" ")),
        body(ctx, s)
    )
}

pub fn render_page_body(page: &Page, ctx: &RenderCtx) -> String {
    page.sections
        .iter()
        .map(|s| render_section(s, ctx))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `index.html` for the home page (the first), else `<slug>.html` (a page
/// without a slug: `<id>.html`).
pub fn page_file_name(doc: &SiteDoc, idx: usize) -> String {
    match doc.pages.get(idx) {
        Some(p) if idx > 0 && !p.slug.is_empty() => format!("{}.html", p.slug),
        Some(p) if idx > 0 => format!("{}.html", p.id),
        _ => "index.html".into(),
    }
}

pub fn page_title(doc: &SiteDoc, idx: usize) -> String {
    let site = doc.title.trim();
    let page = doc.pages.get(idx).map(|p| p.title.trim()).unwrap_or("");
    if idx == 0 || page.is_empty() {
        if !site.is_empty() {
            site.to_string()
        } else if !page.is_empty() {
            page.to_string()
        } else {
            "Site".into()
        }
    } else if site.is_empty() {
        page.to_string()
    } else {
        format!("{page} · {site}")
    }
}

/// How a document gets its stylesheet.
pub enum Css<'a> {
    /// `<link rel="stylesheet" href="…">` (the zip's `site.css`).
    Link(&'a str),
    /// Inlined (the loopback preview).
    Inline(&'a str),
}

/// A standalone HTML document for page `idx`.
pub fn render_document(doc: &SiteDoc, idx: usize, ctx: &RenderCtx, css: Css) -> String {
    let Some(page) = doc.pages.get(idx) else {
        return String::new();
    };
    let lang_raw = if doc.settings.lang.trim().is_empty() {
        "en"
    } else {
        doc.settings.lang.trim()
    };
    let lang: String = lang_raw.chars().take(16).collect();
    let desc = if page.description.trim().is_empty() {
        doc.settings.description.trim()
    } else {
        page.description.trim()
    };
    let desc = if desc.is_empty() {
        String::new()
    } else {
        format!("<meta name=\"description\" content=\"{}\">\n", esc(desc))
    };
    let style = match css {
        Css::Link(href) => format!("<link rel=\"stylesheet\" href=\"{}\">\n", esc(href)),
        Css::Inline(text) => format!("<style>\n{text}</style>\n"),
    };
    format!(
        "<!doctype html>\n<html lang=\"{}\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>{}</title>\n{desc}<meta name=\"generator\" content=\"Otto Site Studio\">\n{style}</head>\n<body>\n<div class=\"os-site\">\n{}\n</div>\n</body>\n</html>\n",
        esc(&lang),
        esc(&page_title(doc, idx)),
        render_page_body(page, ctx)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn theme() -> Theme {
        Theme::from_kit(None)
    }

    fn ctx(theme: &Theme) -> RenderCtx<'_> {
        let mut pages = HashMap::new();
        pages.insert("tiers".to_string(), "tiers.html".to_string());
        RenderCtx {
            theme,
            pages,
            home_href: "index.html".into(),
            assets: HashMap::new(),
            embeds: HashMap::new(),
        }
    }

    fn section(v: serde_json::Value) -> Section {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn every_block_renders_a_scoped_section_without_scripts() {
        let t = theme();
        let c = ctx(&t);
        for b in super::super::schema::SECTION_BLOCKS {
            let s = section(
                json!({ "id": "s1", "block": b, "props": { "heading": "Hi", "headline": "Hi", "logo": "acme" } }),
            );
            let html = render_section(&s, &c);
            assert!(html.contains("class=\"os-sec os-"), "{b}: {html}");
            assert!(!html.to_ascii_lowercase().contains("<script"), "{b}");
            assert!(
                !html.contains("data-os-"),
                "{b}: no editor hooks in the export"
            );
        }
    }

    #[test]
    fn copy_is_escaped_and_links_are_sanitized() {
        let t = theme();
        let c = ctx(&t);
        let s = section(json!({ "id": "hero", "block": "hero/split", "props": {
            "headline": "<script>alert(1)</script> & \"you\"",
            "primary_label": "Go", "primary_href": " JaVa\tScRiPt:alert(1)",
            "secondary_label": "Tiers", "secondary_href": "page:tiers" } }));
        let html = render_section(&s, &c);
        assert!(
            html.contains("&lt;script&gt;alert(1)&lt;/script&gt; &amp; &quot;you&quot;"),
            "{html}"
        );
        assert!(!html.to_ascii_lowercase().contains("javascript:"), "{html}");
        assert!(html.contains("href=\"tiers.html\""), "{html}");
        assert!(
            html.contains("<a class=\"os-btn os-btn--primary\" href=\"#\">"),
            "{html}"
        );
        assert_eq!(safe_href("mailto:a@b.c", &c), "mailto:a@b.c");
        assert_eq!(safe_href("//evil.example", &c), "#");
        assert_eq!(safe_src("http://insecure.example/a.png", &c), None);
        assert_eq!(
            safe_video("https://cdn.example/v.mp4").as_deref(),
            Some("https://cdn.example/v.mp4")
        );
    }

    #[test]
    fn sections_carry_tone_spacing_motion_and_breakpoint_classes() {
        let t = theme();
        let c = ctx(&t);
        let s = section(json!({ "id": "cta", "block": "cta/band",
            "props": { "headline": "Start today." },
            "style": { "background": "token:color.ink", "spacing": "xl", "motion": "fade-up", "min_height": "80vh" },
            "responsive": { "hide": ["mobile"], "stack": "media-first", "mobile_align": "center" } }));
        let html = render_section(&s, &c);
        for want in [
            "os-cta--band",
            "os-tone-dark",
            "os-space-xl",
            "os-motion-fade-up",
            "os-minh-80",
            "os-hide-mobile",
            "os-stack-first",
            "os-malign-center",
            "style=\"--os-bg: var(--os-ink)\"",
        ] {
            assert!(html.contains(want), "{want} in {html}");
        }
        let hidden = section(json!({ "id": "x", "block": "cta/band", "hidden": true }));
        assert_eq!(render_section(&hidden, &c), "");
    }

    #[test]
    fn embeds_render_the_poster_or_a_stand_in() {
        let t = theme();
        let mut c = ctx(&t);
        let s = section(
            json!({ "id": "hero", "block": "hero/split", "props": { "headline": "Hi" },
            "blocks": [{ "id": "card", "block": "embed/3d", "props": { "src": "otto://design/CARD@approved", "alt": "A card" } }] }),
        );
        let stand = render_section(&s, &c);
        assert!(
            stand.contains("os-card3d__name\">Your product in 3D<"),
            "{stand}"
        );
        c.embeds.insert(
            "otto://design/CARD@approved".into(),
            EmbedInfo {
                title: "Rewards Card 3D".into(),
                seq: Some(7),
                policy: "follow_approved".into(),
                poster: Some("assets/CARD-v7.png".into()),
                broken: false,
            },
        );
        let html = render_section(&s, &c);
        assert!(
            html.contains(
                "<img class=\"os-embed-3d__poster\" src=\"assets/CARD-v7.png\" alt=\"A card\""
            ),
            "{html}"
        );
        assert!(html.contains("os-hero__grid--media"));
    }

    #[test]
    fn documents_have_a_title_lang_and_the_stylesheet() {
        let t = theme();
        let c = ctx(&t);
        let doc: SiteDoc = serde_json::from_value(
            json!({ "type": "otto-site", "version": 1, "title": "Rewards+",
            "settings": { "lang": "he", "description": "Earn more" },
            "pages": [{ "id": "home", "title": "Home", "slug": "", "sections": [] },
                      { "id": "tiers", "title": "Tiers", "slug": "tiers", "sections": [] }] }),
        )
        .unwrap();
        let home = render_document(&doc, 0, &c, Css::Link("site.css"));
        assert!(home.starts_with("<!doctype html>\n<html lang=\"he\">"));
        assert!(home.contains("<title>Rewards+</title>"));
        assert!(home.contains("<meta name=\"description\" content=\"Earn more\">"));
        assert!(home.contains("<link rel=\"stylesheet\" href=\"site.css\">"));
        assert_eq!(page_file_name(&doc, 1), "tiers.html");
        assert_eq!(page_title(&doc, 1), "Tiers · Rewards+");
        assert!(render_document(&doc, 1, &c, Css::Inline(".os-site{}"))
            .contains("<style>\n.os-site{}</style>"));
        assert_eq!(
            split_paragraphs("a\nb\n\n  \nc"),
            vec!["a\nb".to_string(), "c".to_string()]
        );
        assert_eq!(initials("Priya Natarajan"), "PN");
    }
}
