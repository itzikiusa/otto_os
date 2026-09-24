//! `otto://design/<artifact_id>[@approved|@latest|@v<seq>][#<node_id>]` — the
//! ONLY way a design document references another artifact (never a raw URL or
//! path, the rule `scene3d` already applies to `gltf`). Pure; unit-tested.

/// The literal every reference starts with.
pub const PREFIX: &str = "otto://design/";

const MAX_ID: usize = 64;
const MAX_NODE: usize = 128;

/// Which version of the target a reference asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionSel {
    /// No selector — the link's policy decides (render rels follow approved).
    Default,
    Approved,
    Latest,
    /// `@v<seq>` — a pinned version, by its 1-based per-artifact sequence.
    Seq(i64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesignUri {
    pub artifact_id: String,
    pub version: VersionSel,
    pub node: Option<String>,
}

fn id_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

fn node_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ':' | '.')
}

impl DesignUri {
    /// Parse a whole string (surrounding whitespace ignored). `None` when it is
    /// not exactly one well-formed reference.
    pub fn parse(s: &str) -> Option<DesignUri> {
        let s = s.trim();
        let rest = s.strip_prefix(PREFIX)?;
        let (uri, used) = parse_tail(rest).ok()?;
        (used == rest.len()).then_some(uri)
    }

    /// The canonical string form.
    pub fn to_uri_string(&self) -> String {
        let mut out = format!("{PREFIX}{}", self.artifact_id);
        match &self.version {
            VersionSel::Default => {}
            VersionSel::Approved => out.push_str("@approved"),
            VersionSel::Latest => out.push_str("@latest"),
            VersionSel::Seq(n) => out.push_str(&format!("@v{n}")),
        }
        if let Some(n) = &self.node {
            out.push('#');
            out.push_str(n);
        }
        out
    }

    /// The link policy this reference implies for `rel`: an explicit selector
    /// wins; otherwise render rels follow the approved version (drafts never
    /// leak into consumers), provenance rels are pinned history, and the rest
    /// follow the latest.
    pub fn policy_for(&self, rel: &str) -> &'static str {
        match self.version {
            VersionSel::Approved => "follow_approved",
            VersionSel::Latest => "follow_latest",
            VersionSel::Seq(_) => "pinned",
            VersionSel::Default => default_policy(rel),
        }
    }
}

/// Default link policy for a relation when nothing more specific is given.
pub fn default_policy(rel: &str) -> &'static str {
    if crate::types::RENDER_RELS.contains(&rel) {
        "follow_approved"
    } else if matches!(
        rel,
        "derived_from" | "references" | "variant_of" | "resized_from"
    ) {
        "pinned"
    } else {
        "follow_latest"
    }
}

/// Parse what follows [`PREFIX`]. Returns the reference and how many bytes of
/// `rest` it consumed; `Err(consumed)` for a malformed reference.
fn parse_tail(rest: &str) -> Result<(DesignUri, usize), usize> {
    let id_len: usize = rest
        .chars()
        .take_while(|c| id_char(*c))
        .map(char::len_utf8)
        .sum();
    if id_len == 0 || id_len > MAX_ID {
        return Err(id_len);
    }
    let artifact_id = rest[..id_len].to_string();
    let mut pos = id_len;
    let mut version = VersionSel::Default;
    if rest[pos..].starts_with('@') {
        let sel_len: usize = rest[pos + 1..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .map(char::len_utf8)
            .sum();
        let sel = &rest[pos + 1..pos + 1 + sel_len];
        version = match sel {
            "approved" => VersionSel::Approved,
            "latest" => VersionSel::Latest,
            _ => match sel.strip_prefix('v').and_then(|n| n.parse::<i64>().ok()) {
                Some(n) if n >= 1 => VersionSel::Seq(n),
                _ => return Err(pos + 1 + sel_len),
            },
        };
        pos += 1 + sel_len;
    }
    let mut node = None;
    if rest[pos..].starts_with('#') {
        let raw_len: usize = rest[pos + 1..]
            .chars()
            .take_while(|c| node_char(*c))
            .map(char::len_utf8)
            .sum();
        // A sentence may end right after a reference ("see …#hero.") — trailing
        // `.`/`:` are punctuation, not part of the node id.
        let raw = &rest[pos + 1..pos + 1 + raw_len];
        let trimmed = raw.trim_end_matches(['.', ':']);
        if trimmed.is_empty() || trimmed.len() > MAX_NODE {
            return Err(pos + 1 + raw_len);
        }
        node = Some(trimmed.to_string());
        pos += 1 + trimmed.len();
    }
    Ok((
        DesignUri {
            artifact_id,
            version,
            node,
        },
        pos,
    ))
}

/// One reference found by [`scan`]: its byte offset in the scanned text and
/// either the parsed reference or the malformed raw text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanHit {
    pub offset: usize,
    pub parsed: Result<DesignUri, String>,
}

/// Find every `otto://design/…` reference in free text (HTML attributes,
/// Mermaid `click` lines, D2 `link`s, JSON string values …). Bounded to the
/// first 2 000 hits so a hostile document can't balloon the link table.
pub fn scan(text: &str) -> Vec<ScanHit> {
    const MAX_HITS: usize = 2_000;
    let mut out = Vec::new();
    let mut from = 0usize;
    while let Some(i) = text[from..].find(PREFIX) {
        let offset = from + i;
        let tail_start = offset + PREFIX.len();
        let rest = &text[tail_start..];
        match parse_tail(rest) {
            Ok((uri, used)) => {
                out.push(ScanHit {
                    offset,
                    parsed: Ok(uri),
                });
                from = tail_start + used;
            }
            Err(used) => {
                let raw_end = tail_start + used.min(rest.len());
                out.push(ScanHit {
                    offset,
                    parsed: Err(text[offset..raw_end].to_string()),
                });
                from = raw_end.max(tail_start);
            }
        }
        if out.len() >= MAX_HITS {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_selector_and_node() {
        let u = DesignUri::parse("otto://design/a_7f3c@approved#view:hero").unwrap();
        assert_eq!(u.artifact_id, "a_7f3c");
        assert_eq!(u.version, VersionSel::Approved);
        assert_eq!(u.node.as_deref(), Some("view:hero"));
        assert_eq!(u.to_uri_string(), "otto://design/a_7f3c@approved#view:hero");

        let u = DesignUri::parse("otto://design/01HXYZ@v12").unwrap();
        assert_eq!(u.version, VersionSel::Seq(12));
        assert_eq!(u.node, None);
        assert_eq!(u.policy_for("embeds"), "pinned");

        let u = DesignUri::parse("otto://design/01HXYZ@latest").unwrap();
        assert_eq!(u.policy_for("embeds"), "follow_latest");

        let u = DesignUri::parse("  otto://design/01HXYZ  ").unwrap();
        assert_eq!(u.version, VersionSel::Default);
        assert_eq!(u.policy_for("embeds"), "follow_approved");
        assert_eq!(u.policy_for("derived_from"), "pinned");
        assert_eq!(u.policy_for("describes"), "follow_latest");
    }

    #[test]
    fn rejects_malformed_references() {
        for bad in [
            "otto://design/",
            "otto://design/@approved",
            "otto://design/a@v0",
            "otto://design/a@vx",
            "otto://design/a@draft",
            "otto://design/a#",
            "https://example.com/a",
            "otto://design/a b",
        ] {
            assert!(DesignUri::parse(bad).is_none(), "{bad} should not parse");
        }
        let long = format!("otto://design/{}", "a".repeat(65));
        assert!(DesignUri::parse(&long).is_none());
    }

    #[test]
    fn scan_finds_references_in_free_text() {
        let html = r#"<img src="otto://design/A1@approved#hero"> and <a href='otto://design/B2'>b</a>.
            see otto://design/C3#faq. broken: otto://design/@v1 end"#;
        let hits = scan(html);
        assert_eq!(hits.len(), 4);
        assert_eq!(hits[0].parsed.as_ref().unwrap().artifact_id, "A1");
        assert_eq!(
            hits[0].parsed.as_ref().unwrap().node.as_deref(),
            Some("hero")
        );
        assert_eq!(hits[1].parsed.as_ref().unwrap().artifact_id, "B2");
        // Trailing sentence punctuation is not part of the node.
        assert_eq!(
            hits[2].parsed.as_ref().unwrap().node.as_deref(),
            Some("faq")
        );
        assert!(hits[3].parsed.is_err());
        // Offsets point at the reference start.
        assert!(html[hits[0].offset..].starts_with(PREFIX));
    }

    #[test]
    fn scan_is_safe_on_multibyte_text_and_bounded() {
        let text = "ünïcode → otto://design/Ä";
        let hits = scan(text);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].parsed.is_err());
        let many = "otto://design/x ".repeat(3_000);
        assert_eq!(scan(&many).len(), 2_000);
    }
}
