import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("upgrade", Path(__file__).with_name("upgrade-pr-review-workflow.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

class UpgradeTests(unittest.TestCase):
    def test_preserves_gates_prompts_edges_and_review_threshold(self):
        source = {"instructions":"Existing rule", "graph":{"nodes":[
            {"id":"gate","kind":"agent_prompt","params":{"prompt":"run pr_gates.sh exactly once"}},
            {"id":"review","kind":"review_run","params":{"threshold":95,"scoring":{"bug":25,"warn":3,"info":5},"reviewers":[{"lens":"security","providers":["claude","codex"]}]}}
        ],"edges":[{"source":"gate","target":"review"}]}}
        result = module.upgrade(source)
        self.assertEqual(result["graph"]["nodes"][0], source["graph"]["nodes"][0])
        self.assertEqual(result["graph"]["edges"], source["graph"]["edges"])
        review = result["graph"]["nodes"][1]["params"]
        self.assertEqual(review["threshold"],95)
        self.assertEqual(review["scoring"],{"bug":25,"warn":3,"info":0})
        self.assertFalse(review["cross_provider_check"])
        self.assertNotIn("cross_provider_check",source["graph"]["nodes"][1]["params"])
        self.assertEqual(module.upgrade(result),result)
        with self.assertRaises(ValueError): module.upgrade(source,{"gate":["docs"]})

    def test_context_reuse_requires_explicit_tracked_dependencies(self):
        source = {"graph":{"nodes":[{"id":"context","kind":"agent_prompt","params":{"prompt":"Read architecture docs"}},{"id":"review","kind":"review_run"}],"edges":[]}}
        upgraded = module.upgrade(source,{"context":["docs/architecture","README.md"]})
        self.assertEqual(upgraded["graph"]["nodes"][0]["params"]["review_context_paths"],["docs/architecture","README.md"])
        for paths in [["../secret"],["/tmp"],[]]:
            with self.assertRaises(ValueError): module.upgrade(source,{"context":paths})

if __name__ == "__main__": unittest.main()
