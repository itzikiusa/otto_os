//! Session name **themes**: fame-ordered pools of memorable handles used to
//! auto-name new agent sessions (e.g. "Ronaldo", "Messi"), plus the allocator
//! that keeps names unique among open sessions and the address resolver that
//! routes `"ronaldo: do X"` to the session named Ronaldo.
//!
//! A *theme* is either:
//!  - **builtin** — a curated, fame-ordered `head` (the most recognizable
//!    figures first) plus first/last name pools used to synthesize a long
//!    unique tail, so the theme never runs out (capacity ≥ 10k candidates); or
//!  - **custom** — a user-supplied ordered list of plain names (e.g. family
//!    names). When a custom theme is exhausted among the *open* sessions it
//!    falls back to `"{name} #2"`, `"{name} #3"`, … just like the old numbered
//!    scheme — and a name frees up the moment its session closes.
//!
//! Allocation is always **unique among the supplied `used` set** (the titles of
//! the workspace's open agent sessions), so addressing a session by name is
//! unambiguous. The data files live in `names/data/<id>.txt`; see
//! [`parse_theme_file`] for the format.

use std::collections::HashSet;
use std::sync::OnceLock;

/// The theme applied to new sessions when a user hasn't chosen one. Matches the
/// "ronaldo" example so the feature works out of the box.
pub const DEFAULT_THEME: &str = "footballers";

/// Sentinel theme id meaning "no theme — use the legacy `{provider} #N`".
pub const THEME_NONE: &str = "none";

/// A successfully allocated session name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Allocated {
    /// The display title stored on the session (e.g. "Ronaldo" or, for a
    /// synthesized tail name, "Diego Ferreira"). Unique among open sessions.
    pub title: String,
    /// The short, single-token handle you address the session by (e.g.
    /// "Ronaldo", "Ferreira"). Used by the resolver.
    pub handle: String,
    /// The full display name (e.g. "Cristiano Ronaldo"); equals `title` for
    /// synthesized tail names and custom names.
    pub full: String,
}

/// One nameable figure parsed from a theme's `## HEAD` section.
#[derive(Debug, Clone)]
struct HeadEntry {
    handle: String,
    full: String,
}

/// A built-in, fame-ordered name theme.
struct BuiltinTheme {
    id: &'static str,
    label: &'static str,
    /// Curated, most-famous-first.
    head: Vec<HeadEntry>,
    /// First-name pool for the synthesized tail.
    firsts: Vec<String>,
    /// Surname pool for the synthesized tail (each is itself a callable handle).
    lasts: Vec<String>,
}

impl BuiltinTheme {
    /// Total distinct candidates this theme can yield (head + first×last).
    fn capacity(&self) -> usize {
        self.head.len() + self.firsts.len() * self.lasts.len()
    }

    /// The first few head display titles, for the settings preview.
    fn sample(&self) -> Vec<String> {
        self.head.iter().take(6).map(|e| e.handle.clone()).collect()
    }

    /// Allocate the first candidate whose lowercased title isn't in `used`.
    ///
    /// Tries the fame-ordered head first (so a fresh workspace gets the icons),
    /// then synthesizes "First Last" tail names (title = full name, handle =
    /// surname) which are unique via first×last combinatorics. As a final,
    /// astronomically-unlikely backstop, falls back to a numeric suffix.
    fn allocate(&self, used: &HashSet<String>) -> Allocated {
        for e in &self.head {
            if !used.contains(&e.handle.to_lowercase()) {
                return Allocated {
                    title: e.handle.clone(),
                    handle: e.handle.clone(),
                    full: e.full.clone(),
                };
            }
        }
        let nf = self.firsts.len();
        let nl = self.lasts.len();
        if nf > 0 && nl > 0 {
            // Vary the surname fastest so distinct handles appear first; the
            // title is the full "First Last" so it stays unique even when the
            // surname repeats across different first names.
            for i in 0..(nf * nl) {
                let last = &self.lasts[i % nl];
                let first = &self.firsts[(i / nl) % nf];
                let title = format!("{first} {last}");
                if !used.contains(&title.to_lowercase()) {
                    return Allocated {
                        title,
                        handle: last.clone(),
                        full: format!("{first} {last}"),
                    };
                }
            }
        }
        numeric_fallback(self.head.first().map(|e| e.handle.as_str()).unwrap_or("Agent"), used)
    }
}

/// Allocate `"{base} #N"` for the smallest N ≥ 2 not in `used`. Shared backstop
/// for an exhausted builtin theme and the custom-theme overflow.
fn numeric_fallback(base: &str, used: &HashSet<String>) -> Allocated {
    let mut n = 2;
    loop {
        let title = format!("{base} #{n}");
        if !used.contains(&title.to_lowercase()) {
            return Allocated {
                handle: base.to_string(),
                full: title.clone(),
                title,
            };
        }
        n += 1;
    }
}

/// Parse a theme data file. Format (UTF-8):
/// ```text
/// ## HEAD
/// Ronaldo | Cristiano Ronaldo
/// ...
/// ## FIRST
/// Cristiano
/// ...
/// ## LAST
/// Ronaldo
/// ...
/// ```
/// Lines starting with a single `#` (not `##`) and blank lines are ignored.
/// HEAD lines are `Handle | Full Name`; FIRST/LAST are one token per line.
fn parse_theme_file(text: &str) -> (Vec<HeadEntry>, Vec<String>, Vec<String>) {
    #[derive(PartialEq)]
    enum Sec {
        None,
        Head,
        First,
        Last,
    }
    let mut sec = Sec::None;
    let (mut head, mut firsts, mut lasts) = (Vec::new(), Vec::new(), Vec::new());
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = line.strip_prefix("## ") {
            sec = match name.trim().to_ascii_uppercase().as_str() {
                "HEAD" => Sec::Head,
                "FIRST" => Sec::First,
                "LAST" => Sec::Last,
                _ => Sec::None,
            };
            continue;
        }
        if line.starts_with('#') {
            continue; // comment
        }
        match sec {
            Sec::Head => {
                if let Some((h, f)) = line.split_once('|') {
                    let handle = h.trim().to_string();
                    let full = f.trim().to_string();
                    if !handle.is_empty() && !full.is_empty() {
                        head.push(HeadEntry { handle, full });
                    }
                }
            }
            Sec::First => firsts.push(line.to_string()),
            Sec::Last => lasts.push(line.to_string()),
            Sec::None => {}
        }
    }
    (head, firsts, lasts)
}

fn load(id: &'static str, label: &'static str, text: &str) -> BuiltinTheme {
    let (head, firsts, lasts) = parse_theme_file(text);
    BuiltinTheme {
        id,
        label,
        head,
        firsts,
        lasts,
    }
}

/// The built-in theme registry, parsed once. Order here is the order shown in
/// the settings picker.
fn builtins() -> &'static [BuiltinTheme] {
    static THEMES: OnceLock<Vec<BuiltinTheme>> = OnceLock::new();
    THEMES.get_or_init(|| {
        vec![
            load("footballers", "Footballers", include_str!("names/data/footballers.txt")),
            load("basketball", "Basketball Stars", include_str!("names/data/basketball.txt")),
            load("movie_stars", "Movie Stars", include_str!("names/data/movie_stars.txt")),
            load("scientists", "Scientists", include_str!("names/data/scientists.txt")),
            load("musicians", "Musicians", include_str!("names/data/musicians.txt")),
            load("authors", "Authors", include_str!("names/data/authors.txt")),
            load("painters", "Painters", include_str!("names/data/painters.txt")),
            load("f1_drivers", "F1 Drivers", include_str!("names/data/f1_drivers.txt")),
        ]
    })
}

fn find_builtin(id: &str) -> Option<&'static BuiltinTheme> {
    builtins().iter().find(|t| t.id == id)
}

/// True when `id` names a built-in theme.
pub fn is_builtin(id: &str) -> bool {
    find_builtin(id).is_some()
}

/// Allocate a name from the builtin theme `id`, unique against `used`
/// (lowercased titles of open sessions). `None` when `id` is unknown.
pub fn allocate_builtin(id: &str, used: &HashSet<String>) -> Option<Allocated> {
    find_builtin(id).map(|t| t.allocate(used))
}

/// Allocate a name from a custom ordered list, unique against `used`. Empty
/// entries are ignored. When every base name is taken, falls back to
/// `"{name} #2"`, `"{name} #3"`, … (recycling base names as sessions close,
/// since `used` is the open-session set). With no usable names, falls back to a
/// generic "Agent #N".
pub fn allocate_custom(names: &[String], used: &HashSet<String>) -> Allocated {
    let clean: Vec<&str> = names
        .iter()
        .map(|n| n.trim())
        .filter(|n| !n.is_empty())
        .collect();
    if clean.is_empty() {
        return numeric_fallback("Agent", used);
    }
    for n in &clean {
        if !used.contains(&n.to_lowercase()) {
            return Allocated {
                title: (*n).to_string(),
                handle: (*n).to_string(),
                full: (*n).to_string(),
            };
        }
    }
    // Every base name taken — suffix in list order, smallest N first.
    let mut suffix = 2;
    loop {
        for n in &clean {
            let title = format!("{n} #{suffix}");
            if !used.contains(&title.to_lowercase()) {
                return Allocated {
                    handle: (*n).to_string(),
                    full: title.clone(),
                    title,
                };
            }
        }
        suffix += 1;
    }
}

/// Public, serialization-friendly description of a builtin theme for the API.
#[derive(Debug, Clone)]
pub struct ThemeInfo {
    pub id: String,
    pub label: String,
    pub capacity: usize,
    pub sample: Vec<String>,
}

/// Metadata for every builtin theme, in display order.
pub fn builtin_theme_infos() -> Vec<ThemeInfo> {
    builtins()
        .iter()
        .map(|t| ThemeInfo {
            id: t.id.to_string(),
            label: t.label.to_string(),
            capacity: t.capacity(),
            sample: t.sample(),
        })
        .collect()
}

// ───────────────────────── address resolver ─────────────────────────

/// A live, addressable session as seen by the resolver.
#[derive(Debug, Clone)]
pub struct Addressable {
    pub id: String,
    /// The short callable handle (`meta.name_handle`), or the title when absent.
    pub handle: String,
    pub title: String,
    pub full: String,
}

/// The outcome of resolving an address prefix on a relay message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Addressed {
    /// Session ids the message is addressed to (empty when unaddressed).
    pub targets: Vec<String>,
    /// The message with the address prefix stripped.
    pub text: String,
    /// True when the prefix was an explicit broadcast keyword (`all`/`everyone`).
    pub broadcast: bool,
}

/// ASCII-fold + lowercase for tolerant matching (`Mbappé` → `mbappe`).
fn fold(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' | 'ø' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ñ' => 'n',
            'ç' => 'c',
            other => other.to_ascii_lowercase(),
        })
        .collect()
}

/// The set of folded tokens a session answers to: its handle, title, and the
/// individual words of its full name (so "diego" matches "Diego Ferreira").
fn answer_tokens(a: &Addressable) -> HashSet<String> {
    let mut set = HashSet::new();
    set.insert(fold(&a.handle));
    set.insert(fold(&a.title));
    for w in a.full.split_whitespace() {
        set.insert(fold(w));
    }
    set
}

/// Strip a single leading `@` and trailing address punctuation from a raw token.
fn clean_token(tok: &str) -> &str {
    tok.trim_start_matches('@').trim_end_matches([',', ':', ';'])
}

/// Resolve a leading name address on `text` against the live `sessions`.
///
/// Recognizes:
///  - `all: …` / `everyone …` / `broadcast …` → broadcast (every session).
///  - `ronaldo: …`, `@ronaldo …`, `ronaldo, messi: …` → those sessions.
///  - bare `ronaldo do X` → the session named Ronaldo (first leading word that
///    matches a known session triggers it).
///
/// Returns the matched ids and the message with the address stripped. When no
/// leading token matches a session (and no broadcast keyword), `targets` is
/// empty and `text` is returned unchanged — the caller should then fall back to
/// its normal handling (e.g. AI orchestration).
pub fn resolve_address(text: &str, sessions: &[Addressable]) -> Addressed {
    let trimmed = text.trim_start();
    let words: Vec<&str> = trimmed.split_whitespace().collect();
    if words.is_empty() {
        return Addressed {
            targets: vec![],
            text: text.to_string(),
            broadcast: false,
        };
    }

    // Explicit broadcast keyword (only as the very first token).
    let first = clean_token(words[0]).to_lowercase();
    if matches!(first.as_str(), "all" | "everyone" | "broadcast" | "everybody") {
        let rest = strip_leading_words(trimmed, 1);
        return Addressed {
            targets: sessions.iter().map(|s| s.id.clone()).collect(),
            text: rest,
            broadcast: true,
        };
    }

    // Pre-compute each session's answer tokens once.
    let token_sets: Vec<(String, HashSet<String>)> = sessions
        .iter()
        .map(|s| (s.id.clone(), answer_tokens(s)))
        .collect();

    // Greedily consume leading name tokens. A token matches when its folded form
    // equals one of some session's answer tokens. Connector words ("and") and a
    // trailing ":" are allowed between names. Stop at the first non-name word.
    let mut targets: Vec<String> = Vec::new();
    let mut consumed = 0usize;
    let mut ended_with_colon = false;
    for (idx, w) in words.iter().enumerate() {
        let bare = clean_token(w);
        if bare.is_empty() {
            break;
        }
        let lower = bare.to_lowercase();
        if (lower == "and" || lower == "&") && !targets.is_empty() {
            consumed = idx + 1;
            continue; // connector between names
        }
        let folded = fold(bare);
        let mut matched_any = false;
        for (id, set) in &token_sets {
            if set.contains(&folded) && !targets.contains(id) {
                targets.push(id.clone());
                matched_any = true;
            }
        }
        if !matched_any {
            break;
        }
        consumed = idx + 1;
        if w.trim_end().ends_with(':') {
            ended_with_colon = true;
            break;
        }
    }

    if targets.is_empty() {
        return Addressed {
            targets: vec![],
            text: text.to_string(),
            broadcast: false,
        };
    }

    let mut rest = strip_leading_words(trimmed, consumed);
    // A leftover leading ":" (e.g. "ronaldo : do X") shouldn't leak into the
    // message.
    if !ended_with_colon {
        rest = rest.trim_start_matches([':', ',']).trim_start().to_string();
    }
    Addressed {
        targets,
        text: rest,
        broadcast: false,
    }
}

/// Return `text` with its first `n` whitespace-delimited words removed,
/// preserving the remainder verbatim.
fn strip_leading_words(text: &str, n: usize) -> String {
    let mut rest = text;
    for _ in 0..n {
        rest = rest.trim_start();
        match rest.find(char::is_whitespace) {
            Some(i) => rest = &rest[i..],
            None => {
                rest = "";
                break;
            }
        }
    }
    rest.trim_start().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn used(names: &[&str]) -> HashSet<String> {
        names.iter().map(|n| n.to_lowercase()).collect()
    }

    #[test]
    fn builtin_allocates_famous_first() {
        // Footballers head is Ronaldo, Messi, ... — first alloc is the icon.
        let a = allocate_builtin("footballers", &HashSet::new()).unwrap();
        assert!(!a.title.is_empty());
        assert_eq!(a.handle, a.title);
        // With the top name used, the next is a different, unused title.
        let u = used(&[&a.title]);
        let b = allocate_builtin("footballers", &u).unwrap();
        assert_ne!(a.title.to_lowercase(), b.title.to_lowercase());
    }

    #[test]
    fn builtin_capacity_is_huge() {
        for info in builtin_theme_infos() {
            assert!(
                info.capacity >= 10_000,
                "theme {} only has {} candidates",
                info.id,
                info.capacity
            );
        }
    }

    #[test]
    fn builtin_never_repeats_under_load() {
        // Allocate 500 names back to back; all must be unique.
        let mut u = HashSet::new();
        for _ in 0..500 {
            let a = allocate_builtin("footballers", &u).unwrap();
            assert!(!u.contains(&a.title.to_lowercase()), "repeat: {}", a.title);
            u.insert(a.title.to_lowercase());
        }
    }

    #[test]
    fn unknown_builtin_is_none() {
        assert!(allocate_builtin("nope", &HashSet::new()).is_none());
    }

    #[test]
    fn custom_recycles_then_suffixes() {
        let names = vec!["Dad".to_string(), "Mom".to_string()];
        let a = allocate_custom(&names, &HashSet::new());
        assert_eq!(a.title, "Dad");
        let b = allocate_custom(&names, &used(&["Dad"]));
        assert_eq!(b.title, "Mom");
        // Both base names taken → suffix scheme.
        let c = allocate_custom(&names, &used(&["Dad", "Mom"]));
        assert_eq!(c.title, "Dad #2");
        let d = allocate_custom(&names, &used(&["Dad", "Mom", "Dad #2"]));
        assert_eq!(d.title, "Mom #2");
    }

    #[test]
    fn custom_empty_falls_back() {
        let a = allocate_custom(&[], &HashSet::new());
        assert_eq!(a.title, "Agent #2");
    }

    fn sess(id: &str, handle: &str, title: &str, full: &str) -> Addressable {
        Addressable {
            id: id.into(),
            handle: handle.into(),
            title: title.into(),
            full: full.into(),
        }
    }

    #[test]
    fn resolve_single_colon() {
        let s = vec![
            sess("1", "Ronaldo", "Ronaldo", "Cristiano Ronaldo"),
            sess("2", "Messi", "Messi", "Lionel Messi"),
        ];
        let r = resolve_address("ronaldo: do X", &s);
        assert_eq!(r.targets, vec!["1"]);
        assert_eq!(r.text, "do X");
        assert!(!r.broadcast);
    }

    #[test]
    fn resolve_bare_leading_name() {
        let s = vec![sess("1", "Ronaldo", "Ronaldo", "Cristiano Ronaldo")];
        let r = resolve_address("ronaldo should do X", &s);
        assert_eq!(r.targets, vec!["1"]);
        assert_eq!(r.text, "should do X");
    }

    #[test]
    fn resolve_multi_and_full_name_word() {
        let s = vec![
            sess("1", "Ronaldo", "Ronaldo", "Cristiano Ronaldo"),
            sess("2", "Messi", "Messi", "Lionel Messi"),
        ];
        let r = resolve_address("ronaldo, messi: ship it", &s);
        assert_eq!(r.targets, vec!["1", "2"]);
        assert_eq!(r.text, "ship it");
        // first-name word of a synthesized session
        let s2 = vec![sess("3", "Ferreira", "Diego Ferreira", "Diego Ferreira")];
        let r2 = resolve_address("diego do it", &s2);
        assert_eq!(r2.targets, vec!["3"]);
    }

    #[test]
    fn resolve_broadcast_keyword() {
        let s = vec![
            sess("1", "Ronaldo", "Ronaldo", "Cristiano Ronaldo"),
            sess("2", "Messi", "Messi", "Lionel Messi"),
        ];
        let r = resolve_address("all: stand down", &s);
        assert_eq!(r.targets, vec!["1", "2"]);
        assert!(r.broadcast);
        assert_eq!(r.text, "stand down");
    }

    #[test]
    fn resolve_unaddressed_is_untouched() {
        let s = vec![sess("1", "Ronaldo", "Ronaldo", "Cristiano Ronaldo")];
        let r = resolve_address("build the feature please", &s);
        assert!(r.targets.is_empty());
        assert_eq!(r.text, "build the feature please");
        assert!(!r.broadcast);
    }

    #[test]
    fn resolve_accent_folding() {
        let s = vec![sess("1", "Mbappe", "Mbappe", "Kylian Mbappé")];
        // typed with the accent should still match the folded handle
        let r = resolve_address("mbappé: go", &s);
        assert_eq!(r.targets, vec!["1"]);
    }
}
