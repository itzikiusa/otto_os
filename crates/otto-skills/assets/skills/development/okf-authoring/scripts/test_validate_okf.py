import hashlib
import io
import json
import unittest
from contextlib import redirect_stdout
from pathlib import Path

import validate_okf


SKILL_ROOT = Path(__file__).resolve().parent.parent
FIXTURES = SKILL_ROOT / "evals" / "fixtures"
def tree_digest(root: Path) -> str:
    digest = hashlib.sha256()
    for path in sorted(p for p in root.rglob("*") if p.is_file()):
        digest.update(path.relative_to(root).as_posix().encode())
        digest.update(path.read_bytes())
    return digest.hexdigest()


class ValidateOkfTests(unittest.TestCase):
    def test_missing_frontmatter_and_type_are_conformance_errors(self):
        report = validate_okf.validate_bundle(FIXTURES / "invalid-concepts")

        self.assertFalse(report["conformant"])
        self.assertEqual(
            [(item["rule"], item["path"]) for item in report["errors"]],
            [("E1", "missing-frontmatter.md"), ("E2", "missing-type.md")],
        )

    def test_reserved_file_violations_are_conformance_errors(self):
        report = validate_okf.validate_bundle(FIXTURES / "reserved-violations")

        self.assertFalse(report["conformant"])
        self.assertEqual(
            [(item["rule"], item["path"]) for item in report["errors"]],
            [("E3", "docs/index.md"), ("E3", "log.md")],
        )

    def test_type_matches_otto_string_or_number_semantics(self):
        report = validate_okf.validate_bundle(FIXTURES / "invalid-type-values")

        self.assertFalse(report["conformant"])
        self.assertEqual(
            [(item["rule"], item["path"]) for item in report["errors"]],
            [
                ("E2", "boolean-type.md"),
                ("E2", "comment-only-type.md"),
                ("E2", "mapping-type.md"),
                ("E2", "sequence-type.md"),
            ],
        )

    def test_quality_warnings_do_not_make_cli_fail(self):
        stdout = io.StringIO()
        with redirect_stdout(stdout):
            exit_code = validate_okf.main(
                [str(FIXTURES / "warnings-only"), "--format", "json"]
            )

        self.assertEqual(exit_code, 0)
        report = json.loads(stdout.getvalue())
        self.assertTrue(report["conformant"])
        self.assertIn("W1", {item["rule"] for item in report["warnings"]})

    def test_clean_bundle_is_conformant_and_has_no_warnings(self):
        report = validate_okf.validate_bundle(FIXTURES / "clean-bundle")

        self.assertEqual(
            report,
            {"conformant": True, "errors": [], "warnings": [], "checked_notes": 6},
        )

    def test_validation_never_mutates_the_bundle(self):
        root = FIXTURES / "clean-bundle"
        before = tree_digest(root)

        validate_okf.validate_bundle(root)

        self.assertEqual(tree_digest(root), before)


if __name__ == "__main__":
    unittest.main()
