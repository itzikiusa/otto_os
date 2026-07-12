import tempfile
import unittest
from pathlib import Path

import validate_reviewer_evals


class ContractTests(unittest.TestCase):
    def test_accepts_exact_source_and_doc_evidence(self):
        finding = {
            "severity": "major", "category": "api", "summary": "Missing body",
            "evidence": [{"repo_path": "src/api.rs", "line": 4}, {"doc_path": "api.md", "section": "POST /x"}],
            "missed_item": "response body", "required_fix": "document it",
        }
        self.assertEqual([], validate_reviewer_evals.validate_finding(finding, "case"))

    def test_rejects_drifted_contract(self):
        finding = {"severity": "blocker", "category": "api", "summary": "x", "doc": "api.md", "source": "src/api.rs:4", "evidence": "x", "repair": "x"}
        self.assertTrue(validate_reviewer_evals.validate_finding(finding, "case"))

    def test_rejects_unknown_category_empty_paths_and_boolean_line(self):
        base = {
            "severity": "major", "category": "banana", "summary": "x",
            "evidence": [{"repo_path": "", "line": True}, {"doc_path": " ", "section": "S"}],
            "missed_item": "x", "required_fix": "x",
        }
        errors = validate_reviewer_evals.validate_finding(base, "case")
        self.assertTrue(any("category" in error for error in errors))
        self.assertTrue(any("evidence[0]" in error for error in errors))
        self.assertTrue(any("evidence[1]" in error for error in errors))

    def test_rejects_zero_line_extra_keys_and_empty_text(self):
        finding = {
            "severity": "major", "category": "api", "summary": " ",
            "evidence": [{"repo_path": "src/api.rs", "line": 0, "extra": True}],
            "missed_item": "x", "required_fix": "x",
        }
        errors = validate_reviewer_evals.validate_finding(finding, "case")
        self.assertTrue(any("textual" in error for error in errors))
        self.assertTrue(any("evidence[0]" in error for error in errors))


if __name__ == "__main__":
    unittest.main()
