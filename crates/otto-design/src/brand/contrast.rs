//! WCAG 2.x contrast for a kit's colours: every colour against white and
//! against the kit's ink, and every pair of colours against each other. The
//! UI computes the same numbers live (`contrastRatio` in
//! `ui/src/modules/design-hall/brand/tokens.ts`); this is the server's copy the
//! impact preview reports. Pure; unit-tested.

use serde::{Deserialize, Serialize};

use super::doc::BrandModel;

pub const WHITE: &str = "#FFFFFF";
const BLACK: &str = "#000000";
/// Pairs reported at most (64 colours → 2 016 pairs; well inside this).
const MAX_PAIRS: usize = 2_100;

/// `#RRGGBB[AA]` → linear-light relative luminance (alpha ignored).
pub fn luminance(hex: &str) -> Option<f64> {
    let h = super::doc::normalize_hex(hex)?;
    let h = &h[1..];
    let mut lin = [0.0f64; 3];
    for (i, slot) in lin.iter_mut().enumerate() {
        let c = u8::from_str_radix(&h[i * 2..i * 2 + 2], 16).ok()? as f64 / 255.0;
        *slot = if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        };
    }
    Some(0.2126 * lin[0] + 0.7152 * lin[1] + 0.0722 * lin[2])
}

/// The contrast ratio of two colours (1 – 21), rounded to 2 decimals.
pub fn ratio(a: &str, b: &str) -> Option<f64> {
    let (la, lb) = (luminance(a)?, luminance(b)?);
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    Some(((hi + 0.05) / (lo + 0.05) * 100.0).round() / 100.0)
}

/// `AAA` (≥ 7), `AA` (≥ 4.5, body text), `AA-large` (≥ 3, 24 px+ text and UI
/// graphics) or `fail`.
pub fn level(r: f64) -> &'static str {
    if r >= 7.0 {
        "AAA"
    } else if r >= 4.5 {
        "AA"
    } else if r >= 3.0 {
        "AA-large"
    } else {
        "fail"
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InkRef {
    /// The colour token used as ink (`ink`, else `text`, else the darkest
    /// colour); `None` = no colours, plain black.
    pub name: Option<String>,
    pub hex: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColorContrast {
    pub name: String,
    pub hex: String,
    pub on_white: f64,
    pub on_ink: f64,
    pub white_level: String,
    pub ink_level: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairContrast {
    pub a: String,
    pub b: String,
    pub ratio: f64,
    pub level: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContrastReport {
    pub ink: InkRef,
    pub colors: Vec<ColorContrast>,
    /// Every unordered pair of colour tokens (`a` before `b` in kit order).
    pub pairs: Vec<PairContrast>,
}

/// The kit's ink: the `ink` token, else `text`, else the darkest colour.
pub fn ink_of(m: &BrandModel) -> InkRef {
    for want in ["ink", "text"] {
        if let Some(c) = m.colors.iter().find(|c| c.name == want) {
            return InkRef {
                name: Some(c.name.clone()),
                hex: c.hex.clone(),
            };
        }
    }
    m.colors
        .iter()
        .filter_map(|c| luminance(&c.hex).map(|l| (l, c)))
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, c)| InkRef {
            name: Some(c.name.clone()),
            hex: c.hex.clone(),
        })
        .unwrap_or(InkRef {
            name: None,
            hex: BLACK.into(),
        })
}

pub fn report(m: &BrandModel) -> ContrastReport {
    let ink = ink_of(m);
    let colors = m
        .colors
        .iter()
        .map(|c| {
            let on_white = ratio(&c.hex, WHITE).unwrap_or(1.0);
            let on_ink = ratio(&c.hex, &ink.hex).unwrap_or(1.0);
            ColorContrast {
                name: c.name.clone(),
                hex: c.hex.clone(),
                on_white,
                on_ink,
                white_level: level(on_white).into(),
                ink_level: level(on_ink).into(),
            }
        })
        .collect();
    let mut pairs = Vec::new();
    'outer: for (i, a) in m.colors.iter().enumerate() {
        for b in &m.colors[i + 1..] {
            if pairs.len() >= MAX_PAIRS {
                break 'outer;
            }
            let r = ratio(&a.hex, &b.hex).unwrap_or(1.0);
            pairs.push(PairContrast {
                a: a.name.clone(),
                b: b.name.clone(),
                ratio: r,
                level: level(r).into(),
            });
        }
    }
    ContrastReport { ink, colors, pairs }
}

/// Human warnings for the impact preview: a colour that reads on neither
/// white nor ink (below 3:1 on both) can't carry text or icons anywhere.
pub fn warnings(r: &ContrastReport) -> Vec<String> {
    r.colors
        .iter()
        .filter(|c| c.on_white < 3.0 && c.on_ink < 3.0 && Some(&c.name) != r.ink.name.as_ref())
        .map(|c| {
            format!(
                "color.{} ({}) is below 3:1 on both white ({:.1}) and ink ({:.1}) — don't use it for text or icons",
                c.name, c.hex, c.on_white, c.on_ink
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn known_ratios() {
        assert_eq!(ratio("#000000", "#FFFFFF"), Some(21.0));
        assert_eq!(ratio("#fff", "#FFFFFF"), Some(1.0));
        // The mockup's Acme primary: 6.1-ish on white.
        let r = ratio("#5B3DF5", WHITE).unwrap();
        assert!((6.0..6.3).contains(&r), "{r}");
        // Amber is not text on white.
        let amber = ratio("#FFB547", WHITE).unwrap();
        assert!(amber < 3.0, "{amber}");
        assert_eq!(ratio("red", WHITE), None);
        assert_eq!(ratio("#777777", "#FFFFFF"), ratio("#FFFFFF", "#777777"));
    }

    #[test]
    fn levels() {
        assert_eq!(level(21.0), "AAA");
        assert_eq!(level(4.5), "AA");
        assert_eq!(level(3.2), "AA-large");
        assert_eq!(level(1.8), "fail");
    }

    #[test]
    fn report_covers_white_ink_and_every_pair() {
        let m = BrandModel::from_value(&json!({
            "color": {
                "primary": { "$value": "#5B3DF5" },
                "accent": { "$value": "#FFB547" },
                "ink": { "$value": "#14122B" },
                "surface": { "$value": "#FFFFFF" }
            }
        }));
        let r = report(&m);
        assert_eq!(r.ink.name.as_deref(), Some("ink"));
        assert_eq!(r.colors.len(), 4);
        assert_eq!(r.pairs.len(), 6, "4 colours → 6 unordered pairs");
        let accent = r.colors.iter().find(|c| c.name == "accent").unwrap();
        assert_eq!(accent.white_level, "fail");
        assert!(accent.on_ink > 7.0, "{}", accent.on_ink);
        let ink = r.colors.iter().find(|c| c.name == "ink").unwrap();
        assert_eq!(ink.on_ink, 1.0);
        assert!(
            warnings(&r).is_empty(),
            "every colour reads on white or ink"
        );
    }

    #[test]
    fn ink_falls_back_to_the_darkest_colour_then_black() {
        let m = BrandModel::from_value(&json!({
            "color": { "a": { "$value": "#777777" }, "b": { "$value": "#222222" } }
        }));
        assert_eq!(ink_of(&m).name.as_deref(), Some("b"));
        let none = ink_of(&BrandModel::default());
        assert_eq!(
            none,
            InkRef {
                name: None,
                hex: "#000000".into()
            }
        );
    }

    #[test]
    fn a_mid_grey_on_a_mid_grey_ink_warns() {
        let m = BrandModel::from_value(&json!({
            "color": { "ink": { "$value": "#595959" }, "mid": { "$value": "#9A9A9A" } }
        }));
        let w = warnings(&report(&m));
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(w[0].contains("color.mid"), "{w:?}");
    }
}
