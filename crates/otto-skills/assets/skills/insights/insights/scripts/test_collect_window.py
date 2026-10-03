"""Calendar-only regressions: never reads provider homes or writes reports."""
import json
import os
import tempfile
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


class ClaudeTokenTest(unittest.TestCase):
    """A streamed response is several lines; only the last has the final output."""

    def _line(self, msg, output, stop):
        return json.dumps({
            "type": "assistant", "timestamp": "2026-10-03T08:00:00Z",
            "requestId": "req_" + msg,
            "message": {"id": "msg_" + msg, "stop_reason": stop, "usage": {
                "input_tokens": 2, "output_tokens": output,
                "cache_read_input_tokens": 100, "cache_creation_input_tokens": 10}},
        })

    def test_final_usage_per_response_including_subagents(self):
        with tempfile.TemporaryDirectory() as d:
            main = os.path.join(d, "sid.jsonl")
            user = json.dumps({"type": "user", "timestamp": "2026-10-03T07:59:00Z",
                               "message": {"content": "hi"}})
            with open(main, "w") as fh:
                fh.write("\n".join([user, self._line("a", 590, "end_turn"),
                                     self._line("a", 590, "end_turn")]) + "\n")
            sub = os.path.join(d, "sid", "subagents")
            os.makedirs(sub)
            with open(os.path.join(sub, "agent-x.jsonl"), "w") as fh:
                fh.write("\n".join([self._line("b", 5, None),
                                     self._line("b", 141, "tool_use")]) + "\n")
            start = datetime.fromisoformat("2026-10-03T00:00:00")
            end = datetime.fromisoformat("2026-10-03T23:59:59")
            meta = collector._parse_claude_jsonl(main, "/p", start, end)
        self.assertEqual(meta["output_tokens"], 590 + 141)
        self.assertEqual(meta["input_tokens"], 4)
        self.assertEqual(meta["cache_read_tokens"], 200)


if __name__ == "__main__":
    unittest.main()
