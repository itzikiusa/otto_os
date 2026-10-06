"""Unit tests for check-nextest-filters.py (pure parsing; no cargo)."""

import importlib.util
import tomllib
import unittest
from pathlib import Path

_SPEC = importlib.util.spec_from_file_location(
    "check_nextest_filters", Path(__file__).with_name("check-nextest-filters.py")
)
mod = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(mod)


class SplitTest(unittest.TestCase):
    def test_splits_only_top_level_or(self):
        expr = "(package(a) & test(/x$/))\n| (package(b) & (test(/y/) | test(/z/)))"
        self.assertEqual(
            mod.split_top_level_or(expr),
            ["(package(a) & test(/x$/))", "(package(b) & (test(/y/) | test(/z/)))"],
        )

    def test_regex_alternation_is_not_a_split_point(self):
        expr = "package(s) & test(/^k8s_(monitor|backfill)::/) | package(t)"
        self.assertEqual(
            mod.split_top_level_or(expr),
            ["package(s) & test(/^k8s_(monitor|backfill)::/)", "package(t)"],
        )

    def test_single_term(self):
        self.assertEqual(mod.split_top_level_or("package(x)"), ["package(x)"])

    def test_counts_only_matching_cases(self):
        listing = {
            "rust-suites": {
                "a::lib": {
                    "testcases": {
                        "t1": {"filter-match": {"status": "matches"}},
                        "t2": {"filter-match": {"status": "mismatch", "reason": "expression"}},
                    }
                },
                "b::it": {"testcases": {"t3": {"filter-match": {"status": "matches"}}}},
            }
        }
        self.assertEqual(mod.count_matches(listing), 2)

    def test_repo_config_parses_into_disjuncts(self):
        # Every override in the real config yields at least one disjunct, and
        # none of them still names the pre-split otto-server lib paths.
        config = tomllib.loads(mod.CONFIG.read_text())
        filters = mod.override_filters(config)
        self.assertTrue(filters)
        for _, flt in filters:
            parts = mod.split_top_level_or(flt)
            self.assertTrue(parts)
            for p in parts:
                self.assertNotIn("swarm_wake::", p)
                self.assertNotRegex(p, r"package\(otto-server\) & kind\(lib\)")


if __name__ == "__main__":
    unittest.main()


class ConventionTest(unittest.TestCase):
    def test_no_regex_alternation_inside_a_disjunct(self):
        # `test(/^(a|b)::/)` keeps matching when ONE of a/b moves, so half the
        # disjunct can be orphaned unseen (S12-308). Write one disjunct each.
        config = tomllib.loads(mod.CONFIG.read_text())
        for profile, flt in mod.override_filters(config):
            for d in mod.split_top_level_or(flt):
                for rx in mod.regexes(d):
                    self.assertNotIn("|", rx, f"[{profile}] regex alternation in {d!r}")

    def test_regexes_extracts_every_regex_literal(self):
        self.assertEqual(
            mod.regexes("package(a) & test(/^x(y|z)$/) & test(/w/)"), ["^x(y|z)$", "w"]
        )
