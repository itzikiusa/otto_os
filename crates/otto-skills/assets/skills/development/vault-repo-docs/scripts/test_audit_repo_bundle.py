import json
import tempfile
import unittest
from collections import Counter
from pathlib import Path

import audit_repo_bundle


BASE = """---
type: {type}
description: Complete source-backed concept.
resource: /repo
tags: [repo]
timestamp: 2026-07-12
---
"""


class AuditRepoBundleTests(unittest.TestCase):
    def setup_bundle(self, root, candidates, mode="full"):
        for name in ("index.md", "overview.md", "log.md"):
            (root / name).write_text(BASE.format(type="Service") + f"# {name}\n")
        manifest = {
            "version": 2,
            "mode": mode,
            "scanned_files": ["src/api.rs"],
            "exclusions": [],
            "counts": {
                "files_scanned": 1,
                "files_excluded": 0,
                "candidates": len(candidates),
                "by_kind": dict(Counter(item["kind"] for item in candidates)),
            },
            "candidates": candidates,
        }
        manifest_path = root / "manifest.json"
        manifest_path.write_text(json.dumps(manifest))
        return manifest_path

    def write_coverage(self, root, rows):
        body = "| Candidate | Kind | Evidence | Status | Document | Reason |\n|---|---|---|---|---|---|\n"
        (root / "coverage.md").write_text(body + "\n".join(rows) + "\n")

    def test_rejects_missing_document_target_and_path_escape(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            manifest = self.setup_bundle(
                root,
                [{"id": "api:a", "kind": "api", "evidence": "src/api.rs:1"}],
            )
            self.write_coverage(
                root,
                ["| `api:a` | api | `src/api.rs:1` | documented | [Missing](does-not-exist.md) | Route verified |"],
            )
            rules = {item["rule"] for item in audit_repo_bundle.audit(root, manifest)}
            self.assertIn("R_COVERAGE_DOC_MISSING", rules)

            self.write_coverage(
                root,
                ["| `api:a` | api | `src/api.rs:1` | documented | [Escape](../outside.md) | Route verified |"],
            )
            rules = {item["rule"] for item in audit_repo_bundle.audit(root, manifest)}
            self.assertIn("R_COVERAGE_DOC_PATH", rules)

    def test_rejects_unrelated_fence_and_skeletal_openapi(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            manifest = self.setup_bundle(
                root,
                [{"id": "api:a", "kind": "api", "evidence": "src/api.rs:1"}],
            )
            self.write_coverage(
                root,
                ["| `api:a` | api | `src/api.rs:1` | documented | [API](api.md) | Route verified |"],
            )
            (root / "api.md").write_text(
                BASE.format(type="Service")
                + """# POST /orders
# Authentication
Bearer.
# Request Body
No schema or example here.
# Success Response
No schema or example here.
# Errors
400 exists.
# Flow
[Flow](flow.md)
# Unrelated config
```json
{"debug":true}
```
# Citations
`src/api.rs:1` `src/dto.rs:2`
"""
            )
            (root / "api-openapi.yaml").write_text(
                """openapi: 3.0.3
paths:
  /x:
    post:
      requestBody:
      responses:
"""
            )

            rules = {item["rule"] for item in audit_repo_bundle.audit(root, manifest)}

            self.assertIn("R_API_REQUEST", rules)
            self.assertIn("R_API_RESPONSE", rules)
            self.assertIn("R_OPENAPI_OPERATION_ID", rules)
            self.assertIn("R_OPENAPI_EXAMPLES", rules)
            self.assertIn("R_OPENAPI_MISMATCH", rules)

    def test_rejects_duplicate_manifest_unknown_rows_and_suspicious_empty_inventory(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            duplicate = {"id": "api:a", "kind": "api", "evidence": "src/api.rs:1"}
            manifest = self.setup_bundle(root, [duplicate, duplicate])
            self.write_coverage(
                root,
                [
                    "| `api:a` | api | `src/api.rs:1` | irrelevant | — | Confirmed false positive |",
                    "| `manual:x` | data | `src/db.rs:2` | irrelevant | — | Confirmed false positive |",
                ],
            )
            rules = {item["rule"] for item in audit_repo_bundle.audit(root, manifest)}
            self.assertIn("R_MANIFEST_DUPLICATE", rules)
            self.assertIn("R_COVERAGE_UNKNOWN", rules)

            payload = json.loads(manifest.read_text())
            payload["counts"]["by_kind"] = {"api": 99}
            manifest.write_text(json.dumps(payload))
            rules = {item["rule"] for item in audit_repo_bundle.audit(root, manifest)}
            self.assertIn("R_MANIFEST_COUNTS", rules)

            empty_manifest = self.setup_bundle(root, [])
            self.write_coverage(root, [])
            rules = {item["rule"] for item in audit_repo_bundle.audit(root, empty_manifest)}
            self.assertIn("R_INVENTORY_EMPTY", rules)

    def test_generated_and_uncertain_rows_cannot_silently_pass(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            manifest = self.setup_bundle(
                root,
                [
                    {"id": "api:a", "kind": "api", "evidence": "src/api.rs:1"},
                    {"id": "data:b", "kind": "data", "evidence": "src/db.rs:2"},
                ],
            )
            self.write_coverage(
                root,
                [
                    "| `api:a` | api | `src/api.rs:1` | generated | — | Generated dependency owns it |",
                    "| `data:b` | data | `src/db.rs:2` | uncertain | — | Schema unavailable |",
                ],
            )
            rules = {item["rule"] for item in audit_repo_bundle.audit(root, manifest)}
            self.assertIn("R_COVERAGE_GENERATED_DOC", rules)
            self.assertIn("R_COVERAGE_UNCERTAIN", rules)
            self.assertTrue(
                any(
                    item["rule"] == "R_COVERAGE_UNCERTAIN" and item["severity"] == "error"
                    for item in audit_repo_bundle.audit(root, manifest)
                )
            )

    def test_complete_source_backed_bundle_is_clean(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            candidates = [
                {"id": "api:a", "kind": "api", "evidence": "src/api.rs:1"},
                {"id": "data:b", "kind": "data", "evidence": "src/dao.rs:2"},
                {"id": "worker:c", "kind": "worker", "evidence": "worker.rs:1"},
                {"id": "messaging:d", "kind": "messaging", "evidence": "events.rs:1"},
                {"id": "runtime:e", "kind": "runtime", "evidence": "main.rs:1"},
            ]
            manifest = self.setup_bundle(root, candidates)
            self.write_coverage(
                root,
                [
                    "| `api:a` | api | `src/api.rs:1` | documented | [API](api.md) | Route and DTO verified |",
                    "| `data:b` | data | `src/dao.rs:2` | documented | [Data](data.md) | Schema and access verified |",
                    "| `worker:c` | worker | `worker.rs:1` | documented | [Worker](worker.md) | Registration verified |",
                    "| `messaging:d` | messaging | `events.rs:1` | documented | [Messaging](messaging.md) | Producer and consumer verified |",
                    "| `runtime:e` | runtime | `main.rs:1` | documented | [Runtime](runtime.md) | Startup and shutdown verified |",
                ],
            )
            (root / "index.md").write_text(
                BASE.format(type="Service")
                + "[API](api.md) [Data](data.md) [Worker](worker.md) "
                + "[Messaging](messaging.md) [Runtime](runtime.md)\n"
            )
            (root / "api.md").write_text(
                BASE.format(type="API Endpoint")
                + """# POST /orders
# Authentication
Bearer role.
# Request Body
```json
{"id":"o1"}
```
Required `id`; see `src/dto.rs:4`.
# Success Response
```json
{"status":"ok"}
```
# Errors
400 returns `{"error":"invalid"}`.
# Flow
[Create flow](worker.md)
# Citations
`src/api.rs:1` `src/dto.rs:4` `src/handler.rs:8`
"""
            )
            (root / "data.md").write_text(
                BASE.format(type="Data Asset")
                + """# Schema
| Field | Type | Description |
|---|---|---|
| id | text | identifier |
# Access Paths
Read SELECT at `src/dao.rs:2`; write INSERT at `src/dao.rs:8`.
# Indexes and TTL
Primary index; retention has no TTL.
# Transactions and Consistency
Atomic transaction.
# Field-level Impact
`id` is read by `src/dao.rs:2` and written by `src/dao.rs:8`.
"""
            )
            (root / "worker.md").write_text(
                BASE.format(type="Runbook")
                + "# Schedule\nHourly worker registered at `worker.rs:1`.\n"
                + "# Failure and retry\nRetries safely with idempotency at `worker.rs:8`.\n"
            )
            (root / "messaging.md").write_text(
                BASE.format(type="Runbook")
                + "# Producer and consumer\nTopic orders at `events.rs:1`.\n"
                + "# Payload example\n```json\n{\"id\":\"o1\"}\n```\n"
                + "# Delivery and retry\nAt-least-once with backoff and dead-letter at `events.rs:8`.\n"
            )
            (root / "runtime.md").write_text(
                BASE.format(type="Runbook")
                + "# Startup and shutdown\nStarts server and handles graceful shutdown at `main.rs:1`.\n"
                + "# Failure and retry\nStartup errors terminate; shutdown retries drain at `main.rs:9`.\n"
            )
            (root / "api-openapi.yaml").write_text(
                """openapi: 3.0.3
paths:
  /orders:
    post:
      operationId: createOrder
      requestBody:
        required: true
        content:
          application/json:
            schema:
              type: object
            example:
              id: o1
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                type: object
              example:
                status: ok
"""
            )

            self.assertEqual([], audit_repo_bundle.audit(root, manifest))

    def test_shipped_api_example_is_a_complete_clean_bundle(self):
        package = Path(__file__).resolve().parent.parent
        example = package / "examples" / "api-flow-bundle"
        self.assertEqual(
            [], audit_repo_bundle.audit(example, example / "manifest.json")
        )


if __name__ == "__main__":
    unittest.main()
