import tempfile
import unittest
from pathlib import Path

import inventory_repo


class InventoryRepoTests(unittest.TestCase):
    def test_polyglot_surfaces_are_stable_and_cited(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "src").mkdir()
            (root / "src/api.rs").write_text('.route("/orders", post(create_order))\nstruct CreateOrder { id: String }\n')
            (root / "schema.sql").write_text("CREATE TABLE orders (id TEXT);\nSELECT id FROM orders;\n")
            (root / "worker.go").write_text('cron.Schedule("@hourly", reconcile)\nproducer.Publish("orders", event)\n')
            first = inventory_repo.inventory(root)["candidates"]
            second = inventory_repo.inventory(root)["candidates"]
            self.assertEqual(first, second)
            self.assertTrue({"api", "data", "messaging", "worker"}.issubset({item["kind"] for item in first}))
            self.assertTrue(all(item["evidence"] == f'{item["path"]}:{item["line"]}' for item in first))

    def test_ignores_vendor_and_large_files(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "vendor").mkdir()
            (root / "vendor/x.rs").write_text('.route("/bad", get(handler))')
            (root / "large.rs").write_text("x" * (inventory_repo.MAX_BYTES + 1) + '.route("/bad", get(handler))')
            self.assertEqual([], inventory_repo.inventory(root)["candidates"])


if __name__ == "__main__":
    unittest.main()
