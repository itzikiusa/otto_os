"""Calendar-only regressions: never reads provider homes or writes reports."""
import unittest
from datetime import datetime
from types import SimpleNamespace
from unittest.mock import patch

import collect_insights as collector


class WindowTest(unittest.TestCase):
    def test_delayed_start_keeps_requested_calendar_and_cadence(self):
        cases = [
            ("day", "2026-09-26", "2026-09-27", "daily", "2026-09-25", "2026-09-25"),
            ("week", "2026-09-27", "2026-09-28", "weekly", "2026-09-14", "2026-09-20"),
            ("month", "2026-09-30", "2026-10-01", "monthly", "2026-08-01", "2026-08-31"),
        ]
        for period, accepted, started, kind, start, end in cases:
            with self.subTest(period=period):
                class DelayedClock(datetime):
                    @classmethod
                    def now(cls):
                        return cls.fromisoformat(started)
                args = SimpleNamespace(period=period, offset=1, as_of=accepted,
                                       start=None, end=None, days_back=None)
                with patch.object(collector, "datetime", DelayedClock):
                    got_kind, got_start, got_end, _ = collector.resolve_window(args)
                self.assertEqual((got_kind, str(got_start.date()), str(got_end.date())),
                                 (kind, start, end))

    def test_explicit_range_still_is_adhoc(self):
        args = SimpleNamespace(period=None, offset=0, as_of="2026-09-26",
                               start="2026-01-01", end="2026-01-05", days_back=None)
        kind, start, end, _ = collector.resolve_window(args)
        self.assertEqual((kind, str(start.date()), str(end.date())),
                         ("adhoc", "2026-01-01", "2026-01-05"))


if __name__ == "__main__":
    unittest.main()
