// Native, dependency-free port of the bundled `skills-reviewer`
// `scripts/skill_review.py`. Included into `skill_review.rs` (shares its imports:
// SkillFinding, SkillScoreRow, SkillStaticReport, Path). Deterministic; produces
// the same verdicts as the reference script on its bundled fixtures.
//
// Three checks go BEYOND the reference script (it has no equivalent):
// `CITED_REFERENCE_MISSING`, `DEAD_PATH` and `PROHIBITION_HEAVY`. None of them
// fire on the bundled fixtures, so the verdict parity asserted above still
// holds; `scripts/skill_review.py` is simply the narrower of the two now.

fn sev_order(s: &str) -> i32 {
    match s {
        "Critical" => 4,
        "High" => 3,
        "Medium" => 2,
        "Low" => 1,
        _ => 0,
    }
}

fn finding(severity: &str, code: &str, title: &str, evidence: &str, why: &str, fix: &str) -> SkillFinding {
    SkillFinding {
        severity: severity.to_string(),
        code: code.to_string(),
        title: title.to_string(),
        evidence: evidence.to_string(),
        why: why.to_string(),
        fix: fix.to_string(),
    }
}

fn read_text(p: &Path) -> Option<String> {
    std::fs::read(p).ok().map(|b| String::from_utf8_lossy(&b).into_owned())
}

fn line_count(t: &str) -> usize {
    if t.is_empty() {
        0
    } else {
        t.matches('\n').count() + 1
    }
}

fn word_count(t: &str) -> usize {
    t.split(|c: char| !c.is_alphanumeric() && c != '_').filter(|s| !s.is_empty()).count()
}

fn has_any(t: &str, terms: &[&str]) -> bool {
    let lower = t.to_lowercase();
    terms.iter().any(|term| lower.contains(&term.to_lowercase()))
}

/// Count files under a dir recursively (skipping dotfiles).
fn count_files(dir: &Path) -> usize {
    fn walk(dir: &Path, n: &mut usize) {
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    continue;
                }
                match e.file_type() {
                    Ok(ft) if ft.is_dir() => walk(&p, n),
                    Ok(ft) if ft.is_file() => *n += 1,
                    _ => {}
                }
            }
        }
    }
    let mut n = 0;
    walk(dir, &mut n);
    n
}

fn list_files(dir: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                match e.file_type() {
                    Ok(ft) if ft.is_dir() => walk(&p, out),
                    Ok(ft) if ft.is_file() => out.push(p),
                    _ => {}
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, &mut out);
    out
}

/// A kebab-case name: `[a-z0-9]`, then `[a-z0-9-]`, ending `[a-z0-9]`, ≤64, no `--`.
fn valid_name(s: &str) -> bool {
    if s.is_empty() || s.len() > 64 || s.contains("--") {
        return false;
    }
    let bytes = s.as_bytes();
    let ok = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    if !ok(bytes[0]) || !ok(bytes[bytes.len() - 1]) {
        return false;
    }
    s.bytes().all(|b| ok(b) || b == b'-')
}

/// True if any word in `firsts` is followed within `window` chars by any word in
/// `seconds` (case-insensitive) — an approximation of the reference proximity
/// regex used for the policy-override check, to avoid false positives.
fn contains_near(text: &str, firsts: &[&str], seconds: &[&str], window: usize) -> bool {
    let lower = text.to_lowercase();
    for first in firsts {
        let mut from = 0;
        while let Some(pos) = lower[from..].find(first) {
            let start = from + pos;
            let end = (start + first.len() + window).min(lower.len());
            let tail = &lower[start + first.len()..end];
            if seconds.iter().any(|s| tail.contains(s)) {
                return true;
            }
            from = start + first.len();
        }
    }
    false
}

/// Drop fenced code blocks (``` … ```) from markdown. Paths and directives
/// inside a fence are almost always *illustrations* — a sample command, a
/// snippet of the output shape — so scanning them produces noise, not findings.
fn strip_fences(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_fence = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if !in_fence {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Characters that mark a path as a placeholder/template rather than a real
/// location (`<skill-root>`, `${HOME}/x`, `path/to/*.md`, `…`).
fn is_placeholder_path(p: &str) -> bool {
    p.contains(['<', '>', '{', '}', '*', '$', '%'])
        || p.contains("...")
        || p.contains('…')
        || p.contains("path/to")
}

/// Extract every `references/<file>` citation from the prose (fences stripped).
/// Returns the cited relative paths, de-duplicated, in first-seen order.
fn cited_references(prose: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let bytes = prose.as_bytes();
    let mut from = 0;
    while let Some(pos) = prose[from..].find("references/") {
        let start = from + pos;
        // Must start at a token boundary, so `my-references/x` does not match.
        let boundary = start == 0 || {
            let b = bytes[start - 1];
            !(b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'/')
        };
        let end = start
            + prose[start..]
                .find(|c: char| c.is_whitespace() || matches!(c, '`' | '"' | '\'' | ')' | ']' | ','))
                .unwrap_or(prose.len() - start);
        let raw = prose[start..end].trim_end_matches(['.', ':', ';']);
        if boundary && raw.len() > "references/".len() && !is_placeholder_path(raw) {
            let s = raw.to_string();
            if !out.contains(&s) {
                out.push(s);
            }
        }
        from = start + "references/".len();
    }
    out
}

/// Extract absolute (`/…`) and home-relative (`~/…`) paths the skill points the
/// agent at. Tuned for precision over recall — a noisy finding on every skill is
/// worth less than no finding at all. Requires two segments, no URL, no
/// placeholder, and **a real file extension on the last segment**.
///
/// That last rule is what makes this usable. Skills are full of API routes
/// (`/api/v1`, `/games/1.0/view/detailed`, `/auth/me`) which are syntactically
/// identical to absolute paths and never exist on disk; over the 59 skills on
/// the development machine they were 8 of 10 hits before the extension rule and
/// 0 of 1 after. The cost is that a dead *directory* pointer is not reported.
///
/// Expansion is pure string work — never `eval`/shell, because this text comes
/// from a skill package we do not trust.
fn cited_abs_paths(prose: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for token in prose.split(|c: char| c.is_whitespace() || matches!(c, '`' | '"' | '\'' | '(' | ')' | '[' | ']' | ',')) {
        let t = token.trim_end_matches(['.', ':', ';', '!', '?']);
        let is_abs = t.starts_with('/') || t.starts_with("~/");
        if !is_abs || t.len() < 6 || is_placeholder_path(t) || t.contains("://") {
            continue;
        }
        let segs: Vec<&str> = t.trim_start_matches('~').split('/').filter(|s| !s.is_empty()).collect();
        if segs.len() < 2 {
            continue;
        }
        // A file extension: 1-5 chars, all alphabetic. `.go`/`.md` qualify;
        // the `.0` of a `1.0.0` version segment does not.
        let has_ext = segs
            .last()
            .and_then(|s| s.rsplit_once('.'))
            .map(|(_, ext)| (1..=5).contains(&ext.len()) && ext.chars().all(|c| c.is_ascii_alphabetic()))
            .unwrap_or(false);
        if !has_ext {
            continue;
        }
        let s = t.to_string();
        if !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

/// Resolve a `~/…` prefix against the real home directory. Pure string work.
fn expand_home(p: &str) -> Option<PathBuf> {
    if let Some(rest) = p.strip_prefix("~/") {
        dirs::home_dir().map(|h| h.join(rest))
    } else if p.starts_with('/') {
        Some(PathBuf::from(p))
    } else {
        None
    }
}

/// Words that direct by prohibition, and words that supply a reason. The ratio
/// between them is the signal — a skill that says "never X" ten times without
/// once saying why is a list of traps; the same skill with reasons generalises.
const DIRECTIVE_WORDS: &[&str] = &["never", "always", "must", "mandatory"];
const RATIONALE_WORDS: &[&str] = &["because", "so that", "otherwise", "reason", "which is why"];

const GENERIC_TERMS: &[&str] = &[
    "helps with", "help with", "useful for", "does things", "various tasks", "anything",
    "everything", "all tasks", "general purpose",
];
const RISKY_TERMS: &[&str] = &[
    "rm -rf", "delete all", "chmod 777", "sudo ", "eval $", "exec(", "subprocess.", "os.system",
];

/// The deterministic static review of a skill package rooted at `dir`.
fn static_review(dir: &Path) -> SkillStaticReport {
    let mut findings: Vec<SkillFinding> = Vec::new();
    let mut scorecard: Vec<SkillScoreRow> = Vec::new();
    let mut score = |area: &str, s: i32, notes: &str| {
        scorecard.push(SkillScoreRow { area: area.to_string(), score: s.clamp(0, 5) as u8, notes: notes.to_string() });
    };

    let skill_md = dir.join("SKILL.md");
    let Some(text) = read_text(&skill_md) else {
        findings.push(finding(
            "Critical", "MISSING_SKILL_MD", "Missing SKILL.md", &skill_md.to_string_lossy(),
            "A skill package cannot load without SKILL.md.",
            "Add SKILL.md with valid Agent Skills frontmatter and instructions.",
        ));
        score("spec_compliance", 0, "Missing SKILL.md");
        return assemble(findings, scorecard);
    };

    // Frontmatter.
    let dir_name = dir.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
    let (fm_name, description, body, fm_ok) = parse_frontmatter(&text);
    // Instructions with code fences removed — what the agent is *told*, minus the
    // illustrations. Path and directive scanning run over this, not `text`.
    let prose = strip_fences(&text);
    // The effective name falls back to the directory (Otto skills omit `name:`),
    // so MISSING_NAME never fires; only an *explicit* invalid name is flagged below.
    let description = description.unwrap_or_default();

    // --- spec_compliance ---
    let mut spec = 5i32;
    let mut spec_notes = String::from("Required frontmatter present");
    if !fm_ok {
        findings.push(finding("High", "BAD_FRONTMATTER", "Invalid or missing frontmatter", "SKILL.md", "Skill selection + parsing depend on valid frontmatter.", "Fix YAML frontmatter syntax."));
        spec -= 2;
        spec_notes = "frontmatter issue".into();
    }
    // name is always resolvable (falls back to the dir); only flag an *explicit*
    // invalid name.
    if let Some(explicit) = fm_name.as_ref().filter(|n| !n.is_empty()) {
        if !valid_name(explicit) {
            findings.push(finding("High", "INVALID_NAME", "Invalid skill name", &format!("name: {explicit}"), "Names should be lowercase kebab-case, <=64 chars, no leading/trailing/consecutive hyphens.", "Rename using lowercase letters, numbers, and single hyphens."));
            spec -= 2;
        } else if !dir_name.is_empty() && &dir_name != explicit {
            findings.push(finding("Medium", "NAME_DIR_MISMATCH", "Skill name does not match directory", &format!("directory={dir_name}, name={explicit}"), "The spec recommends the name match the parent directory for portability.", &format!("Rename the folder to `{explicit}` or update `name`.")));
            spec -= 1;
        }
    }
    if description.is_empty() {
        findings.push(finding("Critical", "MISSING_DESCRIPTION", "Missing required description", "SKILL.md frontmatter", "Agents rely on description text to select skills.", "Add a concise description with task, trigger terms, and boundaries."));
        spec = 0;
    } else if description.len() > 1024 {
        findings.push(finding("High", "DESCRIPTION_TOO_LONG", "Description exceeds 1024 characters", &format!("description length={}", description.len()), "Long descriptions may be truncated by clients.", "Shorten description and front-load trigger terms."));
        spec -= 2;
    }
    score("spec_compliance", spec, &spec_notes);

    // --- trigger_precision ---
    let mut trigger = 5i32;
    if !description.is_empty() && description.len() < 80 {
        findings.push(finding("Medium", "DESCRIPTION_TOO_SHORT", "Description may be too vague", &format!("description: {description}"), "Short descriptions often omit trigger terms and boundaries.", "Add what it does, when to use it, and key user phrases."));
        trigger -= 1;
    }
    if has_any(&description, GENERIC_TERMS) {
        findings.push(finding("High", "GENERIC_DESCRIPTION", "Description is too generic or broad", &format!("description: {description}"), "Generic activation language causes over/under-selection.", "Replace generic wording with specific trigger tasks and non-triggers."));
        trigger -= 2;
    }
    let head: String = body.chars().take(2000).collect();
    if !has_any(&format!("{description}\n{head}"), &["use when", "when asked", "trigger", "do not use", "not use", "non-trigger", "scope"]) {
        findings.push(finding("Medium", "MISSING_BOUNDARIES", "Missing clear trigger boundaries", "SKILL.md", "Skills need activation boundaries to avoid conflicts.", "Add when-to-use and when-not-to-use guidance."));
        trigger -= 1;
    }
    score("trigger_precision", trigger, "Description and activation boundary review");

    // --- workflow_quality ---
    let mut workflow = 5i32;
    let headings = body.lines().filter(|l| is_heading(l)).count();
    if !has_any(&body, &["workflow", "steps", "process", "instructions"]) {
        findings.push(finding("High", "NO_WORKFLOW", "No clear workflow section", "SKILL.md body", "A reusable skill needs repeatable steps.", "Add an ordered workflow with inputs, decisions, and outputs."));
        workflow -= 2;
    }
    if !has_any(&body, &["output", "format", "verdict", "deliverable", "result"]) {
        findings.push(finding("Medium", "NO_OUTPUT_CONTRACT", "No explicit output contract", "SKILL.md body", "Without an output contract, results vary across runs.", "Add the required output shape or template."));
        workflow -= 1;
    }
    if headings < 3 {
        findings.push(finding("Low", "FEW_SECTIONS", "Instructions are lightly structured", "SKILL.md headings", "Clear sections help agents follow the workflow.", "Add sections for purpose, workflow, examples, and output."));
        workflow -= 1;
    }
    score("workflow_quality", workflow, "Workflow and output contract review");

    // --- examples ---
    let mut examples = 5i32;
    // Whole-word "example" (matches the reference `\bexample\b` — plural
    // "examples" does NOT count) plus any files under examples/.
    let example_count = count_whole_word(&text, "example") + count_files(&dir.join("examples"));
    if example_count == 0 {
        findings.push(finding("High", "NO_EXAMPLES", "No examples found", "SKILL.md/examples", "Examples teach expected behavior and boundaries.", "Add at least one positive and one negative/non-trigger example."));
        examples = 1;
    } else if example_count < 2 || !has_any(&text, &["negative example", "non-trigger", "should not", "do not use"]) {
        findings.push(finding("Medium", "MISSING_NEGATIVE_EXAMPLE", "No clear negative/non-trigger example found", "SKILL.md/examples", "Negative examples reduce accidental activation.", "Add an adjacent task that should not use the skill."));
        examples -= 2;
    }
    if !text.contains("```") && !dir.join("examples").exists() {
        findings.push(finding("Low", "NO_CONCRETE_IO_EXAMPLE", "No concrete input/output example detected", "SKILL.md/examples", "Concrete examples make outputs repeatable.", "Add a realistic prompt and expected answer shape."));
        examples -= 1;
    }
    score("examples", examples, &format!("Detected example signals: {example_count}"));

    // --- references ---
    let mut references = 5i32;
    let ref_files = count_files(&dir.join("references"));
    let external_links = count_word(&text, "http");
    if ref_files == 0 && external_links == 0 {
        findings.push(finding("Medium", "NO_REFERENCES", "No references found", "references/ or links", "References help verify standards and design choices.", "Add focused reference files or source notes."));
        references -= 2;
    }
    if line_count(&text) > 500 && ref_files == 0 {
        findings.push(finding("High", "NO_PROGRESSIVE_DISCLOSURE", "Large SKILL.md without references", &format!("SKILL.md lines={}", line_count(&text)), "Large main files waste context and hide the core workflow.", "Move detailed material into references/ and link to it."));
        references -= 2;
    }
    // A citation that does not resolve is unreachable: the agent has no path to
    // the content the skill told it to read. Deterministic — the file is either
    // in the package or it is not — so this is a High.
    for cite in cited_references(&prose) {
        if !dir.join(&cite).exists() {
            findings.push(finding("High", "CITED_REFERENCE_MISSING", "Cited reference file does not exist", &cite, "The agent is told to read a file that is not in the package, so the guidance is silently lost.", &format!("Add `{cite}` to the package, or drop the citation.")));
            references -= 2;
        }
    }
    score("references", references, &format!("Reference files: {ref_files}, external links: {external_links}"));

    // --- scripts ---
    let mut scripts = 5i32;
    let scripts_dir = dir.join("scripts");
    let script_files = if scripts_dir.exists() { list_files(&scripts_dir) } else { vec![] };
    if script_files.is_empty() {
        scripts = 4;
    } else {
        for p in &script_files {
            let stext = read_text(p).unwrap_or_default();
            let risky = has_any(&stext, RISKY_TERMS)
                || (stext.to_lowercase().contains("curl") && (stext.contains("| sh") || stext.contains("|sh") || stext.contains("| bash")));
            if risky {
                let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
                let sev = if matches!(ext, "py" | "sh" | "js") { "High" } else { "Medium" };
                findings.push(finding(sev, "RISKY_SCRIPT_PATTERN", "Potentially risky script pattern", &p.file_name().unwrap_or_default().to_string_lossy(), "Scripts that shell out, delete, sudo, or exec dynamic input need guardrails.", "Add dry-run mode, input validation, docs, and avoid dangerous shell patterns."));
                scripts -= 2;
            }
            if p.extension().and_then(|e| e.to_str()) == Some("py") {
                let imports_dep = stext.lines().any(|l| {
                    let t = l.trim_start();
                    ["import yaml", "import requests", "import click", "import typer"].iter().any(|d| t.starts_with(d))
                });
                if imports_dep {
                    findings.push(finding("Low", "UNDECLARED_PY_DEP", "Possible non-stdlib dependency", &p.file_name().unwrap_or_default().to_string_lossy(), "Undeclared dependencies reduce portability.", "Document dependencies or use stdlib."));
                    scripts -= 1;
                }
            }
        }
    }
    score("scripts", scripts, &format!("Script files: {}", script_files.len()));

    // --- evals ---
    let mut evals = 5i32;
    let evals_json = dir.join("evals").join("evals.json");
    if !evals_json.exists() {
        findings.push(finding("High", "NO_EVALS", "No evals/evals.json found", "evals/evals.json", "Reusable skills need evals to catch regressions and activation mistakes.", "Add evals/evals.json with positive, negative, edge, conflict, bloat, and safety cases."));
        evals = 1;
    } else {
        match read_text(&evals_json).and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()) {
            Some(val) => {
                let cases = val.get("cases").and_then(|c| c.as_array()).map(|a| a.len()).unwrap_or(0);
                if cases < 3 {
                    findings.push(finding("Medium", "TOO_FEW_EVALS", "Eval suite has too few cases", "evals/evals.json", "A tiny eval suite will not catch regressions.", "Add cases for good, missing examples, conflict, bloat, negative trigger, and script risk."));
                    evals -= 2;
                }
                let serialized = val.to_string().to_lowercase();
                for (term, code) in [("negative", "NO_NEGATIVE_EVAL"), ("conflict", "NO_CONFLICT_EVAL"), ("bloat", "NO_BLOAT_EVAL"), ("script", "NO_SCRIPT_EVAL")] {
                    if !serialized.contains(term) {
                        findings.push(finding("Low", code, &format!("Eval suite may not cover {term}"), "evals/evals.json", "Best-in-class evals include this coverage.", &format!("Add at least one {term} eval case.")));
                        evals -= 1;
                    }
                }
            }
            None => {
                findings.push(finding("High", "BAD_EVALS_JSON", "evals.json is invalid JSON", "evals/evals.json", "Broken eval metadata cannot run in CI.", "Fix JSON syntax."));
                evals = 1;
            }
        }
    }
    score("evals", evals, "Eval suite review");

    // --- bloat_control ---
    let mut bloat = 5i32;
    let lines = line_count(&text);
    let words = word_count(&text);
    if lines > 500 {
        findings.push(finding("High", "SKILL_MD_TOO_LONG", "SKILL.md is very long", &format!("SKILL.md lines={lines}"), "Long main instruction files waste context and are harder to maintain.", "Move detailed examples/reference material into separate files."));
        bloat -= 3;
    } else if lines > 250 {
        findings.push(finding("Medium", "SKILL_MD_LONG", "SKILL.md may be bloated", &format!("SKILL.md lines={lines}"), "Main instructions should stay concise unless complexity is justified.", "Trim repetition and move deeper content to references."));
        bloat -= 1;
    }
    if words > 6000 {
        findings.push(finding("Medium", "HIGH_WORD_COUNT", "High word count", &format!("SKILL.md words={words}"), "High word count increases context cost and hides critical steps.", "Compress instructions and use progressive disclosure."));
        bloat -= 1;
    }
    // Prohibition density. A pile of NEVER/ALWAYS/MUST lines with no stated
    // reason is instructions competing for attention: each one names a single
    // trap, none of them describe the goal, and the agent cannot generalise past
    // the listed cases. Heuristic and phrasing-level, so it is Low and does NOT
    // move the score — it must never flip a promote gate on its own.
    let directives: usize = DIRECTIVE_WORDS.iter().map(|w| count_whole_word(&prose, w)).sum();
    let rationale: usize = RATIONALE_WORDS.iter().map(|w| count_word(&prose, w)).sum();
    if directives >= 12 && rationale * 3 < directives {
        findings.push(finding(
            "Low", "PROHIBITION_HEAVY", "Directives outnumber the reasons for them",
            &format!("{directives} NEVER/ALWAYS/MUST-style directives, {rationale} stated reasons"),
            "Rules without a reason only cover the case they name; an agent that knows the goal handles the ones they do not.",
            "Rewrite the weakest as the positive behaviour plus a one-clause why (and a check that confirms it). Keep prohibitions for destructive actions and for when NOT to use the skill.",
        ));
    }
    score("bloat_control", bloat, &format!("SKILL.md lines={lines}, words={words}, directives={directives}/reasons={rationale}"));

    // --- conflict_control ---
    let mut conflict = 5i32;
    let lower = text.to_lowercase();
    let pairs = [
        (vec!["always ask"], vec!["never ask"], "ASK_CONFLICT"),
        (vec!["always browse", "always search"], vec!["never browse", "never search"], "BROWSE_CONFLICT"),
        (vec!["always use script", "run scripts first", "always use scripts"], vec!["never use script", "do not run script"], "SCRIPT_CONFLICT"),
        (vec!["must cite"], vec!["do not cite", "never cite"], "CITATION_CONFLICT"),
    ];
    for (a, b, code) in pairs {
        if a.iter().any(|x| lower.contains(x)) && b.iter().any(|x| lower.contains(x)) {
            findings.push(finding("High", code, "Conflicting instructions detected", "package text", "Contradictory rules make behavior unreliable.", "Remove the weaker rule or define explicit precedence."));
            conflict -= 2;
        }
    }
    if contains_near(&text, &["ignore", "override", "bypass", "disregard"], &["system", "developer", "safety", "policy", "higher-priority"], 40) {
        findings.push(finding("Critical", "POLICY_OVERRIDE", "Potential higher-priority instruction override", "package text", "Skills must not bypass system, developer, safety, or user instructions.", "Remove override language; state that higher-priority instructions always apply."));
        conflict = 0;
    }
    score("conflict_control", conflict, "Conflict and policy override scan");

    // --- maintainability ---
    let mut maintainability = 5i32;
    if !has_version_metadata(&text) {
        findings.push(finding("Low", "NO_VERSION_METADATA", "No version metadata found", "SKILL.md frontmatter", "Versioning helps teams review and roll back skills.", "Add metadata.version (or a top-level version)."));
        maintainability -= 1;
    }
    if !dir.join("README.md").exists() {
        findings.push(finding("Low", "NO_README", "No README found", "README.md", "A README helps install and run the skill outside one conversation.", "Add a brief README with install and usage instructions."));
        maintainability -= 1;
    }
    // Absolute paths that no longer resolve. A better model follows a stale
    // pointer faster and more confidently than a weak instruction, so this is a
    // correctness problem, not tidiness. Medium rather than High because it is
    // machine-dependent: a path can be real on the author's box and absent here.
    let mut dead: Vec<String> = Vec::new();
    for p in cited_abs_paths(&prose) {
        if expand_home(&p).map(|full| !full.exists()).unwrap_or(false) {
            dead.push(p);
        }
    }
    if !dead.is_empty() {
        findings.push(finding(
            "Medium", "DEAD_PATH", "Cited path does not exist on this machine",
            &dead.join(", "),
            "An agent follows a cited path without hesitating; a stale one sends it to the wrong place or to nothing at all.",
            "Update the path, make it relative to the package, or state how to locate the file instead of hardcoding a location.",
        ));
        maintainability -= 1;
    }
    score("maintainability", maintainability, &format!("Versioning and package docs review; dead paths: {}", dead.len()));

    assemble(findings, scorecard)
}

fn assemble(mut findings: Vec<SkillFinding>, scorecard: Vec<SkillScoreRow>) -> SkillStaticReport {
    findings.sort_by(|a, b| sev_order(&b.severity).cmp(&sev_order(&a.severity)).then(a.code.cmp(&b.code)));
    let sum: i32 = scorecard.iter().map(|s| s.score as i32).sum();
    let avg = if scorecard.is_empty() { 0.0 } else { (sum as f32 / scorecard.len() as f32 * 100.0).round() / 100.0 };
    let max_sev = findings.iter().map(|f| sev_order(&f.severity)).max().unwrap_or(0);
    let high_count = findings.iter().filter(|f| f.severity == "High").count();
    let blockers = [
        "POLICY_OVERRIDE", "MISSING_SKILL_MD", "MISSING_NAME", "MISSING_DESCRIPTION", "ASK_CONFLICT",
        "BROWSE_CONFLICT", "SCRIPT_CONFLICT", "CITATION_CONFLICT", "RISKY_SCRIPT_PATTERN", "SKILL_MD_TOO_LONG",
    ];
    let has_blocker = findings.iter().any(|f| blockers.contains(&f.code.as_str()));
    let verdict = if max_sev >= sev_order("Critical") || has_blocker || high_count > 2 || avg < 3.0 {
        "Do not publish"
    } else if high_count > 0 || avg < 4.0 {
        "Ready with fixes"
    } else {
        "Ready"
    };
    SkillStaticReport { verdict: verdict.to_string(), average_score: avg, scorecard, findings }
}

/// Parse `(name, description, body_after_frontmatter, frontmatter_ok)`.
fn parse_frontmatter(text: &str) -> (Option<String>, Option<String>, String, bool) {
    if !text.starts_with("---") {
        return (None, None, text.to_string(), false);
    }
    // Find the closing fence after the first line.
    let mut name = None;
    let mut description = None;
    let mut end_idx = None;
    let mut lines = text.lines();
    lines.next(); // opening ---
    let mut consumed = text.find('\n').map(|i| i + 1).unwrap_or(text.len());
    for line in lines {
        let line_len = line.len() + 1;
        if line.trim() == "---" {
            end_idx = Some(consumed + line_len);
            break;
        }
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("name:") {
            if name.is_none() {
                name = Some(unquote(rest.trim()));
            }
        } else if let Some(rest) = t.strip_prefix("description:") {
            if description.is_none() {
                description = Some(unquote(rest.trim()));
            }
        }
        consumed += line_len;
    }
    match end_idx {
        Some(i) => {
            let body = text.get(i..).unwrap_or("").trim_start_matches('\n').to_string();
            (name, description, body, true)
        }
        None => (name, description, text.to_string(), false),
    }
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    s.trim_matches('"').trim_matches('\'').to_string()
}

fn is_heading(line: &str) -> bool {
    let t = line.trim_start();
    let hashes = t.chars().take_while(|&c| c == '#').count();
    (1..=6).contains(&hashes) && t[hashes..].starts_with(char::is_whitespace) && t.trim().len() > hashes + 1
}

fn count_word(text: &str, word: &str) -> usize {
    let lower = text.to_lowercase();
    let w = word.to_lowercase();
    if w.is_empty() {
        return 0;
    }
    let mut n = 0;
    let mut from = 0;
    while let Some(pos) = lower[from..].find(&w) {
        n += 1;
        from += pos + w.len();
    }
    n
}

/// Whole-word (case-insensitive) count of `word`, mirroring the reference
/// `\bword\b`: the match must be bounded by non-word chars (or string ends).
/// A word char is `[A-Za-z0-9_]`, so "examples" does not count "example".
fn count_whole_word(text: &str, word: &str) -> usize {
    let lower = text.to_lowercase();
    let bytes = lower.as_bytes();
    let w = word.to_lowercase();
    if w.is_empty() {
        return 0;
    }
    let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut n = 0;
    let mut from = 0;
    while let Some(pos) = lower[from..].find(&w) {
        let start = from + pos;
        let end = start + w.len();
        let before_ok = start == 0 || !is_word(bytes[start - 1]);
        let after_ok = end >= bytes.len() || !is_word(bytes[end]);
        if before_ok && after_ok {
            n += 1;
        }
        from = start + w.len();
    }
    n
}

/// True when the frontmatter carries a version marker (top-level `version:` or a
/// nested `metadata:` `version:`).
fn has_version_metadata(text: &str) -> bool {
    let (_, _, _, ok) = parse_frontmatter(text);
    if !ok {
        return false;
    }
    // Scan the frontmatter block only.
    let fm = text.split("---").nth(1).unwrap_or("");
    fm.lines().any(|l| l.trim_start().starts_with("version:"))
}

#[cfg(test)]
mod static_tests {
    use super::*;

    /// Stage the bundled skills-reviewer (with its eval fixtures) into a tempdir
    /// and run the native static reviewer against each fixture case — asserting
    /// the same verdicts the reference python script produces.
    fn staged_cases() -> (tempfile::TempDir, std::path::PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let lib = otto_context::Library::new(tmp.path());
        assert!(otto_skills::install_into(&lib, "skills-reviewer").unwrap());
        let cases = tmp.path().join("skills/skills-reviewer/evals/cases");
        (tmp, cases)
    }

    fn codes(r: &SkillStaticReport) -> Vec<String> {
        r.findings.iter().map(|f| f.code.clone()).collect()
    }

    #[test]
    fn good_focused_is_ready() {
        let (_t, cases) = staged_cases();
        let r = static_review(&cases.join("good-focused"));
        assert_eq!(r.verdict, "Ready", "codes={:?}", codes(&r));
        assert!(r.average_score >= 4.0);
        assert!(!codes(&r).contains(&"NO_EXAMPLES".to_string()));
    }

    #[test]
    fn bad_bloated_do_not_publish() {
        let (_t, cases) = staged_cases();
        let r = static_review(&cases.join("bad-bloated"));
        assert_eq!(r.verdict, "Do not publish", "codes={:?}", codes(&r));
        assert!(codes(&r).contains(&"SKILL_MD_TOO_LONG".to_string()));
        assert!(codes(&r).contains(&"GENERIC_DESCRIPTION".to_string()));
    }

    #[test]
    fn bad_conflicts_do_not_publish() {
        let (_t, cases) = staged_cases();
        let r = static_review(&cases.join("bad-conflicts"));
        assert_eq!(r.verdict, "Do not publish", "codes={:?}", codes(&r));
        assert!(codes(&r).contains(&"ASK_CONFLICT".to_string()));
    }

    #[test]
    fn bad_no_examples_ready_with_fixes() {
        let (_t, cases) = staged_cases();
        let r = static_review(&cases.join("bad-no-examples"));
        assert_eq!(r.verdict, "Ready with fixes", "codes={:?}", codes(&r));
        assert!(codes(&r).contains(&"NO_EXAMPLES".to_string()));
    }

    #[test]
    fn bad_script_risk_do_not_publish() {
        let (_t, cases) = staged_cases();
        let r = static_review(&cases.join("bad-script-risk"));
        assert_eq!(r.verdict, "Do not publish", "codes={:?}", codes(&r));
        assert!(codes(&r).contains(&"RISKY_SCRIPT_PATTERN".to_string()));
    }

    #[test]
    fn missing_skill_md_is_critical() {
        let tmp = tempfile::tempdir().unwrap();
        let r = static_review(tmp.path());
        assert_eq!(r.verdict, "Do not publish");
        assert!(codes(&r).contains(&"MISSING_SKILL_MD".to_string()));
    }

    /// Write a one-off package and review it.
    fn review_pkg(files: &[(&str, &str)]) -> (tempfile::TempDir, SkillStaticReport) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("probe");
        for (rel, body) in files {
            let p = dir.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, body).unwrap();
        }
        let r = static_review(&dir);
        (tmp, r)
    }

    const HEAD: &str = "---\nname: probe\ndescription: Reviews probe packages for citation \
        integrity when asked to audit a probe skill; do not use for generic review.\n---\n\n\
        # Probe\n\n## Workflow\n\n1. Step one.\n\n## Output format\n\nA verdict.\n\n\
        ## Example\n\nPositive example above; non-trigger: do not use for prose.\n";

    #[test]
    fn cited_reference_that_exists_is_not_flagged() {
        let (_t, r) = review_pkg(&[
            ("SKILL.md", &format!("{HEAD}\nSee `references/notes.md`.\n")),
            ("references/notes.md", "notes"),
        ]);
        assert!(!codes(&r).contains(&"CITED_REFERENCE_MISSING".to_string()), "codes={:?}", codes(&r));
    }

    #[test]
    fn cited_reference_that_is_absent_is_high() {
        let (_t, r) = review_pkg(&[
            ("SKILL.md", &format!("{HEAD}\nSee `references/gone.md`.\n")),
            ("references/notes.md", "notes"),
        ]);
        let f = r.findings.iter().find(|f| f.code == "CITED_REFERENCE_MISSING").expect("finding");
        assert_eq!(f.severity, "High");
        assert_eq!(f.evidence, "references/gone.md");
    }

    /// A path inside a fenced block is an illustration, not a pointer.
    #[test]
    fn paths_inside_code_fences_are_ignored() {
        let body = format!("{HEAD}\n```bash\ncat /nope/does-not-exist-xyz/file.md\n```\n");
        let (_t, r) = review_pkg(&[("SKILL.md", &body)]);
        assert!(!codes(&r).contains(&"DEAD_PATH".to_string()), "codes={:?}", codes(&r));
    }

    #[test]
    fn dead_absolute_path_in_prose_is_flagged() {
        let body = format!("{HEAD}\nRead /nope/does-not-exist-xyz/file.md before starting.\n");
        let (_t, r) = review_pkg(&[("SKILL.md", &body)]);
        let f = r.findings.iter().find(|f| f.code == "DEAD_PATH").expect("finding");
        assert_eq!(f.severity, "Medium");
        assert!(f.evidence.contains("/nope/does-not-exist-xyz/file.md"));
    }

    /// API routes look exactly like absolute paths and never exist on disk.
    /// Before the extension rule these were most of what the check reported.
    #[test]
    fn api_route_fragments_are_not_paths() {
        let body = format!(
            "{HEAD}\nCall /api/v1 then /auth/me, and POST /invoices/generate/1.0.0 \
             plus /games/1.0/view/detailed when done.\n"
        );
        let (_t, r) = review_pkg(&[("SKILL.md", &body)]);
        assert!(!codes(&r).contains(&"DEAD_PATH".to_string()), "codes={:?}", codes(&r));
    }

    /// Placeholders and URLs are not machine paths and must not be resolved.
    #[test]
    fn placeholder_and_url_paths_are_ignored() {
        let body = format!(
            "{HEAD}\nRun it on <skill-root>/SKILL.md, see /path/to/thing.md and \
             https://example.com/docs/guide.md for details.\n"
        );
        let (_t, r) = review_pkg(&[("SKILL.md", &body)]);
        assert!(!codes(&r).contains(&"DEAD_PATH".to_string()), "codes={:?}", codes(&r));
    }

    #[test]
    fn prohibition_heavy_fires_low_without_moving_the_score() {
        let rules = "You MUST do it. NEVER skip. ALWAYS check. MUST verify. NEVER guess. \
                     ALWAYS report. MUST log. NEVER cache. ALWAYS retry. MUST close. \
                     NEVER inline. ALWAYS flush.\n";
        let with = format!("{HEAD}\n{rules}");
        let without = format!("{HEAD}\nDo it, and check the result.\n");
        let (_a, heavy) = review_pkg(&[("SKILL.md", &with)]);
        let (_b, light) = review_pkg(&[("SKILL.md", &without)]);
        assert!(codes(&heavy).contains(&"PROHIBITION_HEAVY".to_string()), "codes={:?}", codes(&heavy));
        assert!(!codes(&light).contains(&"PROHIBITION_HEAVY".to_string()));
        // Low + no deduction: the finding must never be what flips a gate.
        let bloat = |r: &SkillStaticReport| {
            r.scorecard.iter().find(|s| s.area == "bloat_control").unwrap().score
        };
        assert_eq!(bloat(&heavy), bloat(&light));
        assert_eq!(heavy.verdict, light.verdict);
    }

    /// Stating the reasons defuses it — the ratio is the signal, not the count.
    #[test]
    fn directives_with_reasons_do_not_fire() {
        let rules = "You MUST do it because it breaks otherwise. NEVER skip, because the \
                     reason is data loss. ALWAYS check so that state stays valid. MUST verify \
                     because retries are unsafe otherwise. NEVER guess — the reason is silent \
                     corruption. ALWAYS report so that operators see it. MUST log because \
                     audits need it. NEVER cache, otherwise reads go stale. ALWAYS retry so \
                     that flakes clear. MUST close because handles leak. NEVER inline, \
                     otherwise diffs explode. ALWAYS flush because buffers drop.\n";
        let (_t, r) = review_pkg(&[("SKILL.md", &format!("{HEAD}\n{rules}"))]);
        assert!(!codes(&r).contains(&"PROHIBITION_HEAVY".to_string()), "codes={:?}", codes(&r));
    }
}
