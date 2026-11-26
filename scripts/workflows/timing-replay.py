#!/usr/bin/env python3
"""Timing replay (test plan §4.1): the old engine's own waits, and the full local replay.

The CI replay (crates/otto-server/tests/wf_timing.rs) runs each clone's committed
median trace through the new engine on the in-memory fakes. A trace holds each
step's measured duration; what the old engine added on top of the work (the
admission gate's 300 s holds and the 20 s idle confirm, runs.md §3–§4) comes from
the private export and is committed as numbers only, in compat/timing.json.

  waits DEFS RUNS            rewrite compat/timing.json for the committed traces
  full  DEFS RUNS OUT        scrub every successful run into OUT (outside the
                             repository) and replay them all; report in OUT

DEFS / RUNS are the export's workflows-export and workflow-runs-export dirs.
"""
import argparse
import csv
import importlib.util
import json
import os
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
COMPAT = ROOT / "crates/otto-server/tests/fixtures/workflows/compat"
_spec = importlib.util.spec_from_file_location("scrub", ROOT / "scripts/scrub-workflow-export.py")
scrub = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(scrub)

# runs.md §0: the permanent 300 s gate wait starts on 11-17 between 09:27 and 11:06 UTC.
ERA_B = datetime(2026, 11, 17, 10, 0, tzinfo=timezone.utc)
GATE_S = 300  # admission.rs DEFAULT_MAX_WAIT: every later gated wave in era B
GATED = 200   # a spawn this late after the step start waited for the gate


def spawned(ulid: str) -> datetime:
    """A session id is a ULID: its first 10 characters are the spawn time in ms (runs.md §0)."""
    ms = 0
    for ch in ulid[:10].upper():
        ms = ms * 32 + "0123456789ABCDEFGHJKMNPQRSTVWXYZ".index(ch)
    return datetime.fromtimestamp(ms / 1000, tz=timezone.utc)


def old_waits(run: dict, clone: dict, src: dict, sessions: list[dict]) -> dict:
    """{era, nodes: {id: [wait_s, idle_s]}} — what the old engine spent besides the work."""
    kind = {n["id"]: n["kind"] for n in clone["graph"]["nodes"]}
    name = {n["id"]: n.get("name") or "" for n in src["graph"]["nodes"]}
    era_b = scrub.parse_time(run["started_at"]) >= ERA_B
    out = {}
    for n in run["nodes"]:
        k = kind.get(n["node_id"])
        if k not in ("agent_prompt", "review_run") or not n.get("started_at"):
            continue
        mine = sorted(spawned(s["session_id"]) for s in sessions if s["step"] == name[n["node_id"]])
        first = (mine[0] - scrub.parse_time(n["started_at"])).total_seconds() if mine else 0
        wait = round(first) if first > GATED else 0
        if k == "review_run":
            scope = any(s["step"] == name[n["node_id"]] and s["title"] == "Scope assessment" for s in sessions)
            wait += GATE_S * (2 if scope else 1) if era_b else 0
            idle = 0
        else:
            idle = 20 if any("confirming completion" in l for l in n.get("logs") or []) else 0
        out[n["node_id"]] = [wait, idle]
    return {"era": "B" if era_b else "A", "nodes": out}


def load(defs_dir: Path, runs_dir: Path):
    defs = {p.stem: json.loads(p.read_text()) for p in sorted(defs_dir.glob("*.json"))}
    order = sorted(defs, key=lambda s: (defs[s].get("created_at") or "", s))
    sessions = list(csv.DictReader((runs_dir / "sessions.csv").open()))
    sc = scrub.Scrubber(ROOT)
    for letter, stem in zip(scrub.LETTERS, order):
        src = defs[stem]
        runs = [json.loads(p.read_text()) for p in sorted((runs_dir / "runs" / stem).glob("*.json"))]
        runs = [r for r in runs if r.get("workflow_id") == src.get("id")]
        yield letter, src, sc.definition(src, letter), runs, sessions, sc


def shape(trace: dict):
    return trace["run"]["duration_s"], [(n["id"], n["status"], n["duration_s"]) for n in trace["nodes"]]


def cmd_waits(a) -> int:
    timing = {}
    stems = list(load(a.defs, a.runs))
    for letter in scrub.LETTERS[:len(stems)]:
        clone = json.loads((COMPAT / "clones" / f"{letter}.json").read_text())
        # The export grows after a fixture is cut: find each trace's run in any definition.
        runs = [(src, r, s, sc) for _, src, _, rs, s, sc in stems for r in rs
                if {n["id"] for n in src["graph"]["nodes"]} == {n["id"] for n in clone["graph"]["nodes"]}]
        for path in sorted((COMPAT / "traces" / letter).glob("*.json")):
            want = shape(json.loads(path.read_text()))
            for src, run, sessions, sc in runs:
                mine = [s for s in sessions if s.get("run_id") == run["id"]]
                if shape(scrub.trace(run, clone, src, mine, sc)) == want:
                    timing[f"{letter}/{path.stem}"] = old_waits(run, clone, src, mine)
                    break
            else:
                print(f"{letter}/{path.stem}: no export run matches", file=sys.stderr)
    (COMPAT / "timing.json").write_text(json.dumps(timing, indent=1, sort_keys=True) + "\n")
    print(f"wrote {len(timing)} traces' waits to {COMPAT / 'timing.json'}")
    return 0


def cmd_full(a) -> int:
    out: Path = a.out.resolve()
    if ROOT in out.parents or out == ROOT:
        raise SystemExit("the full replay writes outside the repository")
    timing = {}
    for letter, src, clone, runs, sessions, sc in load(a.defs, a.runs):
        (out / "clones").mkdir(parents=True, exist_ok=True)
        (out / "clones" / f"{letter}.json").write_text(json.dumps(clone, indent=1))
        (out / "traces" / letter).mkdir(parents=True, exist_ok=True)
        ok = [r for r in runs if r["status"] == "success" and not r.get("resume_attempts")]
        for i, run in enumerate(sorted(ok, key=lambda r: r["id"])):
            mine = [s for s in sessions if s.get("run_id") == run["id"]]
            name = f"run-{i:03}"
            (out / "traces" / letter / f"{name}.json").write_text(
                json.dumps(scrub.trace(run, clone, src, mine, sc), indent=1))
            timing[f"{letter}/{name}"] = old_waits(run, clone, src, mine)
    (out / "timing.json").write_text(json.dumps(timing, indent=1, sort_keys=True))
    print(f"{len(timing)} runs scrubbed into {out}")
    env = dict(os.environ, CARGO_INCREMENTAL="0", OTTO_WF_TIMING_DIR=str(out),
               OTTO_WF_TIMING_REPORT=str(out / "report.json"))
    heavy = os.environ.get("HEAVY", "")
    cmd = ([heavy] if heavy else []) + ["cargo", "test", "-p", "otto-server", "--features",
           "otto-server/test-util", "--test", "it", "wf_timing::replay_every_trace", "--",
           "--ignored", "--nocapture", "--test-threads=1"]
    return subprocess.call(cmd, cwd=ROOT, env=env)


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    for name in ("waits", "full"):
        p = sub.add_parser(name)
        p.add_argument("defs", type=Path)
        p.add_argument("runs", type=Path)
        if name == "full":
            p.add_argument("out", type=Path)
    a = ap.parse_args(argv)
    return cmd_waits(a) if a.cmd == "waits" else cmd_full(a)


if __name__ == "__main__":
    sys.exit(main())
