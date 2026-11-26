"""Closed-world vocabulary test for the workflow compat fixtures (leak check 1).

Every string in crates/otto-server/tests/fixtures/workflows/compat/ (clones,
traces, goldens, chat transcripts) must be made of Otto's own vocabulary, the
scrubber's filler words, or placeholder shapes. It needs no private input, so
it runs on every push and catches private text in a fixture whoever wrote it.
The rest checks the scrubber itself on a synthetic export with planted
private words.
"""

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FIXTURES = ROOT / "crates/otto-server/tests/fixtures/workflows/compat"
spec = importlib.util.spec_from_file_location("scrub", Path(__file__).with_name("scrub-workflow-export.py"))
scrub = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scrub)

PLANTED = ["zebrafinch", "QUARKLY-4711", "/Users/somebody/acme-repo", "acme-repo", "Wolfgang",
           "U0123ABCDXY", "https://intranet.example.com/x", "01M8744R6JHAB0SKBCRB2WCH9M"]


def synthetic_export(root: Path, extra_param=None) -> tuple[Path, Path]:
    defs, runs = root / "in" / "defs", root / "in" / "runs"
    (runs / "runs" / "one").mkdir(parents=True)
    defs.mkdir(parents=True)
    params = {"provider": "claude", "skills": ["okf-authoring", "zebrafinch-skill"],
              "prompt": "Wolfgang: read run-brief.md and step2-zebrafinch.md, then call otto_get_pr "
                        "for QUARKLY-4711 in /Users/somebody/acme-repo " + "x" * 1200}
    if extra_param:
        params.update(extra_param)
    wf = {
        "id": "01M8744R6JHAB0SKBCRB2WCH9M", "workspace_id": "zebrafinch-ws", "name": "Zebrafinch flow",
        "description": "Wolfgang owns acme-repo", "instructions": "",
        "graph": {"nodes": [
            {"id": "n1", "kind": "manual_trigger", "name": "Kickoff Wolfgang", "x": 41.0, "y": 29.0, "params": {}},
            {"id": "n2", "kind": "agent_prompt", "name": "Zebrafinch step", "x": 300, "y": 30, "params": params},
            {"id": "n3", "kind": "condition", "name": "Gate", "x": 0, "y": 0, "params": {"expr": "passed == \"zebrafinch\""}},
            {"id": "n4", "kind": "channel_notify", "name": "Tell", "x": 0, "y": 0,
             "params": {"message": "✅ Wolfgang — {jira_ticket} · {zebrafinch} (score {score})"}},
            {"id": "n5", "kind": "review_run", "name": "Rev", "x": 0, "y": 0, "params": {
                "threshold": 80, "require_pass": False, "timeout_s": 7200, "mode": "orchestrator",
                "scoring": {"bug": 25, "warn": 3, "info": 0},
                "reviewers": [{"lens": "correctness-review", "providers": ["claude", "codex"]},
                              {"lens": "zebrafinch-review", "providers": ["claude"]}],
                "summarizer": {"provider": "claude", "instructions": "Wolfgang says hi"}}},
        ], "edges": [{"id": "e1", "source": "n1", "target": "n2"},
                     {"id": "e2", "source": "n3", "target": "n4", "condition": "output.result == true"}]},
        "created_by": "wolfgang", "created_at": "2026-09-01T10:00:00Z", "updated_at": "2026-09-02T10:00:00Z",
        "version": 7, "on_restart": "resume",
    }
    (defs / "one.json").write_text(json.dumps(wf))
    run = {"id": "01M8744R6JHAB0SKBCRB2WCH9N", "workflow_id": wf["id"], "status": "error",
           "started_at": "2026-09-03T10:00:00Z", "finished_at": "2026-09-03T10:05:00Z",
           "input": {"working_directory": "/Users/somebody/acme-repo", "user": "U0123ABCDXY",
                     "jira_ticket": "QUARKLY-4711"},
           "error": "one or more nodes failed",
           "nodes": [{"node_id": "n1", "status": "success", "started_at": "2026-09-03T10:00:01Z",
                      "duration_ms": 10, "attempts": 1, "output": {"working_directory": "/Users/somebody/acme-repo"}},
                     {"node_id": "n2", "status": "error", "started_at": "2026-09-03T10:00:02Z",
                      "duration_ms": 9000, "attempts": 3, "sessions": ["s1", "s2", "s3"],
                      "error": "upstream: agent error: Login expired · Wolfgang please run /login"}]}
    (runs / "runs" / "one" / "r.json").write_text(json.dumps(run))
    return defs, runs


class ClosedWorld(unittest.TestCase):
    def test_committed_fixtures_are_closed_world(self):
        allow = scrub.vocabulary(ROOT)
        files = sorted(p for p in FIXTURES.rglob("*") if p.is_file())
        self.assertGreaterEqual(len([p for p in files if p.parent.name == "clones"]), 5)
        for p in files:
            text = p.read_text()
            value = json.loads(text) if p.suffix == ".json" else text
            self.assertEqual(scrub.closed_world_hits(value, allow), [], f"{p.relative_to(ROOT)}")

    def test_a_private_word_in_a_fixture_is_caught(self):
        allow = scrub.vocabulary(ROOT)
        self.assertEqual(scrub.closed_world_hits({"name": "Agent step 3", "x": "lorem ipsum"}, allow), [])
        self.assertTrue(scrub.closed_world_hits({"name": "Agent step zebrafinch"}, allow))
        self.assertTrue(scrub.closed_world_hits(["QUARKLY-4711"], allow))


class Scrubber(unittest.TestCase):
    def run_scrub(self, extra_param=None):
        tmp = Path(tempfile.mkdtemp())
        defs, runs = synthetic_export(tmp, extra_param)
        out = tmp / "out"
        code = scrub.main([str(defs), str(runs), str(out)], root=ROOT)
        return code, out, defs

    def test_scrub_keeps_structure_and_drops_every_private_word(self):
        code, out, defs = self.run_scrub()
        self.assertEqual(code, 0)
        src = json.loads((defs / "one.json").read_text())
        clone = json.loads((out / "clones" / "a.json").read_text())
        text = "\n".join(p.read_text() for p in out.rglob("*.json"))
        for word in PLANTED:
            self.assertNotIn(word, text)
        self.assertEqual(clone["graph"]["edges"], src["graph"]["edges"])
        for s, c in zip(src["graph"]["nodes"], clone["graph"]["nodes"]):
            self.assertEqual((s["id"], s["kind"], sorted(s["params"])), (c["id"], c["kind"], sorted(c["params"])))
        p = {n["id"]: n for n in clone["graph"]["nodes"]}
        self.assertEqual(p["n1"]["name"], "Start 1")
        self.assertEqual((p["n1"]["x"], p["n1"]["y"]), (40, 30))
        self.assertEqual(p["n2"]["params"]["skills"], ["okf-authoring", "missing-skill-1"])
        prompt = p["n2"]["params"]["prompt"]
        self.assertTrue(prompt.startswith("[compat]\n"))
        self.assertIn("References: run-brief.md, step2, step2-*.md, otto_get_pr\nOTTO_FAKE: @script", prompt)
        self.assertEqual(scrub.bucket_size(len(src["graph"]["nodes"][1]["params"]["prompt"])), 2500)
        self.assertEqual(p["n3"]["params"]["expr"], "passed == \"x1\"")
        self.assertRegex(p["n4"]["params"]["message"], r"^✅ \w+ — \{jira_ticket\} · \{x1\} \(\w+ \{score\}\)$")
        review = p["n5"]["params"]
        self.assertEqual({k: review[k] for k in ("threshold", "require_pass", "timeout_s", "mode", "scoring")},
                         {"threshold": 80, "require_pass": False, "timeout_s": 7200, "mode": "orchestrator",
                          "scoring": {"bug": 25, "warn": 3, "info": 0}})
        self.assertEqual([r["lens"] for r in review["reviewers"]], ["correctness-review", "missing-lens-1"])
        self.assertEqual((clone["id"], clone["name"], clone["version"]), ("wf-compat-a", "Compat A", 7))
        self.assertEqual(clone["updated_at"], "2026-01-02T00:00:00Z")
        trace = json.loads((out / "traces" / "a" / "error-auth.json").read_text())
        self.assertEqual(trace["run"]["error_class"], "auth")
        n2 = trace["nodes"][1]
        self.assertEqual((n2["attempts"], n2["sessions"], n2["duration_s"], n2["step_file"]),
                         (3, 3, 9, "step2-agent-step-2.md"))
        self.assertEqual(trace["input_keys"], {"jira_ticket": "string", "user": "string",
                                               "working_directory": "string"})

    def test_a_private_param_key_aborts_the_write(self):
        code, out, _ = self.run_scrub({"zebrafinch_key": 1})
        self.assertEqual(code, 1)
        self.assertFalse(out.exists())

    def test_refuses_an_existing_or_private_out_dir(self):
        tmp = Path(tempfile.mkdtemp())
        defs, runs = synthetic_export(tmp)
        self.assertEqual(scrub.main([str(defs), str(runs), str(defs / "out")], root=ROOT), 2)
        self.assertEqual(scrub.main([str(defs), str(runs), str(tmp / "in" / "out")], root=ROOT), 2)
        (tmp / "taken").mkdir()
        self.assertEqual(scrub.main([str(defs), str(runs), str(tmp / "taken")], root=ROOT), 2)

    def test_error_classes_are_fixed_words(self):
        self.assertEqual(scrub.error_class("internal: spawn claude: Too many open files (os error 24)"), "fd")
        self.assertEqual(scrub.error_class("review_run: review incomplete — not a verdict"), "review_incomplete")
        self.assertEqual(scrub.error_class("invalid: review_run: no declared repos[] entry resolved"),
                         "checkout_unresolved")
        self.assertEqual(scrub.error_class("something else entirely"), "provider")
        self.assertIsNone(scrub.error_class(None))


if __name__ == "__main__":
    unittest.main()
