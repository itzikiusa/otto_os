import hashlib
import io
import json
import unittest
from contextlib import redirect_stdout
from pathlib import Path

import audit_bundle


SKILL_ROOT = Path(__file__).resolve().parent.parent
FIXTURES = SKILL_ROOT / "evals" / "fixtures"
def tree_digest(root: Path) -> str:
    digest = hashlib.sha256()
    for path in sorted(p for p in root.rglob("*") if p.is_file()):
        digest.update(path.relative_to(root).as_posix().encode())
        digest.update(path.read_bytes())
    return digest.hexdigest()


class AuditBundleTests(unittest.TestCase):
    def test_missing_endpoint_sections_are_reported(self):
        findings = audit_bundle.audit_bundle(FIXTURES / "missing-endpoint-sections")

        self.assertEqual(
            {item["rule"] for item in findings},
            {"Q_API_REQUEST", "Q_API_SUCCESS", "Q_API_ERRORS"},
        )
        self.assertTrue(all(item["path"] == "endpoints/create-widget.md" for item in findings))
        self.assertTrue(all(item["severity"] == "warning" for item in findings))

    def test_shallow_data_asset_reports_each_required_depth_area(self):
        findings = audit_bundle.audit_bundle(FIXTURES / "shallow-data-asset")

        self.assertEqual(
            {item["rule"] for item in findings},
            {
                "Q_DATA_FIELDS",
                "Q_DATA_ACCESS",
                "Q_DATA_INDEX_TTL",
                "Q_DATA_TRANSACTIONS",
                "Q_DATA_IMPACT",
                "Q_CITATIONS",
            },
        )

    def test_clean_bundle_has_no_findings(self):
        self.assertEqual(audit_bundle.audit_bundle(FIXTURES / "clean-bundle"), [])

    def test_json_cli_emits_finding_objects_and_quality_findings_do_not_fail(self):
        stdout = io.StringIO()
        with redirect_stdout(stdout):
            exit_code = audit_bundle.main(
                [str(FIXTURES / "shallow-data-asset"), "--format", "json"]
            )

        self.assertEqual(exit_code, 0)
        findings = json.loads(stdout.getvalue())
        self.assertTrue(findings)
        self.assertEqual(set(findings[0]), {"rule", "path", "message", "severity"})

    def test_conformance_errors_make_audit_cli_fail(self):
        stdout = io.StringIO()
        with redirect_stdout(stdout):
            exit_code = audit_bundle.main(
                [str(FIXTURES / "invalid-concepts"), "--format", "text"]
            )

        self.assertEqual(exit_code, 1)
        self.assertIn("ERROR E1", stdout.getvalue())

    def test_audit_never_mutates_the_bundle(self):
        root = FIXTURES / "clean-bundle"
        before = tree_digest(root)

        audit_bundle.audit_bundle(root)

        self.assertEqual(tree_digest(root), before)


if __name__ == "__main__":
    unittest.main()
