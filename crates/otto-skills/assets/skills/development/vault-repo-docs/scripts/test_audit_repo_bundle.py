import json
import tempfile
import unittest
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
    def setup_bundle(self, root):
        for name in ("index.md", "overview.md", "log.md"):
            (root / name).write_text(BASE.format(type="Service") + f"# {name}\n")
        manifest = {"version": 1, "candidates": [{"id": "api:a", "kind": "api"}, {"id": "data:b", "kind": "data"}, {"id": "worker:c", "kind": "worker"}]}
        manifest_path = root / "manifest.json"
        manifest_path.write_text(json.dumps(manifest))
        return manifest_path

    def test_reports_missing_coverage_and_shallow_api_data(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            manifest = self.setup_bundle(root)
            (root / "coverage.md").write_text("| Candidate | Kind | Evidence | Status | Document | Reason |\n|---|---|---|---|---|---|\n")
            (root / "api.md").write_text(BASE.format(type="API Endpoint") + "# Request\nDTO\n# Response\nDTO\n")
            (root / "data.md").write_text(BASE.format(type="Data Asset") + "# Schema\norders\n")
            rules = {item["rule"] for item in audit_repo_bundle.audit(root, manifest)}
            self.assertIn("R_COVERAGE_MISSING", rules)
            self.assertIn("R_API_REQUEST", rules)
            self.assertIn("R_API_RESPONSE", rules)
            self.assertIn("R_DATA_ACCESS", rules)
            self.assertIn("R_DATA_IMPACT", rules)

    def test_complete_compact_bundle_is_clean(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            manifest = self.setup_bundle(root)
            (root / "coverage.md").write_text("""| Candidate | Kind | Evidence | Status | Document | Reason |
|---|---|---|---|---|---|
| `api:a` | api | `src/api.rs:1` | documented | [API](api.md) | Route verified |
| `data:b` | data | `schema.sql:1` | documented | [Data](data.md) | Schema and calls verified |
| `worker:c` | worker | `worker.rs:1` | documented | [Worker](worker.md) | Registration verified |
""")
            (root / "api.md").write_text(BASE.format(type="API Endpoint") + """# Authentication
Bearer role.
# Request Body
```json
{"id":"o1"}
```
# Success Response
```json
{"status":"ok"}
```
# Errors
400:
```json
{"error":"invalid"}
```
# Flow
[Create flow](worker.md)
# Citations
`src/api.rs:1` `src/dto.rs:4`
""")
            (root / "data.md").write_text(BASE.format(type="Data Asset") + """# Schema
| Field | Type | Description |
|---|---|---|
| id | text | identifier |
# Access Paths
Read SELECT and write INSERT. `src/dao.rs:2`
# Indexes and TTL
Primary index; retention has no TTL.
# Transactions and Consistency
Atomic transaction.
# Field-level Impact
`id` is read by `src/dao.rs:2` and written by `src/dao.rs:8`.
""")
            (root / "worker.md").write_text(BASE.format(type="Runbook") + "# Worker schedule\nReconcile worker registration `worker.rs:1`.\n")
            (root / "api-openapi.yaml").write_text("""openapi: 3.0.3
paths:
  /orders:
    post:
      requestBody:
        content: {}
      responses:
        '200': {description: ok}
""")
            self.assertEqual([], audit_repo_bundle.audit(root, manifest))


if __name__ == "__main__":
    unittest.main()
