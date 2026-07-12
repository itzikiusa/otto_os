import tempfile
import unittest
from pathlib import Path
from subprocess import run

import inventory_repo


def git(root, *args):
    return run(
        ["git", "-C", str(root), *args],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()


class InventoryRepoTests(unittest.TestCase):
    def test_polyglot_surfaces_are_inventoried_with_scan_accounting(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "src").mkdir()
            (root / "src/api.rs").write_text(
                '#[post("/orders")]\nasync fn create() {}\n', encoding="utf-8"
            )
            (root / "service.proto").write_text(
                'service Orders { rpc CreateOrder (CreateRequest) returns (CreateResponse); }\n',
                encoding="utf-8",
            )
            (root / "schema.graphql").write_text(
                'type Mutation { createOrder(input: OrderInput!): Order! }\n', encoding="utf-8"
            )
            (root / "OrderRepository.java").write_text(
                'interface Orders extends JpaRepository<Order, String> { '
                'Optional<Order> findByCustomerId(String id); }\n',
                encoding="utf-8",
            )
            (root / "mongo.ts").write_text(
                'orders.findOne({ id }); cache.mget(keys);\n', encoding="utf-8"
            )
            (root / "worker.go").write_text(
                'cron.Schedule("@hourly", reconcile)\nproducer.Publish("orders", event)\n',
                encoding="utf-8",
            )

            result = inventory_repo.inventory(root)

            self.assertTrue(
                {"api", "data", "messaging", "worker"}.issubset(
                    {item["kind"] for item in result["candidates"]}
                )
            )
            self.assertEqual("full", result["mode"])
            self.assertEqual(6, result["counts"]["files_scanned"])
            self.assertEqual(6, len(result["scanned_files"]))
            self.assertEqual(
                len(result["candidates"]), result["counts"]["candidates"]
            )
            self.assertTrue(
                all(item["evidence"] == f'{item["path"]}:{item["line"]}' for item in result["candidates"])
            )

    def test_candidate_id_survives_harmless_line_shift(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / "api.rs"
            source.write_text('.route("/orders", post(create))\n', encoding="utf-8")
            first = inventory_repo.inventory(root)["candidates"][0]
            source.write_text('// comment\n\n.route("/orders", post(create))\n', encoding="utf-8")
            second = inventory_repo.inventory(root)["candidates"][0]
            self.assertEqual(first["id"], second["id"])
            self.assertNotEqual(first["evidence"], second["evidence"])

    def test_generic_collection_getters_and_setters_are_not_surfaces(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "utility.rs").write_text(
                "let item = values.get(&key);\n"
                "let row = r.get(\"id\");\n"
                "cell.set(value);\n"
                "items.delete(index);\n"
                "let dir = app.path();\n"
                "Query(q): Query<Options>;\n"
                "/// update selects the next UI row.\n",
                encoding="utf-8",
            )
            self.assertEqual([], inventory_repo.inventory(root)["candidates"])

    def test_records_vendor_large_unsupported_and_symlink_exclusions(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "vendor").mkdir()
            (root / "vendor/x.rs").write_text('.route("/bad", get(handler))')
            (root / "large.rs").write_text(
                "x" * (inventory_repo.MAX_BYTES + 1) + '.route("/bad", get(handler))'
            )
            (root / "README.md").write_text("documentation")
            (root / "real.rs").write_text('.route("/ok", get(handler))')
            (root / "link.rs").symlink_to(root / "real.rs")

            result = inventory_repo.inventory(root)
            reasons = {item["reason"] for item in result["exclusions"]}

            self.assertEqual(["real.rs"], result["scanned_files"])
            self.assertTrue({"skipped directory", "too large", "unsupported extension", "symlink"}.issubset(reasons))
            self.assertEqual(len(result["exclusions"]), result["counts"]["files_excluded"])

    def test_changed_since_scans_only_changed_and_explicit_dependency_files(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            git(root, "init", "-q")
            git(root, "config", "user.email", "test@example.com")
            git(root, "config", "user.name", "Test")
            (root / "api.rs").write_text('.route("/v1", get(v1))\n')
            (root / "dto.rs").write_text("struct V1 {}\n")
            (root / "other.rs").write_text("fn other() {}\n")
            git(root, "add", ".")
            git(root, "commit", "-qm", "base")
            base = git(root, "rev-parse", "HEAD")
            (root / "api.rs").write_text('.route("/v2", get(v2))\n')

            result = inventory_repo.inventory(
                root, changed_since=base, include_files=["dto.rs"]
            )

            self.assertEqual("incremental", result["mode"])
            self.assertEqual(["api.rs", "dto.rs"], result["scanned_files"])
            self.assertIsNone(result["fallback_reason"])

    def test_invalid_or_non_ancestor_baseline_falls_back_to_full_scan(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            git(root, "init", "-q")
            git(root, "config", "user.email", "test@example.com")
            git(root, "config", "user.name", "Test")
            (root / "api.rs").write_text('.route("/v1", get(v1))\n')
            (root / "data.sql").write_text("SELECT id FROM orders;\n")
            git(root, "add", ".")
            git(root, "commit", "-qm", "base")

            result = inventory_repo.inventory(root, changed_since="not-a-revision")

            self.assertEqual("full-fallback", result["mode"])
            self.assertEqual(["api.rs", "data.sql"], result["scanned_files"])
            self.assertIn("invalid", result["fallback_reason"].lower())


if __name__ == "__main__":
    unittest.main()
