#!/usr/bin/env python3
"""Scrub a private workflow export into committed compatibility fixtures.

    scripts/scrub-workflow-export.py <defs-dir> <runs-dir> <out-dir> \
        [--order stem,stem,stem,stem,stem] [--traces N]

Reads the exported definitions (<defs-dir>/*.json, the API's Workflow shape)
and their runs (<runs-dir>/runs/<stem>/*.json + sessions.csv) and writes

    <out-dir>/clones/{a..e}.json              structure-identical, content-scrubbed definitions
    <out-dir>/traces/<clone>/<scenario>.json  what the old engine did, as numbers and classes

The clone keeps the graph exactly (node and edge ids, sources, targets, kinds,
every param KEY, every boolean and number, version, on_restart, positions
rounded to 10) and the strings that are Otto's own vocabulary (provider names,
bundled lens and skill names read from this repository, review modes,
expressions made only of allowed words). Everything else is replaced: names
become "<kind word> <n>", prompts become "[compat]" + filler of the original
length bucket + a References line of the contract tokens the original
mentioned + "OTTO_FAKE: @script", notify messages keep their {placeholders}
and punctuation with every word replaced, summarizer instructions and
descriptions become filler.

--order names the definition file stems in clone order a..e (default: oldest
created first). It is passed on the command line so no private name is written
into this script.

Nothing is written unless the output passes BOTH leak checks: the closed world
(every token is Otto vocabulary, filler, or a placeholder pattern; the same
check scripts/test_scrub_workflow_export.py runs on the committed fixtures in
CI) and the source diff (no prose token, path, email, URL, ticket key, ULID,
Slack id or repository name of the private source appears). A failure prints
only where, never the text.
"""

from __future__ import annotations

import argparse
import csv
import json
import os
import re
import sys
from datetime import datetime, timedelta, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BASE_TIME = datetime(2026, 1, 1, tzinfo=timezone.utc)
LETTERS = "abcde"

# Neutral filler: every replaced word comes from here.
FILLER = """
lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor
incididunt ut labore et dolore magna aliqua enim ad minim veniam quis nostrud
exercitation ullamco laboris nisi aliquip ex ea commodo consequat duis aute
irure in reprehenderit voluptate velit esse cillum fugiat nulla pariatur
excepteur sint occaecat cupidatat non proident sunt culpa qui officia deserunt
mollit anim id est laborum curabitur pretium tincidunt lacus nunc vel risus
viverra maecenas accumsan lacinia purus faucibus scelerisque eleifend
donec vitae sapien pellentesque habitant morbi tristique senectus netus
malesuada fames ac turpis egestas integer feugiat vivamus arcu felis bibendum
ultrices gravida dictum fusce placerat orci eu volutpat odio facilisis mauris
quam suspendisse potenti nullam porttitor massa justo tellus
""".split()

# File names and patterns a stored prompt may reference (G4: renaming one
# fails CI). jira-KEY.md, stepN, stepN-*.md and stepN-*.output.json are
# normalized forms; MCP tool names are read from the repository.
CONTRACT_FILES = [
    "run-brief.md", "repos.json", "prs.json", "instructions.md", "prompt.md",
    "previous-review.md", "final-output.md",
]
PROVIDERS = ["claude", "codex", "agy", "gemini", "grok", "opencode", "muse", "pi", "shell"]
REVIEW_MODES = ["fan_out", "orchestrator"]
KIND_WORD = {
    "manual_trigger": "Start", "prepare_context": "Prepare", "agent_prompt": "Agent step",
    "review_run": "Review", "condition": "Condition", "channel_notify": "Notify",
    "log": "Log", "transform": "Transform", "delay": "Delay", "loop": "Loop",
    "human_approval": "Approval", "http_request": "Request", "git_pr": "Pull request",
}
# The keys this script writes into traces.
TRACE_KEYS = [
    "workflow", "run", "status", "error_class", "resume_attempts", "duration_s", "input_keys",
    "nodes", "routing", "id", "kind", "skip", "attempts", "sessions", "step_no", "step_file",
    "output_keys", "reply", "condition", "review", "score", "passed", "blocking", "advisory",
    "total", "severity", "reviewers_by_provider", "reviewers_by_lens", "scope_ran",
    "summarizer_ran", "edge", "taken", "branch", "error", "other", "bool", "number",
    "string", "array", "object", "null", "auth", "fd", "review_incomplete", "checkout_unresolved",
    "canceled", "provider",
]
# Every param key the five definitions use (Otto's own names).
PARAM_KEYS = [
    "prompt", "provider", "model", "skills", "skill", "lenses", "require", "key",
    "threshold", "require_pass", "reviewers", "lens", "providers", "summarizer",
    "instructions", "scoring", "bug", "warn", "info", "timeout_s", "mode", "expr",
    "message", "channel", "chat_id",
]
# Prompt length buckets (< 1k, 1-4k, 4-8k, 8-16k) and the filler size written for each.
BUCKETS = [(1000, 600), (4000, 2500), (8000, 6000), (16000, 12000), (10**9, 20000)]
# Engine sources: their tokens are Otto's own words (the goldens carry engine
# text such as the step preamble).
VOCAB_SOURCES = [
    "crates/otto-workflows/src", "crates/otto-server/src/workflow_engine",
    "crates/otto-server/src/workflow_engine.rs", "crates/otto-core/src/workflows.rs",
    "crates/otto-orchestrator/src/e2e_stub.rs",
]
TOKEN_RE = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.\-]*")
# Placeholder shapes the fixtures may carry besides vocabulary words.
PLACEHOLDERS = [re.compile(p) for p in (
    r"wf-compat-[a-e]", r"ws-compat", r"user-compat", r"chat-compat", r"thread-compat",
    r"n\d+", r"e\d+", r"x\d+", r"step\d+(-[a-z0-9]+)*(\.md|\.output\.json)?",
    r"step\d+-\*(\.md|\.output\.json)?", r"missing-(skill|lens)-\d+",
    r"(run|session|review|repo|id)-\d+", r"\d+(\.\d+)*[a-zA-Z]{0,2}", r"202[67]-\d\d-\d\dT[0-9:.]+Z?",
    r"[0-9a-f]{64}", r"sha256",
)]
ERROR_CLASSES = [
    ("auth", ("login expired", "/login", "not logged in", "unauthorized")),
    ("fd", ("too many open files", "dup of fd", "os error 24")),
    ("review_incomplete", ("review incomplete",)),
    ("checkout_unresolved", ("unresolved", "no declared repos")),
    ("canceled", ("canceled", "cancelled")),
]
NOT_STR = "\x60'\""  # characters that end a file name inside prose


def bucket_size(n: int) -> int:
    return next(size for limit, size in BUCKETS if n < limit)


def filler(n_chars: int, seed: int = 0) -> str:
    words, total, i = [], 0, seed
    while total < n_chars:
        w = FILLER[i % len(FILLER)]
        words.append(w)
        total += len(w) + 1
        i += 7
    return " ".join(words)


def tokens(text: str) -> list[str]:
    return [t.rstrip(".-") for t in TOKEN_RE.findall(text) if t.rstrip(".-")]


def bundled(root: Path) -> tuple[set[str], set[str]]:
    """(skill names, lens names) of the bundled library in this repository."""
    assets = root / "crates/otto-skills/assets"
    skills = {p.parent.name for p in assets.glob("skills/*/*/SKILL.md")}
    skills |= {p.parent.name for p in assets.glob("product-skills/*/SKILL.md")}
    lenses = {p.parent.name for p in assets.glob("skills/review/*/SKILL.md")}
    return skills, lenses


def mcp_tools(root: Path) -> set[str]:
    found: set[str] = set()
    for d in ("crates/otto-mcp/src/outward", "crates/ottod/src"):
        for f in (root / d).rglob("*.rs"):
            found |= set(re.findall(r'"(otto[._][a-z_]+)"', f.read_text(errors="ignore")))
    return found


def vocabulary(root: Path) -> set[str]:
    """ALLOW: Otto's contract vocabulary, read from the repository."""
    skills, lenses = bundled(root)
    words = set(CONTRACT_FILES) | set(PROVIDERS) | set(REVIEW_MODES) | set(PARAM_KEYS)
    words |= skills | lenses | mcp_tools(root)
    words |= {"jira-KEY.md", "compat", "Compat", "References", "OTTO_FAKE", "script",
              "true", "false", "null", "tmp", "data"}
    words |= set(" ".join(KIND_WORD.values()).split()) | set(KIND_WORD) | set("ABCDE")
    words |= set(TRACE_KEYS)
    for src in VOCAB_SOURCES:
        p = root / src
        for f in ([p] if p.is_file() else sorted(p.rglob("*.rs"))):
            words |= set(tokens(f.read_text(errors="ignore")))
    return words


def closed_world_hits(value, allow: set[str], path: str = "$") -> list[str]:
    """Where a string in value holds a token outside ALLOW, FILLER and the placeholders."""
    hits: list[str] = []
    if isinstance(value, dict):
        for k, v in value.items():
            hits += closed_world_hits(k, allow, f"{path}.<key>")
            hits += closed_world_hits(v, allow, f"{path}.{k}")
    elif isinstance(value, list):
        for i, v in enumerate(value):
            hits += closed_world_hits(v, allow, f"{path}[{i}]")
    elif isinstance(value, str):
        fill = set(FILLER)
        for i, t in enumerate(tokens(value)):
            # a dotted token (output.result) passes when it, or each of its parts, does
            if not allowed_word(t, allow, fill) and \
                    not all(allowed_word(w, allow, fill) for w in t.split(".")):
                hits.append(f"{path} token #{i}")
    return hits


def allowed_word(t: str, allow: set[str], fill: set[str]) -> bool:
    return t in allow or t.lower() in fill or any(p.fullmatch(t) for p in PLACEHOLDERS)


def _strings(v):
    if isinstance(v, str):
        yield v
    elif isinstance(v, dict):
        for x in v.values():
            yield from _strings(x)
    elif isinstance(v, list):
        for x in v:
            yield from _strings(x)


def prose_fields(defs: list[dict], runs: list[dict]) -> list[str]:
    out: list[str] = []
    for d in defs:
        out += [d.get("name") or "", d.get("description") or "", d.get("instructions") or ""]
        for n in d["graph"]["nodes"]:
            out.append(n.get("name") or "")
            out += list(_strings(n.get("params") or {}))
    for r in runs:
        out += list(_strings(r.get("input") or {}))
        out.append(r.get("error") or "")
        for n in r.get("nodes") or []:
            out += [n.get("error") or ""] + list(n.get("logs") or [])
    return out


DENY_PATTERNS = [
    re.compile(p) for p in (
        r"(?:~|/Users|/home|/private|/var)/[^\s\"']+", r"[\w.+-]+@[\w-]+\.[\w.]+", r"https?://\S+",
        r"\b[\w-]+\.(?:com|io|net|org|local|dev)\b", r"\b[A-Z][A-Z0-9]+-\d+\b",
        r"\b[0-9A-HJKMNP-TV-Z]{26}\b", r"\b[UCW][A-Z0-9]{8,}\b",
    )
]


def deny_set(defs: list[dict], runs: list[dict], allow: set[str]) -> set[str]:
    """Leak check 2: what of the private source must never appear in the output."""
    deny: set[str] = set()
    fill = set(FILLER)
    for text in prose_fields(defs, runs):
        deny |= {t for t in tokens(text) if len(t) >= 4 and t not in allow and t.lower() not in fill}
        for p in DENY_PATTERNS:
            deny |= set(p.findall(text))
    for r in runs:
        wd = (r.get("input") or {}).get("working_directory")
        if isinstance(wd, str) and wd.strip():
            deny.add(Path(wd.strip()).name)
    return {d for d in deny if d and not all(allowed_word(w, allow, fill) for w in d.split("."))}


def source_diff_hits(text: str, deny: set[str]) -> list[int]:
    own = set(tokens(text))
    return sorted({text.find(d) for d in deny
                   if d in own or (not TOKEN_RE.fullmatch(d) and d in text)})


# ---------------------------------------------------------------------------
# Definitions
# ---------------------------------------------------------------------------

def contract_refs(prompt: str, tools: set[str]) -> list[str]:
    refs = [f for f in CONTRACT_FILES if f in prompt]
    if re.search(r"jira-[A-Za-z0-9<>{}_-]+\.md", prompt):
        refs.append("jira-KEY.md")
    refs += [f"step{n}" for n in sorted({int(m) for m in re.findall(r"\bstep(\d+)\b", prompt)})]
    md = re.findall(r"\bstep(\d+)-[^\s" + NOT_STR + r"]*?\.md\b", prompt)
    refs += [f"step{n}-*.md" for n in sorted({int(m) for m in md})]
    oj = re.findall(r"\bstep(\d+)-[^\s" + NOT_STR + r"]*?\.output\.json\b", prompt)
    refs += [f"step{n}-*.output.json" for n in sorted({int(m) for m in oj})]
    if "*.output.json" in prompt:
        refs.append("*.output.json")
    refs += sorted({t for t in re.findall(r"\b(otto[._][a-z_]+)\b", prompt) if t in tools})
    return list(dict.fromkeys(refs))


def scrub_message(msg: str, allow: set[str]) -> str:
    """Keep {placeholders} (Otto keys only), punctuation and emoji; replace every word."""
    out, i = [], 0

    def word(_m):
        nonlocal i
        i += 1
        return FILLER[(i * 7) % len(FILLER)]

    for part in re.split(r"(\{[^{}]*\})", msg):
        if part.startswith("{") and part.endswith("}"):
            inner = part[1:-1]
            out.append(part if inner in allow and re.fullmatch(r"[a-z_]+", inner) else "{x1}")
        else:
            out.append(re.sub(r"[A-Za-z0-9_]+", word, part))
    return "".join(out)


def scrub_expr(expr: str, allow: set[str]) -> str:
    def lit(m):
        t = m.group(0)
        return t if t in allow else "x1"
    expr = re.sub(r"\"[^\"]*\"|'[^']*'", "\"x1\"", expr)
    return re.sub(r"[A-Za-z_][A-Za-z0-9_]*", lit, expr)


class Scrubber:
    def __init__(self, root: Path):
        self.allow = vocabulary(root)
        self.skills, self.lenses = bundled(root)
        self.tools = mcp_tools(root)
        self.unknown_skills: dict[str, str] = {}
        self.unknown_lenses: dict[str, str] = {}
        self.buckets: dict[int, int] = {}

    def skill(self, name: str) -> str:
        if name in self.skills:
            return name
        return self.unknown_skills.setdefault(name, f"missing-skill-{len(self.unknown_skills) + 1}")

    def lens(self, name: str) -> str:
        if name in self.lenses:
            return name
        return self.unknown_lenses.setdefault(name, f"missing-lens-{len(self.unknown_lenses) + 1}")

    def reviewer(self, r: dict) -> dict:
        out = {}
        for k, v in r.items():
            if k == "lens" and isinstance(v, str):
                out[k] = self.lens(v)
            elif k == "providers" and isinstance(v, list):
                out[k] = [p for p in v if p in PROVIDERS]
            else:
                out[k] = v if isinstance(v, (bool, int, float)) or v is None else "x1"
        return out

    def params(self, p: dict) -> dict:
        out = {}
        for k, v in p.items():
            if isinstance(v, (bool, int, float)) or v is None:
                out[k] = v
            elif k == "prompt" and isinstance(v, str):
                size = bucket_size(len(v))
                self.buckets[size] = self.buckets.get(size, 0) + 1
                refs = contract_refs(v, self.tools)
                out[k] = ("[compat]\n" + filler(size) + "\nReferences: " + ", ".join(refs)
                          + "\nOTTO_FAKE: @script\n")
            elif k == "provider" and isinstance(v, str):
                out[k] = v if v in PROVIDERS else ""
            elif k == "skills" and isinstance(v, list):
                out[k] = [self.skill(s) for s in v]
            elif k == "lenses" and isinstance(v, list):
                out[k] = [self.lens(s) for s in v]
            elif k == "skill" and isinstance(v, str):
                out[k] = self.skill(v)
            elif k == "expr" and isinstance(v, str):
                out[k] = scrub_expr(v, self.allow)
            elif k == "message" and isinstance(v, str):
                out[k] = scrub_message(v, self.allow)
            elif k == "mode" and isinstance(v, str):
                out[k] = v if v in REVIEW_MODES else "x1"
            elif k == "reviewers" and isinstance(v, list):
                out[k] = [self.reviewer(r) for r in v]
            elif k == "summarizer" and isinstance(v, dict):
                out[k] = {sk: (sv if sk == "provider" and sv in PROVIDERS
                               else filler(bucket_size(len(sv))) if isinstance(sv, str) else sv)
                          for sk, sv in v.items()}
            elif isinstance(v, dict):
                out[k] = self.params(v)
            elif isinstance(v, list):
                out[k] = [x if isinstance(x, (bool, int, float)) else "x1" for x in v]
            else:
                out[k] = "x1"
        return out

    def definition(self, d: dict, letter: str) -> dict:
        created, updated = parse_time(d.get("created_at")), parse_time(d.get("updated_at"))
        nodes = []
        for n in d["graph"]["nodes"]:
            num = re.sub(r"\D", "", n["id"]) or "0"
            node = {
                "id": n["id"], "kind": n["kind"],
                "name": f"{KIND_WORD.get(n['kind'], 'Step')} {num}",
                "x": round((n.get("x") or 0) / 10) * 10, "y": round((n.get("y") or 0) / 10) * 10,
                "params": self.params(n.get("params") or {}),
            }
            if "retry" in n:
                node["retry"] = n["retry"]
            nodes.append(node)
        edges = [{k: (scrub_expr(v, self.allow) if k == "condition" and isinstance(v, str) else v)
                  for k, v in e.items()} for e in d["graph"]["edges"]]
        desc, instr = d.get("description") or "", d.get("instructions") or ""
        delta = updated - created if created and updated else timedelta()
        return {
            "id": f"wf-compat-{letter}", "workspace_id": "ws-compat",
            "name": f"Compat {letter.upper()}",
            "description": filler(bucket_size(len(desc))) if desc else "",
            "instructions": filler(bucket_size(len(instr))) if instr else "",
            "graph": {"nodes": nodes, "edges": edges},
            "created_by": "user-compat",
            "created_at": iso(BASE_TIME), "updated_at": iso(BASE_TIME + delta),
            "version": d.get("version"), "on_restart": d.get("on_restart"),
        }


def parse_time(s):
    return datetime.fromisoformat(s.replace("Z", "+00:00")) if s else None


def iso(t: datetime) -> str:
    return t.strftime("%Y-%m-%dT%H:%M:%SZ")


# ---------------------------------------------------------------------------
# Golden traces
# ---------------------------------------------------------------------------

def error_class(text: str | None) -> str | None:
    """Fixed classifier: the class of an error, never its text."""
    if not text:
        return None
    low = text.lower()
    for cls, needles in ERROR_CLASSES:
        if any(n in low for n in needles):
            return cls
    return "provider"


def run_error_class(run: dict) -> str | None:
    if run["status"] == "canceled":
        return "canceled"
    for n in run.get("nodes") or []:
        cls = error_class(n.get("error"))
        if cls:
            return cls
    return error_class(run.get("error")) if run["status"] == "error" else None


def type_name(v) -> str:
    return {bool: "bool", int: "number", float: "number", str: "string", list: "array",
            dict: "object", type(None): "null"}[type(v)]


def reply_bucket(v) -> str | None:
    if not isinstance(v, str):
        return None
    return next(f"{limit // 1000}k" for limit, _ in BUCKETS if len(v) < limit)


def slug(name: str) -> str:
    s = re.sub(r"[^a-z0-9]+", "-", name.lower()).strip("-")
    return s[:40].strip("-") or "step"


def review_numbers(out: dict, sessions: list[dict], scrub: Scrubber) -> dict:
    lenses: dict[str, int] = {}
    providers: dict[str, int] = {}
    for s in sessions:
        if s["source"] != "review" or s["title"] == "Scope assessment":
            continue
        lens = slug(s["title"].split(" · ")[0])
        lens = scrub.lens(lens) if lens.endswith("-review") else "other"
        lenses[lens] = lenses.get(lens, 0) + 1
        if s["provider"] in PROVIDERS:
            providers[s["provider"]] = providers.get(s["provider"], 0) + 1
    sev = out.get("severity") or {}
    return {
        "score": out.get("score"), "passed": out.get("passed"),
        "blocking": out.get("blocking"), "advisory": out.get("advisory"), "total": out.get("total"),
        "severity": {k: sev.get(k) for k in ("bug", "warn", "info")},
        "reviewers_by_provider": dict(sorted(providers.items())),
        "reviewers_by_lens": dict(sorted(lenses.items())),
        "scope_ran": any(s["title"] == "Scope assessment" for s in sessions),
        "summarizer_ran": any(s["source"] == "review_summarizer" for s in sessions),
    }


def trace(run: dict, clone: dict, src: dict, sessions: list[dict], scrub: Scrubber) -> dict:
    by_id = {n["id"]: n for n in clone["graph"]["nodes"]}
    src_name = {n["id"]: (n.get("name") or "") for n in src["graph"]["nodes"]}
    started = sorted((n for n in run["nodes"] if n.get("started_at")), key=lambda n: n["started_at"])
    step_no = {n["node_id"]: i + 1 for i, n in enumerate(started)}
    state = {n["node_id"]: n for n in run["nodes"]}
    branch_skipped = {nid for nid, n in state.items() if n["status"] == "skipped"
                      and any("branch not taken" in l for l in n.get("logs") or [])}
    nodes = []
    for n in run["nodes"]:
        node = by_id.get(n["node_id"])
        if node is None:
            continue
        out = n.get("output") if isinstance(n.get("output"), dict) else {}
        no = step_no.get(n["node_id"])
        entry = {
            "id": n["node_id"], "kind": node["kind"], "status": n["status"],
            "skip": None if n["status"] != "skipped" else
                    "branch" if n["node_id"] in branch_skipped else "error",
            "attempts": n.get("attempts"), "sessions": len(n.get("sessions") or []),
            "duration_s": None if n.get("duration_ms") is None else round(n["duration_ms"] / 1000),
            "step_no": no,
            "step_file": f"step{no}-{slug(node['name'])}.md" if no else None,
            "output_keys": sorted(out.keys()),
            "error_class": error_class(n.get("error")),
            "reply": reply_bucket(out.get("reply")),
        }
        if node["kind"] == "condition" and "result" in out:
            entry["condition"] = out["result"]
        if node["kind"] == "review_run" and "score" in out:
            mine = [s for s in sessions if s["step"] == src_name[n["node_id"]]]
            entry["review"] = review_numbers(out, mine, scrub)
        nodes.append(entry)
    routing = [{"edge": e["id"],
                "taken": (state.get(e["source"]) or {}).get("status") == "success"
                and e["target"] not in branch_skipped}
               for e in clone["graph"]["edges"] if e.get("condition")]
    a, b = parse_time(run.get("started_at")), parse_time(run.get("finished_at"))
    return {
        "workflow": clone["id"],
        "run": {"status": run["status"], "error_class": run_error_class(run),
                "resume_attempts": run.get("resume_attempts") or 0,
                "duration_s": round((b - a).total_seconds()) if a and b else None},
        "input_keys": {k: type_name(v) for k, v in sorted((run.get("input") or {}).items())},
        "nodes": nodes, "routing": routing,
    }


def pick_scenarios(runs: list[dict], limit: int) -> dict[str, dict]:
    """Median and P90 success, one FAILED verdict, one run per failure class, one resumed run."""
    def dur(r):
        a, b = parse_time(r.get("started_at")), parse_time(r.get("finished_at"))
        return (b - a).total_seconds() if a and b else 0.0

    def verdict(r):
        for n in r["nodes"]:
            o = n.get("output")
            if isinstance(o, dict) and "passed" in o and "score" in o:
                return o["passed"]
        return None

    ok = sorted((r for r in runs if r["status"] == "success" and not r.get("resume_attempts")),
                key=lambda r: (dur(r), r["id"]))
    picked: dict[str, dict] = {}
    if ok:
        picked["success-median"] = ok[len(ok) // 2]
        picked["success-p90"] = ok[min(len(ok) - 1, int(len(ok) * 0.9))]
    failed = [r for r in ok if verdict(r) is False]
    if failed:
        picked["verdict-failed"] = failed[len(failed) // 2]
    for r in sorted(runs, key=lambda r: r["id"]):
        cls = run_error_class(r)
        if cls and f"error-{cls}" not in picked:
            picked[f"error-{cls}"] = r
        if r.get("resume_attempts") and "resumed" not in picked:
            picked["resumed"] = r
    return dict(list(picked.items())[:limit])


# ---------------------------------------------------------------------------

def scrub_export(defs_dir: Path, runs_dir: Path, order: list[str], root: Path, limit: int):
    """(files to write, leak-check failures)."""
    defs = {p.stem: json.loads(p.read_text()) for p in sorted(defs_dir.glob("*.json"))}
    order = order or sorted(defs, key=lambda s: (defs[s].get("created_at") or "", s))
    if sorted(order) != sorted(defs) or len(order) > len(LETTERS):
        raise SystemExit("--order must name every definition stem once (at most five)")
    sessions_csv = runs_dir / "sessions.csv"
    sessions = list(csv.DictReader(sessions_csv.open())) if sessions_csv.exists() else []
    scrub = Scrubber(root)
    files: dict[str, object] = {}
    all_runs: list[dict] = []
    for letter, stem in zip(LETTERS, order):
        src = defs[stem]
        clone = scrub.definition(src, letter)
        files[f"clones/{letter}.json"] = clone
        runs = [json.loads(p.read_text()) for p in sorted((runs_dir / "runs" / stem).glob("*.json"))]
        runs = [r for r in runs if r.get("workflow_id") == src.get("id")]
        all_runs += runs
        for name, run in pick_scenarios(runs, limit).items():
            mine = [s for s in sessions if s.get("run_id") == run["id"]]
            files[f"traces/{letter}/{name}.json"] = trace(run, clone, src, mine, scrub)
    deny = deny_set(list(defs.values()), all_runs, scrub.allow)
    failures = []
    for path, value in files.items():
        text = json.dumps(value, indent=2, ensure_ascii=False)
        failures += [f"{path}: closed world, {h}" for h in closed_world_hits(value, scrub.allow)]
        failures += [f"{path}: private token at offset {h}" for h in source_diff_hits(text, deny)]
    return files, failures, scrub, len(deny)


def main(argv: list[str] | None = None, root: Path = ROOT) -> int:
    ap = argparse.ArgumentParser(description="Scrub a private workflow export into compat fixtures.")
    ap.add_argument("defs_dir", type=Path)
    ap.add_argument("runs_dir", type=Path)
    ap.add_argument("out_dir", type=Path)
    ap.add_argument("--order", default="")
    ap.add_argument("--traces", type=int, default=12)
    a = ap.parse_args(argv)
    defs_dir, runs_dir, out = a.defs_dir.resolve(), a.runs_dir.resolve(), a.out_dir.resolve()
    private = Path(os.path.commonpath([defs_dir, runs_dir]))
    if out == private or private in out.parents:
        print("refusing to write inside the private export", file=sys.stderr)
        return 2
    if out.exists():
        print(f"refusing to overwrite {out} (remove it first)", file=sys.stderr)
        return 2
    files, failures, scrub, deny = scrub_export(
        defs_dir, runs_dir, [s for s in a.order.split(",") if s], root, a.traces)
    if failures:
        print("leak check failed, nothing written:", file=sys.stderr)
        for f in failures[:50]:
            print("  " + f, file=sys.stderr)
        return 1
    for path, value in files.items():
        p = out / path
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n")
    clones = sum(1 for p in files if p.startswith("clones/"))
    print(f"wrote {clones} clones and {len(files) - clones} traces")
    print(f"prompts per filler size: {dict(sorted(scrub.buckets.items()))}")
    print(f"unknown skills replaced: {len(scrub.unknown_skills)}; "
          f"unknown lenses replaced: {len(scrub.unknown_lenses)}; deny-set size: {deny}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
