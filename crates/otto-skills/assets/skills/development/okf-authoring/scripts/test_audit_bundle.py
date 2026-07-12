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
                "Q_DATA_EXAMPLES",
                "Q_CITATIONS",
            },
        )

    def test_syntax_tokens_do_not_satisfy_endpoint_depth(self):
        findings = audit_bundle.audit_bundle(FIXTURES / "adversarial-depth")

        endpoint_rules = {
            item["rule"]
            for item in findings
            if item["path"] == "endpoints/create-widget.md"
        }
        self.assertEqual(
            endpoint_rules,
            {
                "Q_API_AUTH",
                "Q_API_PARAMETERS",
                "Q_API_REQUEST",
                "Q_API_SUCCESS",
                "Q_API_ERRORS",
                "Q_API_VALIDATION",
                "Q_API_SIDE_EFFECTS",
                "Q_API_FLOW",
            },
        )

    def test_headers_and_keywords_do_not_satisfy_data_depth(self):
        findings = audit_bundle.audit_bundle(FIXTURES / "adversarial-depth")

        data_rules = {
            item["rule"]
            for item in findings
            if item["path"] == "datasets/orders.md"
        }
        self.assertEqual(
            data_rules,
            {
                "Q_DATA_GRAIN",
                "Q_DATA_FIELDS",
                "Q_DATA_ACCESS",
                "Q_DATA_INDEX_TTL",
                "Q_DATA_TRANSACTIONS",
                "Q_DATA_RELATIONSHIPS",
                "Q_DATA_IMPACT",
                "Q_DATA_EXAMPLES",
            },
        )

    def test_evidence_backed_unknowns_are_an_explicit_quality_state(self):
        self.assertEqual(
            audit_bundle.audit_bundle(FIXTURES / "evidence-backed-unknowns"),
            [],
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
