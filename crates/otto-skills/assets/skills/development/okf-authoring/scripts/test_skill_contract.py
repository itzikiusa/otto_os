import json
import unittest
from pathlib import Path


SKILL_ROOT = Path(__file__).resolve().parent.parent


class SkillContractTests(unittest.TestCase):
    def test_skill_contains_hard_static_completion_gate(self):
        skill = (SKILL_ROOT / "SKILL.md").read_text(encoding="utf-8")

        self.assertIn("## Hard static enforcement", skill)
        self.assertIn("scripts/check_vault.py", skill)
        self.assertIn("validate_okf.py ROOT --strict", skill)
        self.assertIn("Do not claim produce or maintain work complete", skill)

    def test_linking_reference_forbids_non_graph_targets(self):
        linking = (SKILL_ROOT / "references" / "linking-indexes-logs.md").read_text(
            encoding="utf-8"
        )

        self.assertIn("Never link to a directory", linking)
        self.assertIn("Never use `file://`", linking)

    def test_evals_cover_static_link_and_completion_regressions(self):
        evaluation = json.loads(
            (SKILL_ROOT / "evals" / "evals.json").read_text(encoding="utf-8")
        )
        cases = {case["id"]: case for case in evaluation["cases"]}

        self.assertIn("static-link-enforcement", cases)
        self.assertIn("strict-completion-gate", cases)
        self.assertEqual(
            cases["static-link-enforcement"]["fixture"],
            "fixtures/static-link-violations",
        )
        self.assertTrue(
            cases["strict-completion-gate"]["expect"]["must_refuse_completion"]
        )


if __name__ == "__main__":
    unittest.main()
