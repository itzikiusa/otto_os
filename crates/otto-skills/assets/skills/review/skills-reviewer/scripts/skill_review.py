#!/usr/bin/env python3
"""Static reviewer for Agent Skills packages.

The script intentionally uses only Python stdlib so it can run in constrained
agent sandboxes. It is not a replacement for human/LLM review; it catches common
structural and quality issues and produces a repeatable baseline report.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import sys
from dataclasses import dataclass, asdict
from pathlib import Path
from typing import Dict, Iterable, List, Optional, Tuple

NAME_RE = re.compile(r"^[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?$")
GENERIC_DESCRIPTION_RE = re.compile(
    r"\b(helps? with|useful for|does things|various tasks|anything|everything|all tasks|general purpose)\b",
    re.IGNORECASE,
)
CONFLICT_PATTERNS = [
    (re.compile(r"always ask", re.I), re.compile(r"never ask", re.I), "ASK_CONFLICT"),
    (re.compile(r"always browse|always search", re.I), re.compile(r"never browse|never search", re.I), "BROWSE_CONFLICT"),
    (re.compile(r"always use scripts?|run scripts? first", re.I), re.compile(r"never use scripts?|do not run scripts?", re.I), "SCRIPT_CONFLICT"),
    (re.compile(r"must cite", re.I), re.compile(r"do not cite|never cite", re.I), "CITATION_CONFLICT"),
]
POLICY_OVERRIDE_RE = re.compile(
    r"\b(ignore|override|bypass|disregard)\b.{0,40}\b(system|developer|safety|policy|higher-priority)\b",
    re.IGNORECASE | re.DOTALL,
)
RISKY_SCRIPT_TEXT_RE = re.compile(
    r"\b(rm\s+-rf|delete\s+all|curl\s+[^\n]*\|\s*(sh|bash)|chmod\s+777|sudo\s+|eval\s+\$|exec\(|subprocess\.|os\.system)\b",
    re.IGNORECASE,
)
REMOTE_URL_RE = re.compile(r"https?://")
CODE_FENCE_RE = re.compile(r"```")
HEADING_RE = re.compile(r"^#{1,6}\s+(.+)$", re.MULTILINE)

SEVERITY_ORDER = {"Critical": 4, "High": 3, "Medium": 2, "Low": 1}


@dataclass
class Finding:
    severity: str
    code: str
    title: str
    evidence: str
    why: str
    fix: str


@dataclass
class Score:
    score: int
    notes: str


def read_text(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except UnicodeDecodeError:
        return path.read_text(encoding="utf-8", errors="replace")


def line_count(text: str) -> int:
    return 0 if not text else text.count("\n") + 1


def parse_frontmatter(text: str) -> Tuple[Dict[str, object], str, Optional[str]]:
    if not text.startswith("---\n"):
        return {}, text, "SKILL.md does not start with YAML frontmatter delimiter '---'."
    end = text.find("\n---", 4)
    if end == -1:
        return {}, text, "SKILL.md frontmatter is not closed with '---'."
    raw = text[4:end].strip("\n")
    body = text[end + len("\n---") :].lstrip("\n")
    data: Dict[str, object] = {}
    current_map: Optional[str] = None
    for idx, raw_line in enumerate(raw.splitlines(), start=1):
        line = raw_line.rstrip()
        if not line or line.lstrip().startswith("#"):
            continue
        if line.startswith("  ") and current_map:
            if ":" in line:
                k, v = line.strip().split(":", 1)
                child = data.setdefault(current_map, {})
                if isinstance(child, dict):
                    child[k.strip()] = clean_scalar(v.strip())
            continue
        current_map = None
        if ":" not in line:
            return data, body, f"Unparseable frontmatter line {idx}: {line}"
        k, v = line.split(":", 1)
        key = k.strip()
        val = v.strip()
        if val == "":
            data[key] = {}
            current_map = key
        else:
            data[key] = clean_scalar(val)
    return data, body, None


def clean_scalar(value: str) -> str:
    if (value.startswith('"') and value.endswith('"')) or (value.startswith("'") and value.endswith("'")):
        return value[1:-1]
    return value


def rel(path: Path, root: Path) -> str:
    try:
        return str(path.relative_to(root))
    except ValueError:
        return str(path)


def has_any(text: str, terms: Iterable[str]) -> bool:
    lower = text.lower()
    return any(term.lower() in lower for term in terms)


def count_examples(root: Path, skill_text: str) -> int:
    count = len(re.findall(r"\bexample\b", skill_text, flags=re.I))
    examples_dir = root / "examples"
    if examples_dir.exists():
        count += sum(1 for p in examples_dir.rglob("*") if p.is_file() and not p.name.startswith("."))
    return count


def find_skill_md(target: Path) -> Tuple[Path, Path]:
    if target.is_file():
        return target.parent, target
    return target, target / "SKILL.md"


def review_skill(target: Path) -> Dict[str, object]:
    root, skill_md = find_skill_md(target.resolve())
    findings: List[Finding] = []
    scorecard: Dict[str, Score] = {}

    if not skill_md.exists():
        findings.append(Finding(
            "Critical", "MISSING_SKILL_MD", "Missing SKILL.md",
            str(skill_md), "A skill package cannot load without SKILL.md.",
            "Add SKILL.md with valid Agent Skills frontmatter and instructions.",
        ))
        return assemble_result(target, None, findings, {"spec_compliance": Score(0, "Missing SKILL.md")}, root)

    text = read_text(skill_md)
    frontmatter, body, fm_error = parse_frontmatter(text)
    name = str(frontmatter.get("name", "")).strip()
    description = str(frontmatter.get("description", "")).strip()

    # Spec compliance
    spec_score = 5
    spec_notes = []
    if fm_error:
        findings.append(Finding("High", "BAD_FRONTMATTER", "Invalid or missing frontmatter", "SKILL.md", fm_error, "Fix YAML frontmatter syntax."))
        spec_score -= 2
        spec_notes.append("frontmatter issue")
    if not name:
        findings.append(Finding("Critical", "MISSING_NAME", "Missing required name", "SKILL.md frontmatter", "Agent Skills require a name field.", "Add lowercase kebab-case name."))
        spec_score = 0
    elif not NAME_RE.match(name) or "--" in name:
        findings.append(Finding("High", "INVALID_NAME", "Invalid skill name", f"name: {name}", "Skill names should be lowercase kebab-case, <=64 chars, with no leading/trailing/consecutive hyphens.", "Rename the skill using lowercase letters, numbers, and single hyphens."))
        spec_score -= 2
    elif target.is_dir() and root.name != name:
        findings.append(Finding("Medium", "NAME_DIR_MISMATCH", "Skill name does not match directory", f"directory={root.name}, name={name}", "The open spec recommends the name match the parent directory for portability.", f"Rename the folder to `{name}` or update `name`."))
        spec_score -= 1
    if not description:
        findings.append(Finding("Critical", "MISSING_DESCRIPTION", "Missing required description", "SKILL.md frontmatter", "Agents rely on description text to select skills.", "Add a concise description with task, trigger terms, and boundaries."))
        spec_score = 0
    elif len(description) > 1024:
        findings.append(Finding("High", "DESCRIPTION_TOO_LONG", "Description exceeds 1024 characters", f"description length={len(description)}", "Long descriptions may violate the spec and be truncated by clients.", "Shorten description and front-load trigger terms."))
        spec_score -= 2
    scorecard["spec_compliance"] = Score(max(spec_score, 0), ", ".join(spec_notes) or "Required frontmatter present")

    # Trigger precision
    trigger_score = 5
    if description and len(description) < 80:
        findings.append(Finding("Medium", "DESCRIPTION_TOO_SHORT", "Description may be too vague", f"description: {description}", "Short descriptions often omit trigger terms and boundaries.", "Add what the skill does, when to use it, and key user phrases."))
        trigger_score -= 1
    if GENERIC_DESCRIPTION_RE.search(description):
        findings.append(Finding("High", "GENERIC_DESCRIPTION", "Description is too generic or broad", f"description: {description}", "Generic activation language causes over-selection or under-selection.", "Replace generic wording with specific trigger tasks and non-triggers."))
        trigger_score -= 2
    if not has_any(description + "\n" + body[:2000], ["use when", "when asked", "trigger", "do not use", "not use", "non-trigger", "scope"]):
        findings.append(Finding("Medium", "MISSING_BOUNDARIES", "Missing clear trigger boundaries", "SKILL.md", "Skills need activation boundaries to avoid conflicts with adjacent skills.", "Add when-to-use and when-not-to-use guidance."))
        trigger_score -= 1
    scorecard["trigger_precision"] = Score(max(trigger_score, 0), "Description and activation boundary review")

    # Workflow quality
    workflow_score = 5
    headings = [h.lower() for h in HEADING_RE.findall(body)]
    if not has_any(body, ["workflow", "steps", "process", "instructions"]):
        findings.append(Finding("High", "NO_WORKFLOW", "No clear workflow section", "SKILL.md body", "A reusable skill needs repeatable steps, not only general advice.", "Add an ordered workflow with inputs, decisions, and outputs."))
        workflow_score -= 2
    if not has_any(body, ["output", "format", "verdict", "deliverable", "result"]):
        findings.append(Finding("Medium", "NO_OUTPUT_CONTRACT", "No explicit output contract", "SKILL.md body", "Without an output contract, reviews vary across runs.", "Add the required output shape or template."))
        workflow_score -= 1
    if len(headings) < 3:
        findings.append(Finding("Low", "FEW_SECTIONS", "Instructions are lightly structured", "SKILL.md headings", "Clear sections help agents follow the workflow.", "Add concise sections for purpose, workflow, examples, and output."))
        workflow_score -= 1
    scorecard["workflow_quality"] = Score(max(workflow_score, 0), "Workflow and output contract review")

    # Examples
    examples_score = 5
    example_count = count_examples(root, text)
    if example_count == 0:
        findings.append(Finding("High", "NO_EXAMPLES", "No examples found", "SKILL.md/examples", "Examples teach the agent expected behavior and boundaries.", "Add at least one positive example and one negative/non-trigger example."))
        examples_score = 1
    elif example_count < 2 or not has_any(text, ["negative example", "non-trigger", "should not", "do not use"]):
        findings.append(Finding("Medium", "MISSING_NEGATIVE_EXAMPLE", "No clear negative/non-trigger example found", "SKILL.md/examples", "Negative examples reduce accidental activation.", "Add an adjacent task that should not use the skill."))
        examples_score -= 2
    if not CODE_FENCE_RE.search(text) and not (root / "examples").exists():
        findings.append(Finding("Low", "NO_CONCRETE_IO_EXAMPLE", "No concrete input/output example detected", "SKILL.md/examples", "Concrete examples make outputs more repeatable.", "Add a realistic prompt and expected answer shape."))
        examples_score -= 1
    scorecard["examples"] = Score(max(examples_score, 0), f"Detected example signals: {example_count}")

    # References
    references_dir = root / "references"
    references_score = 5
    ref_files = [p for p in references_dir.rglob("*") if p.is_file()] if references_dir.exists() else []
    external_links = REMOTE_URL_RE.findall(text)
    if not ref_files and not external_links:
        findings.append(Finding("Medium", "NO_REFERENCES", "No references found", "references/ or links", "References help reviewers verify standards, domain facts, and design choices.", "Add focused reference files or source notes for important external claims."))
        references_score -= 2
    if line_count(text) > 500 and not ref_files:
        findings.append(Finding("High", "NO_PROGRESSIVE_DISCLOSURE", "Large SKILL.md without references", f"SKILL.md lines={line_count(text)}", "Large main files waste context and hide the core workflow.", "Move detailed material into `references/` and link to it."))
        references_score -= 2
    scorecard["references"] = Score(max(references_score, 0), f"Reference files: {len(ref_files)}, external links: {len(external_links)}")

    # Scripts
    scripts_dir = root / "scripts"
    scripts_score = 5
    script_files = [p for p in scripts_dir.rglob("*") if p.is_file()] if scripts_dir.exists() else []
    if script_files:
        for p in script_files:
            stext = read_text(p)
            if RISKY_SCRIPT_TEXT_RE.search(stext):
                sev = "High" if p.suffix in {".py", ".sh", ".js"} else "Medium"
                findings.append(Finding(sev, "RISKY_SCRIPT_PATTERN", "Potentially risky script pattern", rel(p, root), "Scripts that shell out, delete, sudo, or execute dynamic input need explicit guardrails.", "Add dry-run mode, input validation, documentation, and avoid dangerous shell patterns."))
                scripts_score -= 2
            if p.suffix == ".py" and re.search(r"^import\s+(yaml|requests|click|typer)\b", stext, re.M):
                findings.append(Finding("Low", "UNDECLARED_PY_DEP", "Possible non-stdlib dependency", rel(p, root), "Undeclared dependencies reduce portability.", "Document dependencies or use stdlib."))
                scripts_score -= 1
    else:
        scripts_score = 4
    scorecard["scripts"] = Score(max(scripts_score, 0), f"Script files: {len(script_files)}")

    # Evals
    evals_score = 5
    evals_json = root / "evals" / "evals.json"
    if not evals_json.exists():
        findings.append(Finding("High", "NO_EVALS", "No evals/evals.json found", "evals/evals.json", "Reusable skills need evals to catch regressions and activation mistakes.", "Add evals/evals.json with positive, negative, edge, conflict, bloat, and safety cases."))
        evals_score = 1
    else:
        try:
            eval_data = json.loads(read_text(evals_json))
            cases = eval_data.get("cases", []) if isinstance(eval_data, dict) else []
            if len(cases) < 3:
                findings.append(Finding("Medium", "TOO_FEW_EVALS", "Eval suite has too few cases", rel(evals_json, root), "A tiny eval suite will not catch regressions.", "Add cases for good, missing examples, conflict, bloat, negative trigger, and script risk."))
                evals_score -= 2
            serialized = json.dumps(eval_data).lower()
            for term, code in [("negative", "NO_NEGATIVE_EVAL"), ("conflict", "NO_CONFLICT_EVAL"), ("bloat", "NO_BLOAT_EVAL"), ("script", "NO_SCRIPT_EVAL")]:
                if term not in serialized:
                    findings.append(Finding("Low", code, f"Eval suite may not cover {term}", rel(evals_json, root), f"Best-in-class evals include {term} coverage.", f"Add at least one {term} eval case."))
                    evals_score -= 1
        except json.JSONDecodeError as exc:
            findings.append(Finding("High", "BAD_EVALS_JSON", "evals.json is invalid JSON", f"{rel(evals_json, root)}: {exc}", "Broken eval metadata cannot run in CI.", "Fix JSON syntax."))
            evals_score = 1
    scorecard["evals"] = Score(max(evals_score, 0), "Eval suite review")

    # Bloat
    bloat_score = 5
    lines = line_count(text)
    words = len(re.findall(r"\w+", text))
    if lines > 500:
        findings.append(Finding("High", "SKILL_MD_TOO_LONG", "SKILL.md is very long", f"SKILL.md lines={lines}", "Long main instruction files waste context and are harder to maintain.", "Move detailed examples/reference material into separate files."))
        bloat_score -= 3
    elif lines > 250:
        findings.append(Finding("Medium", "SKILL_MD_LONG", "SKILL.md may be bloated", f"SKILL.md lines={lines}", "Main instructions should stay concise unless complexity is justified.", "Trim repetition and move deeper content to references."))
        bloat_score -= 1
    if words > 6000:
        findings.append(Finding("Medium", "HIGH_WORD_COUNT", "High word count", f"SKILL.md words={words}", "High word count increases context cost and hides critical steps.", "Compress instructions and use progressive disclosure."))
        bloat_score -= 1
    scorecard["bloat_control"] = Score(max(bloat_score, 0), f"SKILL.md lines={lines}, words={words}")

    # Conflicts and safety
    # Static conflict checks intentionally focus on SKILL.md. Reference files often
    # contain quoted anti-patterns or examples, so package-wide scans create false
    # positives. A human/LLM review should still inspect referenced instruction files
    # when they are operational rather than explanatory.
    conflict_score = 5
    all_text = text
    for a, b, code in CONFLICT_PATTERNS:
        if a.search(all_text) and b.search(all_text):
            findings.append(Finding("High", code, "Conflicting instructions detected", "package text", "Contradictory rules make behavior unreliable.", "Remove the weaker rule or define explicit precedence."))
            conflict_score -= 2
    if POLICY_OVERRIDE_RE.search(all_text):
        findings.append(Finding("Critical", "POLICY_OVERRIDE", "Potential higher-priority instruction override", "package text", "Skills must not try to bypass system, developer, safety, or user instructions.", "Remove the override language and state that higher-priority instructions always apply."))
        conflict_score = 0
    scorecard["conflict_control"] = Score(max(conflict_score, 0), "Conflict and policy override scan")

    # Maintainability
    maintainability_score = 5
    if "metadata" not in frontmatter or not isinstance(frontmatter.get("metadata"), dict) or "version" not in frontmatter.get("metadata", {}):
        findings.append(Finding("Low", "NO_VERSION_METADATA", "No version metadata found", "SKILL.md frontmatter", "Versioning helps teams review and roll back skills.", "Add metadata.version."))
        maintainability_score -= 1
    if not (root / "README.md").exists():
        findings.append(Finding("Low", "NO_README", "No README found", "README.md", "A README helps install and run the skill outside one conversation.", "Add a brief README with install and usage instructions."))
        maintainability_score -= 1
    scorecard["maintainability"] = Score(max(maintainability_score, 0), "Versioning and package docs review")

    return assemble_result(target, name or None, findings, scorecard, root)


def assemble_result(target: Path, name: Optional[str], findings: List[Finding], scorecard: Dict[str, Score], root: Path) -> Dict[str, object]:
    sorted_findings = sorted(findings, key=lambda f: (-SEVERITY_ORDER.get(f.severity, 0), f.code))
    avg = round(sum(s.score for s in scorecard.values()) / max(len(scorecard), 1), 2)
    max_sev = max((SEVERITY_ORDER.get(f.severity, 0) for f in sorted_findings), default=0)
    high_count = sum(1 for f in sorted_findings if f.severity == "High")
    blocker_codes = {
        "POLICY_OVERRIDE",
        "MISSING_SKILL_MD",
        "MISSING_NAME",
        "MISSING_DESCRIPTION",
        "ASK_CONFLICT",
        "BROWSE_CONFLICT",
        "SCRIPT_CONFLICT",
        "CITATION_CONFLICT",
        "RISKY_SCRIPT_PATTERN",
        "SKILL_MD_TOO_LONG",
    }
    found_codes = {f.code for f in sorted_findings}
    has_blocker = bool(blocker_codes & found_codes)
    if max_sev >= SEVERITY_ORDER["Critical"] or has_blocker:
        verdict = "Do not publish"
    elif high_count > 2 or avg < 3.0:
        verdict = "Do not publish"
    elif high_count or avg < 4.0:
        verdict = "Ready with fixes"
    else:
        verdict = "Ready"

    assets = summarize_assets(root)
    return {
        "target": str(target),
        "skill_name": name or "unknown",
        "verdict": verdict,
        "average_score": avg,
        "scorecard": {k: asdict(v) for k, v in scorecard.items()},
        "findings": [asdict(f) for f in sorted_findings],
        "assets": assets,
    }


def summarize_assets(root: Path) -> Dict[str, str]:
    examples = "present" if (root / "examples").exists() and any((root / "examples").rglob("*")) else "missing"
    references = "present" if (root / "references").exists() and any((root / "references").rglob("*")) else "missing"
    evals = "present" if (root / "evals" / "evals.json").exists() else "missing"
    scripts = "present" if (root / "scripts").exists() and any((root / "scripts").rglob("*")) else "not present"
    return {"examples": examples, "references": references, "evals": evals, "scripts": scripts}


def markdown_report(result: Dict[str, object]) -> str:
    scorecard = result["scorecard"]  # type: ignore[index]
    findings = result["findings"]  # type: ignore[index]
    assets = result["assets"]  # type: ignore[index]
    lines = [
        f"# Skill Review: {result['skill_name']}",
        "",
        "## Verdict",
        f"{result['verdict']} — average score {result['average_score']}/5.",
        "",
        "## Scorecard",
        "| Area | Score | Notes |",
        "| --- | ---: | --- |",
    ]
    for area, score in scorecard.items():
        lines.append(f"| {area} | {score['score']} | {score['notes']} |")
    lines.extend(["", "## Top findings"])
    if findings:
        for idx, f in enumerate(findings[:20], start=1):
            lines.extend([
                f"{idx}. [{f['severity']}] {f['title']} (`{f['code']}`)",
                f"   - Evidence: {f['evidence']}",
                f"   - Why it matters: {f['why']}",
                f"   - Fix: {f['fix']}",
            ])
    else:
        lines.append("No findings detected by static review.")
    lines.extend([
        "",
        "## Missing best-practice assets",
        f"- Examples: {assets['examples']}",
        f"- References: {assets['references']}",
        f"- Evals: {assets['evals']}",
        f"- Scripts: {assets['scripts']}",
        "",
        "## Final recommendation",
        str(result["verdict"]),
    ])
    return "\n".join(lines)


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(description="Review an Agent Skill package for common quality issues.")
    parser.add_argument("target", help="Skill root directory or SKILL.md file")
    parser.add_argument("--format", choices=["markdown", "json"], default="markdown")
    args = parser.parse_args(argv)

    target = Path(args.target)
    result = review_skill(target)
    if args.format == "json":
        print(json.dumps(result, indent=2, ensure_ascii=False))
    else:
        print(markdown_report(result))
    return 0 if result["verdict"] != "Do not publish" else 2


if __name__ == "__main__":
    raise SystemExit(main())
