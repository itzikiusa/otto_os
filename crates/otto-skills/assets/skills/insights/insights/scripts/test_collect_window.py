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



class AccountingWindowTest(unittest.TestCase):
    def setUp(self):
        self.start = datetime(2026, 10, 18)
        self.end = datetime(2026, 10, 18, 23, 59, 59, 999999)

    def test_equivalent_instants_use_host_calendar(self):
        import time
        self.addCleanup(time.tzset)
        with patch.dict(os.environ, {"TZ": "Etc/GMT-3"}):
            time.tzset()
            try:
                expected = datetime(2026, 10, 18, 1, 30)
                for value in ("2026-10-17T22:30:00Z", "2026-10-18T01:30:00+03:00",
                              1792276200):
                    self.assertEqual(collector.parse_date(value), expected)
            finally:
                pass
        time.tzset()

    def test_codex_tools_and_latency_are_windowed(self):
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "rollout.jsonl")
            rows = [{"type": "session_meta", "payload": {"id": "s", "cwd": "/p"}}]
            for day, duration in ((17, 1000), (18, 100000)):
                stamp = f"2026-10-{day}T12:00:00Z"
                rows += [
                    {"timestamp": stamp, "type": "event_msg", "payload": {"type": "user_message"}},
                    {"timestamp": stamp, "type": "response_item", "payload": {"type": "function_call", "name": "read"}},
                    {"timestamp": stamp, "type": "event_msg", "payload": {"type": "task_complete", "duration_ms": duration}},
                ]
            with open(path, "w") as f:
                f.write("\n".join(map(json.dumps, rows)))
            got = collector._parse_codex_jsonl(path, self.start, self.end)
            self.assertEqual(got["tool_counts"], {"read": 1})
            self.assertEqual(got["user_response_times"], [100])

    def test_metadata_does_not_override_window_tokens(self):
        with tempfile.TemporaryDirectory() as d:
            meta_dir, projects = os.path.join(d, "meta"), os.path.join(d, "projects")
            os.makedirs(meta_dir)
            os.makedirs(os.path.join(projects, "p"))
            path = os.path.join(projects, "p", "s.jsonl")
            rows = []
            for day, tokens in ((17, 900), (18, 100)):
                stamp = f"2026-10-{day}T12:00:00Z"
                rows += [
                    {"timestamp": stamp, "type": "user", "message": {"content": "hello"}},
                    {"timestamp": stamp, "type": "assistant", "message": {"id": str(day), "usage": {"input_tokens": tokens}, "content": [{"type": "tool_use", "name": "read"}]}},
                ]
            with open(path, "w") as f:
                f.write("\n".join(map(json.dumps, rows)))
            with open(os.path.join(meta_dir, "s.json"), "w") as f:
                json.dump({"session_id": "s", "start_time": "2026-10-17T12:00:00Z",
                           "user_message_timestamps": ["2026-10-17T12:00:00Z", "2026-10-18T12:00:00Z"],
                           "input_tokens": 1000, "git_commits": 10, "transcript_path": path}, f)
            with patch.multiple(collector, CLAUDE_META_DIR=meta_dir, CLAUDE_PROJECTS_DIR=projects,
                                CLAUDE_FACETS_DIR=os.path.join(d, "facets")):
                got = collector.collect_claude(self.start, self.end)[0]["meta"]
            self.assertEqual(got["input_tokens"], 100)
            self.assertEqual(got["tool_counts"], {"read": 1})
            self.assertNotIn("git_commits", got)


if __name__ == "__main__":
    unittest.main()
