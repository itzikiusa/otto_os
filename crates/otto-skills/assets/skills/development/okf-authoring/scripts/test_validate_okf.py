import hashlib
import io
import json
import tempfile
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
    def temporary_bundle(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        root = Path(temporary.name)
        (root / "index.md").write_text(
            '---\nokf_version: "0.1"\n---\n\n# Test\n', encoding="utf-8"
        )
        (root / "log.md").write_text("## 2026-07-16\n", encoding="utf-8")
        return root

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

    def test_strict_mode_fails_when_warnings_exist(self):
        stdout = io.StringIO()
        with redirect_stdout(stdout):
            exit_code = validate_okf.main(
                [str(FIXTURES / "warnings-only"), "--strict", "--format", "json"]
            )

        self.assertEqual(exit_code, 1)

    def test_unquoted_colon_plain_scalar_is_invalid_yaml(self):
        root = self.temporary_bundle()
        (root / "flow.md").write_text(
            """---
type: flow
title: Flow: Startup
description: Starts the service.
timestamp: 2026-07-16T00:00:00Z
---
""",
            encoding="utf-8",
        )

        report = validate_okf.validate_bundle(root)

        self.assertEqual(
            [(item["rule"], item["path"]) for item in report["errors"]],
            [("E1", "flow.md")],
        )

    def test_quoted_scalar_allows_inline_comment_with_colon(self):
        root = self.temporary_bundle()
        (root / "endpoint.md").write_text(
            """---
type: endpoint
title: Healthcheck
description: Describes the healthcheck.
resource: "GET /healthcheck"  # alias: GET /actuator/health
timestamp: 2026-07-16T00:00:00Z
---
""",
            encoding="utf-8",
        )

        report = validate_okf.validate_bundle(root)

        self.assertEqual(report["errors"], [])

    def test_existing_non_markdown_attachment_resolves(self):
        root = self.temporary_bundle()
        (root / "api.yaml").write_text("openapi: 3.0.0\n", encoding="utf-8")
        (root / "reference.md").write_text(
            """---
type: reference
title: API
description: Links the API artifact.
timestamp: 2026-07-16T00:00:00Z
---

[OpenAPI](api.yaml)
""",
            encoding="utf-8",
        )

        report = validate_okf.validate_bundle(root)

        self.assertNotIn("W2", {item["rule"] for item in report["warnings"]})

    def test_file_uri_source_citation_outside_bundle_is_allowed(self):
        root = self.temporary_bundle()
        (root / "reference.md").write_text(
            """---
type: reference
title: Source citation
description: Links source evidence outside the Vault bundle.
timestamp: 2026-07-16T00:00:00Z
---

[Source](file:///Users/example/service/main.go#L12)
""",
            encoding="utf-8",
        )

        report = validate_okf.validate_bundle(root)

        self.assertNotIn("L1", {item["rule"] for item in report["warnings"]})

    def test_directory_only_and_file_uri_links_are_static_failures(self):
        report = validate_okf.validate_bundle(FIXTURES / "static-link-violations")

        self.assertEqual(
            [(item["rule"], item["message"]) for item in report["warnings"]],
            [
                ("L1", "machine-local file URI duplicates a Vault note -> `file:///Users/example/platform_docs/targets/index.md`"),
                ("W2", "directory link must name index.md -> `targets/`"),
            ],
        )

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
