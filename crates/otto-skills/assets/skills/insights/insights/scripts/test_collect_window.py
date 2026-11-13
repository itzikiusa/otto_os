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
            # Measured from the in-window transcript, never the lifetime 10.
            self.assertEqual(got["git_commits"], 0)
            self.assertIn("lines_added", got["_unavailable_window_metrics"])
            self.assertNotIn("git_commits", got["_unavailable_window_metrics"])


class ClaudeWindowMetricsTest(unittest.TestCase):
    """Tool errors, response times and git ops come from the window itself."""

    def setUp(self):
        self.start = datetime(2026, 10, 18)
        self.end = datetime(2026, 10, 18, 23, 59, 59, 999999)

    def _parse(self, rows):
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "s.jsonl")
            with open(path, "w") as f:
                f.write("\n".join(map(json.dumps, rows)))
            return collector._parse_claude_jsonl(path, "/p", self.start, self.end)

    @staticmethod
    def _user(stamp, content="do it"):
        return {"timestamp": stamp, "type": "user", "message": {"content": content}}

    @staticmethod
    def _call(stamp, call_id, name, command=None):
        block = {"type": "tool_use", "id": call_id, "name": name}
        if command is not None:
            block["input"] = {"command": command}
        return {"timestamp": stamp, "type": "assistant",
                "message": {"id": call_id, "content": [block]}}

    @staticmethod
    def _result(stamp, call_id, is_error):
        return {"timestamp": stamp, "type": "user", "message": {"content": [
            {"type": "tool_result", "tool_use_id": call_id, "is_error": is_error,
             "content": "boom"}]}}

    def test_failed_tool_results_are_counted_per_tool_inside_the_window(self):
        got = self._parse([
            # The day before: a failure outside the window does not count.
            self._user("2026-10-17T10:00:00"),
            self._call("2026-10-17T10:00:01", "old", "Bash", "ls"),
            self._result("2026-10-17T10:00:02", "old", True),
            # A call made before midnight whose failure lands inside the window.
            self._call("2026-10-17T23:59:59", "late", "Edit"),
            self._result("2026-10-18T00:00:01", "late", True),
            self._user("2026-10-18T09:00:00"),
            self._call("2026-10-18T09:00:01", "a", "Bash", "make"),
            self._result("2026-10-18T09:00:02", "a", True),
            self._call("2026-10-18T09:00:03", "b", "Bash", "make"),
            self._result("2026-10-18T09:00:04", "b", False),
            self._call("2026-10-18T09:00:05", "c", "mcp__otto__otto_db_query"),
            self._result("2026-10-18T09:00:06", "c", True),
        ])
        self.assertEqual(got["tool_error_categories"],
                         {"Edit": 1, "Bash": 1, "otto_db_query": 1})

    def test_response_time_is_turn_end_to_the_next_real_message(self):
        got = self._parse([
            self._user("2026-10-18T09:00:00"),
            self._call("2026-10-18T09:00:05", "a", "Read"),
            # A tool result is not the person replying.
            self._result("2026-10-18T09:00:06", "a", False),
            {"timestamp": "2026-10-18T09:00:10", "type": "assistant",
             "message": {"id": "t1", "content": [{"type": "text", "text": "done"}]}},
            self._user("2026-10-18T09:00:55"),           # 45 s after the turn ended
            {"timestamp": "2026-10-18T09:01:00", "type": "assistant",
             "message": {"id": "t2", "content": [{"type": "text", "text": "ok"}]}},
            self._user("2026-10-18T09:01:01"),           # 1 s: queued, not a reply
            {"timestamp": "2026-10-18T09:02:00", "type": "assistant",
             "message": {"id": "t3", "content": [{"type": "text", "text": "ok"}]}},
            self._user("2026-10-18T12:00:00"),           # 3 h: the person left
        ])
        self.assertEqual(got["user_response_times"], [45.0])

    def test_git_commits_and_pushes_match_whole_commands_only(self):
        commands = [
            "git add -A && git commit -m 'x'",       # 1 commit
            "git -C repo commit --amend --no-edit",  # 1 commit
            "/usr/bin/git push origin main",         # 1 push
            "git commit-tree HEAD^{tree}",           # plumbing: not a commit
            "legit commit && git pushx",             # lookalikes
            "git log --grep 'commit'",               # mentions the word only
        ]
        rows = [self._user("2026-10-18T09:00:00")]
        for i, cmd in enumerate(commands):
            rows.append(self._call("2026-10-18T09:00:%02d" % (i + 1), "c%d" % i, "Bash", cmd))
        # A commit outside the window does not count.
        rows.append(self._call("2026-10-19T00:00:01", "next", "Bash", "git commit -m y"))
        got = self._parse(rows)
        self.assertEqual((got["git_commits"], got["git_pushes"]), (2, 1))


class UnavailableMetricsTest(unittest.TestCase):
    """What no session measured is null ("not available"), never 0."""

    def _session(self, sid, **meta):
        base = {"session_id": sid, "start_time": "2026-10-18T09:00:00",
                "user_message_count": 1, "user_message_timestamps": ["2026-10-18T09:00:00"],
                "project_path": "/p/x"}
        base.update(meta)
        return {"meta": base, "facet": {}, "date": base["start_time"]}

    def _aggregate(self, sessions):
        return collector.aggregate(sessions, "combined", "mixed", "", datetime(2026, 10, 18),
                                   datetime(2026, 10, 18, 23, 59, 59))

    def test_lifetime_only_sessions_report_null_not_zero(self):
        lifetime = list(collector.WINDOW_METRIC_FIELDS)
        got = self._aggregate([self._session("a", _unavailable_window_metrics=lifetime)])
        for stat in ("total_commits", "total_pushes", "median_response_time",
                     "avg_response_time", "total_lines_added", "total_input_tokens"):
            self.assertIsNone(got["stats"][stat], stat)
        self.assertIsNone(got["charts"]["tool_error_categories"])
        self.assertIsNone(got["charts"]["response_time_buckets"])
        self.assertIn("git_commits", got["unavailable_window_metrics"])
        self.assertIn("tool_error_categories", got["unavailable_window_metrics"])
        self.assertEqual(got["partial_window_metrics"], {})

    def test_measured_and_unmeasured_sessions_mix_as_partial(self):
        claude = self._session(
            "a", git_commits=2, git_pushes=0, tool_error_categories={"Bash": 3},
            user_response_times=[30.0], tool_counts={"Bash": 4},
            _unavailable_window_metrics=list(collector.CLAUDE_TRANSCRIPT_UNMEASURED))
        codex = self._session("b", tool_counts={"read": 1}, user_response_times=[5.0])
        got = self._aggregate([claude, codex])
        self.assertEqual(got["stats"]["total_commits"], 2)
        self.assertEqual(got["stats"]["total_pushes"], 0, "measured zero stays 0")
        self.assertEqual(got["charts"]["tool_error_categories"], {"Bash": 3})
        self.assertEqual(got["stats"]["median_response_time"], 30.0)
        self.assertEqual(got["partial_window_metrics"]["git_commits"], 1)
        self.assertIsNone(got["stats"]["total_lines_added"])
        self.assertIn("lines_added", got["unavailable_window_metrics"])
        self.assertNotIn("git_commits", got["unavailable_window_metrics"])


if __name__ == "__main__":
    unittest.main()
