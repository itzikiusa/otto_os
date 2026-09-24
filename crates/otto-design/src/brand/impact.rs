//! The impact preview (`POST /design/artifacts/{id}/brand/impact`): which
//! artifacts read this kit (`uses_tokens` links into it), which tokens each of
//! them references, and which of those a proposed kit document changes —
//! BEFORE the change is saved (proposal §3.3: "affects 14 artifacts in 4
//! studios").
//!
//! A consumer references a token by name, in any text format:
//!   - `token:<group>.<name>` (the Design Hall reference form; a type style's
//!     sub-property `token:type.display.size` counts as `type.display`);
//!   - the CSS custom property `--brand-<group>-<name>` (`var(--brand-color-primary)`).
//!
//! A consumer that links to the kit but names no single token gets the WHOLE
//! kit applied (a site theme), so every change reaches it.
//!
//! The IO (loading the kit's versions and each consumer's head) lives in the
//! route (`brand::http`); everything here is pure and unit-tested.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use otto_core::Id;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::contrast::ContrastReport;
use super::doc::{css_var, BrandModel, GROUPS};
use crate::types::DesignArtifact;

/// Token references collected per consumer document (bounded).
const MAX_REFS_PER_DOC: usize = 2_000;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct BrandImpactReq {
    /// The proposed kit: a JSON object or its text. Omitted = the current head
    /// (a plain "who uses this kit" listing, no changes).
    #[serde(default)]
    pub content: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenChange {
    /// `group.name`, e.g. `color.primary`.
    pub token: String,
    /// `changed` | `added` | `removed`.
    pub change: String,
    /// Canonical value before / after (`#5B3DF5`, `14px`, `64/72/800` …).
    pub before: Option<String>,
    pub after: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StudioCount {
    pub studio: String,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrandConsumer {
    pub artifact: DesignArtifact,
    /// The consumer's link policy: `follow_approved` (moves when a kit
    /// version is approved) | `follow_latest` (moves on every save) | `pinned`.
    pub policy: String,
    pub pinned_version_id: Option<Id>,
    /// Tokens the consumer's head references by name (sorted).
    pub tokens: Vec<String>,
    /// References no single token → the whole kit applies to it.
    pub whole_kit: bool,
    /// The changed tokens this consumer would see (sorted).
    pub affected: Vec<String>,
    /// `false` when the head couldn't be read as text (binary, too large).
    pub scanned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrandImpactResp {
    pub kit_id: Id,
    /// What the proposal is compared with: `approved` (what consumers
    /// follow) | `head` (no approved version yet) | `none` (empty kit).
    pub base: String,
    pub base_version_id: Option<Id>,
    pub base_seq: Option<i64>,
    pub changes: Vec<TokenChange>,
    /// Every visible, non-archived consumer.
    pub artifact_count: i64,
    pub studio_count: i64,
    pub by_studio: Vec<StudioCount>,
    /// Consumers that would see at least one change.
    pub affected_count: i64,
    pub affected_studio_count: i64,
    pub affected_by_studio: Vec<StudioCount>,
    pub consumers: Vec<BrandConsumer>,
    /// Consumers in workspaces the caller can't view (counted, never listed).
    pub hidden_count: i64,
    /// WCAG contrast of the PROPOSED kit's colours.
    pub contrast: ContrastReport,
    pub warnings: Vec<String>,
}

/// Token-level diff of two kits (sorted by token).
pub fn diff(old: &BrandModel, new: &BrandModel) -> Vec<TokenChange> {
    let (a, b) = (old.flat(), new.flat());
    let mut out = Vec::new();
    let keys: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
    for k in keys {
        let (before, after) = (a.get(k), b.get(k));
        let change = match (before, after) {
            (Some(x), Some(y)) if x == y => continue,
            (Some(_), Some(_)) => "changed",
            (None, Some(_)) => "added",
            (Some(_), None) => "removed",
            (None, None) => continue,
        };
        out.push(TokenChange {
            token: k.clone(),
            change: change.into(),
            before: before.cloned(),
            after: after.cloned(),
        });
    }
    out
}

/// CSS custom property → token key for every token of the given kits
/// (`--brand-color-primary` → `color.primary`; a type style's three
/// sub-properties → `type.<name>`).
pub fn css_index(models: &[&BrandModel]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for m in models {
        for c in &m.colors {
            out.insert(css_var("color", &c.name), format!("color.{}", c.name));
        }
        for f in &m.fonts {
            out.insert(css_var("font", &f.name), format!("font.{}", f.name));
        }
        for t in &m.types {
            let base = css_var("type", &t.name);
            for suffix in ["size", "line", "weight"] {
                out.insert(format!("{base}-{suffix}"), format!("type.{}", t.name));
            }
        }
        for r in &m.radius {
            out.insert(css_var("radius", &r.name), format!("radius.{}", r.name));
        }
        for s in &m.space {
            out.insert(css_var("space", &s.name), format!("space.{}", s.name));
        }
    }
    out
}

fn ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// The tokens a consumer document names: `token:<group>.<name>[.<sub>]`
/// (group must be a kit group) and any `--brand-…` property the kit defines.
pub fn token_refs(text: &str, css: &HashMap<String, String>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut hits = 0usize;
    // `token:group.name`
    let mut from = 0usize;
    while let Some(i) = text[from..].find("token:") {
        let at = from + i;
        from = at + "token:".len();
        if text[..at].chars().next_back().is_some_and(ident_char) {
            continue; // `mytoken:` — not ours
        }
        let tail: String = text[from..]
            .chars()
            .take_while(|c| ident_char(*c) || *c == '.')
            .collect();
        let mut parts = tail.split('.').filter(|p| !p.is_empty());
        if let (Some(g), Some(n)) = (parts.next(), parts.next()) {
            if GROUPS.contains(&g) {
                out.insert(format!("{g}.{n}"));
            }
        }
        hits += 1;
        if hits >= MAX_REFS_PER_DOC {
            return out;
        }
    }
    // `--brand-group-name`
    let mut from = 0usize;
    while let Some(i) = text[from..].find("--brand-") {
        let at = from + i;
        from = at + "--brand-".len();
        if text[..at].chars().next_back().is_some_and(ident_char) {
            continue;
        }
        let ident: String = text[at..].chars().take_while(|c| ident_char(*c)).collect();
        if let Some(key) = css.get(&ident) {
            out.insert(key.clone());
        }
        hits += 1;
        if hits >= MAX_REFS_PER_DOC {
            break;
        }
    }
    out
}

/// One consumer's row. `text` = its head as text (`None` = not scannable,
/// treated as whole-kit so a change is never under-reported).
pub fn consumer(
    artifact: DesignArtifact,
    policy: String,
    pinned_version_id: Option<Id>,
    text: Option<&str>,
    css: &HashMap<String, String>,
    changed: &BTreeSet<String>,
) -> BrandConsumer {
    let refs = text.map(|t| token_refs(t, css)).unwrap_or_default();
    let whole_kit = refs.is_empty();
    let affected: Vec<String> = if whole_kit {
        changed.iter().cloned().collect()
    } else {
        refs.intersection(changed).cloned().collect()
    };
    BrandConsumer {
        artifact,
        policy,
        pinned_version_id,
        tokens: refs.into_iter().collect(),
        whole_kit,
        affected,
        scanned: text.is_some(),
    }
}

/// Count by studio: most first, then by name.
pub fn studio_counts<'a>(studios: impl Iterator<Item = &'a str>) -> Vec<StudioCount> {
    let mut m: BTreeMap<&str, i64> = BTreeMap::new();
    for s in studios {
        *m.entry(s).or_default() += 1;
    }
    let mut out: Vec<StudioCount> = m
        .into_iter()
        .map(|(studio, count)| StudioCount {
            studio: studio.to_string(),
            count,
        })
        .collect();
    out.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.studio.cmp(&b.studio)));
    out
}

/// Assemble the response from the rows the route gathered.
#[allow(clippy::too_many_arguments)]
pub fn summarize(
    kit_id: Id,
    base: &str,
    base_version: Option<(Id, i64)>,
    changes: Vec<TokenChange>,
    mut consumers: Vec<BrandConsumer>,
    hidden_count: i64,
    contrast: ContrastReport,
    warnings: Vec<String>,
) -> BrandImpactResp {
    // Affected first, then most recently updated.
    consumers.sort_by(|a, b| {
        a.affected
            .is_empty()
            .cmp(&b.affected.is_empty())
            .then_with(|| b.artifact.updated_at.cmp(&a.artifact.updated_at))
    });
    let by_studio = studio_counts(consumers.iter().map(|c| c.artifact.studio.as_str()));
    let affected_by_studio = studio_counts(
        consumers
            .iter()
            .filter(|c| !c.affected.is_empty())
            .map(|c| c.artifact.studio.as_str()),
    );
    let (base_version_id, base_seq) = match base_version {
        Some((id, seq)) => (Some(id), Some(seq)),
        None => (None, None),
    };
    BrandImpactResp {
        kit_id,
        base: base.to_string(),
        base_version_id,
        base_seq,
        changes,
        artifact_count: consumers.len() as i64,
        studio_count: by_studio.len() as i64,
        affected_count: consumers.iter().filter(|c| !c.affected.is_empty()).count() as i64,
        affected_studio_count: affected_by_studio.len() as i64,
        by_studio,
        affected_by_studio,
        consumers,
        hidden_count,
        contrast,
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use serde_json::json;

    fn kit(primary: &str, card: f64) -> BrandModel {
        BrandModel::from_value(&json!({
            "color": { "primary": { "$value": primary }, "ink": { "$value": "#14122B" } },
            "type": { "display": { "size": 64, "line": 72, "weight": 800 } },
            "radius": { "card": { "$value": card } }
        }))
    }

    /// Built from JSON so read-only joined fields (`#[serde(default)]`) that
    /// the store adds over time never break the fixture.
    fn art(id: &str, studio: &str, minute: u32) -> DesignArtifact {
        let t = Utc.with_ymd_and_hms(2026, 9, 23, 12, minute, 0).unwrap();
        serde_json::from_value(json!({
            "id": id,
            "project_id": null,
            "workspace_id": "w",
            "studio": studio,
            "format": "otto-site",
            "mime": "application/vnd.otto.site+json",
            "title": id,
            "status": "draft",
            "head_version_id": null,
            "head_seq": null,
            "approved_version_id": null,
            "tags": [],
            "thumb_blob": null,
            "meta": null,
            "source_kind": null,
            "source_id": null,
            "created_by": "u",
            "created_by_kind": "user",
            "created_session_id": null,
            "created_at": t,
            "updated_at": t
        }))
        .unwrap()
    }

    #[test]
    fn diff_reports_changed_added_removed() {
        let old = kit("#5B3DF5", 14.0);
        let mut new = kit("#0F9D8A", 14.0);
        new.space.push(super::super::doc::PxTok {
            name: "md".into(),
            px: 16.0,
            description: None,
        });
        new.types.clear();
        let d = diff(&old, &new);
        assert_eq!(
            d,
            vec![
                TokenChange {
                    token: "color.primary".into(),
                    change: "changed".into(),
                    before: Some("#5B3DF5".into()),
                    after: Some("#0F9D8A".into()),
                },
                TokenChange {
                    token: "space.md".into(),
                    change: "added".into(),
                    before: None,
                    after: Some("16px".into()),
                },
                TokenChange {
                    token: "type.display".into(),
                    change: "removed".into(),
                    before: Some("64/72/800".into()),
                    after: None,
                },
            ]
        );
        assert!(diff(&old, &old).is_empty());
    }

    #[test]
    fn token_refs_cover_both_reference_forms() {
        let k = kit("#5B3DF5", 14.0);
        let css = css_index(&[&k]);
        let site = r#"{"brand":"otto://design/KIT","hero":{"bg":"token:color.primary","radius":"token:radius.card"},
            "h1":{"size":"token:type.display.size"},"x":"mytoken:color.ink","y":"token:nope.thing","z":"token:color"}"#;
        let refs = token_refs(site, &css);
        assert_eq!(
            refs.into_iter().collect::<Vec<_>>(),
            vec!["color.primary", "radius.card", "type.display"]
        );
        let html = "<style>.cta{background:var(--brand-color-primary);border-radius:var(--brand-radius-card)}\
                    h1{font-size:var(--brand-type-display-size)} .x{color:var(--brand-color-unknown)} a{--my--brand-color-ink:1}</style>";
        let refs = token_refs(html, &css);
        assert_eq!(
            refs.into_iter().collect::<Vec<_>>(),
            vec!["color.primary", "radius.card", "type.display"]
        );
    }

    #[test]
    fn consumers_see_only_what_they_reference_unless_whole_kit() {
        let old = kit("#5B3DF5", 14.0);
        let new = kit("#0F9D8A", 14.0);
        let changes = diff(&old, &new);
        let changed: BTreeSet<String> = changes.iter().map(|c| c.token.clone()).collect();
        let css = css_index(&[&old, &new]);

        let hero = consumer(
            art("hero", "site", 1),
            "follow_approved".into(),
            None,
            Some("token:color.primary token:radius.card"),
            &css,
            &changed,
        );
        assert_eq!(hero.tokens, vec!["color.primary", "radius.card"]);
        assert_eq!(hero.affected, vec!["color.primary"]);
        assert!(!hero.whole_kit && hero.scanned);

        let card = consumer(
            art("card", "3d", 2),
            "pinned".into(),
            Some("v4".into()),
            Some("var(--brand-radius-card)"),
            &css,
            &changed,
        );
        assert!(card.affected.is_empty(), "radius didn't change");

        let theme = consumer(
            art("theme", "site", 3),
            "follow_latest".into(),
            None,
            Some("{}"),
            &css,
            &changed,
        );
        assert!(theme.whole_kit);
        assert_eq!(theme.affected, vec!["color.primary"]);

        let png = consumer(
            art("png", "graphics", 4),
            "follow_approved".into(),
            None,
            None,
            &css,
            &changed,
        );
        assert!(!png.scanned && png.whole_kit && !png.affected.is_empty());

        let resp = summarize(
            "kit".into(),
            "approved",
            Some(("v4id".into(), 4)),
            changes,
            vec![card, hero, theme, png],
            2,
            super::super::contrast::report(&new),
            vec![],
        );
        assert_eq!(resp.artifact_count, 4);
        assert_eq!(resp.studio_count, 3);
        assert_eq!(resp.affected_count, 3);
        assert_eq!(resp.affected_studio_count, 2);
        assert_eq!(
            resp.by_studio[0],
            StudioCount {
                studio: "site".into(),
                count: 2
            }
        );
        assert_eq!(resp.hidden_count, 2);
        assert_eq!(resp.base_seq, Some(4));
        // Affected consumers first (newest first among them), unaffected last.
        let order: Vec<&str> = resp
            .consumers
            .iter()
            .map(|c| c.artifact.id.as_str())
            .collect();
        assert_eq!(order, vec!["png", "theme", "hero", "card"]);
        let v = serde_json::to_value(&resp).unwrap();
        assert_eq!(v["changes"][0]["token"], "color.primary");
        assert!(v["contrast"]["colors"].is_array());
    }

    #[test]
    fn studio_counts_sort_by_count_then_name() {
        let c = studio_counts(["site", "3d", "site", "frames", "3d", "site"].into_iter());
        let flat: Vec<(String, i64)> = c.into_iter().map(|s| (s.studio, s.count)).collect();
        assert_eq!(
            flat,
            vec![("site".into(), 3), ("3d".into(), 2), ("frames".into(), 1)]
        );
    }
}
