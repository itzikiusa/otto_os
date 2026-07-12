import unittest
from pathlib import Path

import run_evals


PACKAGE = Path(__file__).resolve().parent.parent


class RunnableEvalTests(unittest.TestCase):
    def test_every_declared_eval_executes_and_passes(self):
        results = run_evals.run(PACKAGE)
        self.assertEqual(
            {
                "full-polyglot-scan",
                "focused-data-scan",
                "incomplete-evidence",
                "negative-code-review",
                "bloat-pressure",
                "conflict-and-script-safety",
            },
            {item["id"] for item in results},
        )
        self.assertEqual([], [item for item in results if not item["passed"]])


if __name__ == "__main__":
    unittest.main()
