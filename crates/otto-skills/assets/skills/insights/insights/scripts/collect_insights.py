#!/usr/bin/env python3
"""
collect_insights.py — multi-provider session collector for the Otto `insights` skill.

Gathers coding-agent sessions from every provider Otto uses, tags each with its
provider, aggregates a COMBINED view plus PER-PROVIDER views, writes the run's
metrics to a history directory (so the next run can compute trends), and prints
the whole thing as JSON for the agent to render into an action-first HTML report.

Providers
---------
  claude  — FULL signal. Transcripts in ~/.claude/projects/<enc_cwd>/*.jsonl
            PLUS the rich facet data in ~/.claude/usage-data/{session-meta,facets}/*.json
            (goal_categories, friction, outcomes, brief_summary, ...). This is the
            only provider with narrative facets, so its insights can be narrative.
  codex   — BASIC signal. Transcripts in
            ~/.codex/sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl
            First line is a `session_meta` object; user messages are
            event_msg/user_message, tools are response_item/function_call +
            custom_tool_call, timing is event_msg/task_complete. NO facets, so its
            insights are quantitative/behavioral only (message counts, durations,
            tools, hour histogram). The report must say so.
  agy     — PLUGGABLE / BASIC. The exact transcript path for agy/gemini is NOT
            confirmed in this codebase, so this is a clearly-marked adapter that
            *probes* a list of candidate locations and parses whatever it finds with
            the generic basic-metrics path. If nothing is found it emits an empty
            provider section with a note — it never hardcodes a wrong path.

Usage
-----
  collect_insights.py                         # back-compat: last 7 days (combined+per-provider)
  collect_insights.py 14                      # back-compat positional days_back
  collect_insights.py --period day            # the CURRENT day
  collect_insights.py --period week --offset 1   # the PREVIOUS week (Mon-Sun)
  collect_insights.py --period month --offset 1  # the PREVIOUS calendar month
  collect_insights.py --start 2026-06-01 --end 2026-06-07   # explicit range (inclusive)
  collect_insights.py --period day --offset 1 --no-history  # don't write history

Week convention: Monday 00:00 .. Sunday 23:59:59 (ISO week, Mon-Sun).

Output shape
------------
  {
    "period":   { kind, start, end, label, offset, ... },
    "providers_present": ["claude", "codex", ...],
    "combined": { ...same aggregate shape as the original report... },
    "per_provider": {
        "claude": { ...full aggregate + session_summaries/friction_details... },
        "codex":  { ...basic aggregate, "depth": "basic", "note": "..." },
        "agy":    { ...basic-or-empty, "depth": "basic", "note": "..." }
    },
    "history": { "kind": ..., "metrics_path": ..., "previous_metrics_path": ... or null }
  }

On total failure (no sessions anywhere) it prints {"error": "..."} and exits 0,
matching the original's graceful behavior so a headless scheduler never crashes.
"""

import argparse
import calendar
import glob
import json
import os
import sys
import uuid
from collections import defaultdict
from datetime import datetime, timedelta, timezone
from typing import Any, Dict

# ----------------------------------------------------------------------------- paths
HOME = os.path.expanduser("~")

# claude
CLAUDE_USAGE_DIR = os.path.join(HOME, ".claude", "usage-data")
CLAUDE_META_DIR = os.path.join(CLAUDE_USAGE_DIR, "session-meta")
CLAUDE_FACETS_DIR = os.path.join(CLAUDE_USAGE_DIR, "facets")
CLAUDE_PROJECTS_DIR = os.path.join(HOME, ".claude", "projects")

# codex
CODEX_SESSIONS_DIR = os.path.join(HOME, ".codex", "sessions")

# agy / gemini — UNCONFIRMED. We probe these candidates; we do NOT assert any of
# them is "the" path. Add/adjust as the real layout is confirmed in Otto.
AGY_CANDIDATE_DIRS = [
    os.path.join(HOME, ".agy", "sessions"),
    os.path.join(HOME, ".gemini", "antigravity-cli", "sessions"),
    os.path.join(HOME, ".gemini", "sessions"),
    os.path.join(HOME, ".gemini", "tmp"),  # last resort; may contain session jsonl
]

# history (the collector writes the run's metrics here; the skill writes HTML beside it)
HISTORY_ROOT = os.path.join(
    HOME, "Library", "Application Support", "Otto", "insights"
)


# ----------------------------------------------------------------------------- time
def parse_date(iso_str):
    """Parse an ISO timestamp to a naive (tz-stripped) datetime, like the original."""
    if isinstance(iso_str, (int, float)):
        # epoch millis (gemini history.jsonl) or seconds
        ts = iso_str / 1000.0 if iso_str > 1e11 else float(iso_str)
        return datetime.fromtimestamp(ts, tz=timezone.utc).replace(tzinfo=None)
    return datetime.fromisoformat(str(iso_str).replace("Z", "+00:00")).replace(
        tzinfo=None
    )


def resolve_window(args):
    """Return (kind, start_dt, end_dt, label). Inclusive end (23:59:59.999999)."""
    now = datetime.now()

    def day_floor(d):
        return d.replace(hour=0, minute=0, second=0, microsecond=0)

    def day_ceil(d):
        return d.replace(hour=23, minute=59, second=59, microsecond=999999)

    # explicit start/end wins
    if args.start and args.end:
        s = day_floor(datetime.fromisoformat(args.start))
        e = day_ceil(datetime.fromisoformat(args.end))
        return ("adhoc", s, e, f"{args.start} → {args.end}")

    if args.period:
        off = args.offset or 0
        if args.period == "day":
            anchor = day_floor(now) - timedelta(days=off)
            s, e = day_floor(anchor), day_ceil(anchor)
            return ("daily", s, e, s.strftime("%a %d %b %Y"))
        if args.period == "week":
            # ISO week, Monday-Sunday
            this_mon = day_floor(now) - timedelta(days=now.weekday())
            mon = this_mon - timedelta(weeks=off)
            sun = mon + timedelta(days=6)
            s, e = day_floor(mon), day_ceil(sun)
            return (
                "weekly",
                s,
                e,
                f"week of {mon.strftime('%d %b')}–{sun.strftime('%d %b %Y')}",
            )
        if args.period == "month":
            # walk back `off` calendar months from the current month
            y, mth = now.year, now.month
            total = (y * 12 + (mth - 1)) - off
            y2, m2 = divmod(total, 12)
            m2 += 1
            last_day = calendar.monthrange(y2, m2)[1]
            s = datetime(y2, m2, 1, 0, 0, 0, 0)
            e = datetime(y2, m2, last_day, 23, 59, 59, 999999)
            return ("monthly", s, e, s.strftime("%B %Y"))

    # back-compat positional days_back
    days_back = args.days_back if args.days_back is not None else 7
    s = day_floor(now - timedelta(days=days_back))
    e = now
    return ("adhoc", s, e, f"last {days_back} days")


# ----------------------------------------------------------------------------- shared aggregate
def in_window(dt, start, end):
    return start <= dt <= end


def empty_aggregate(provider, depth, note):
    """The shape every provider/combined view conforms to (matches the original)."""
    return {
        "provider": provider,
        "depth": depth,  # "full" (claude) or "basic" (codex/agy)
        "note": note,
        "stats": {
            "total_sessions": 0,
            "analyzed_sessions": 0,
            "total_messages": 0,
            "total_duration_minutes": 0,
            "total_commits": 0,
            "total_pushes": 0,
            "active_days": 0,
            "msgs_per_day": 0,
            "achievement_rate": 0,
            "median_response_time": 0,
            "avg_response_time": 0,
            "total_lines_added": 0,
            "total_lines_removed": 0,
            "total_files_modified": 0,
            "total_input_tokens": 0,
            "total_output_tokens": 0,
        },
        "charts": {
            "goal_categories": {},
            "tool_counts": {},
            "languages": {},
            "session_types": {},
            "outcomes": {},
            "friction_types": {},
            "satisfaction": {},
            "success_types": {},
            "helpfulness": {},
            "projects": {},
            "hour_counts": {},
            "tool_error_categories": {},
            "response_time_buckets": {},
        },
        "multi_clauding": {
            "overlap_events": 0,
            "overlap_sessions": 0,
            "overlap_messages_pct": 0,
        },
        "session_summaries": [],
        "friction_details": [],
        "project_sessions": {},
    }


def aggregate(sessions, provider, depth, note, start, end):
    """Aggregate a list of {"meta","facet","date"} into the report shape.

    This is the original generate_report.py aggregation, generalized so it works
    for any provider. `facet` is empty {} for codex/agy (no facet signal), so all
    the facet-derived charts simply come out empty for those providers.
    """
    out = empty_aggregate(provider, depth, note)
    if not sessions:
        return out

    sessions = sorted(sessions, key=lambda x: x["date"])

    total_messages = sum(s["meta"].get("user_message_count", 0) for s in sessions)
    total_duration = sum(s["meta"].get("duration_minutes", 0) for s in sessions)
    total_commits = sum(s["meta"].get("git_commits", 0) for s in sessions)
    total_pushes = sum(s["meta"].get("git_pushes", 0) for s in sessions)
    total_sessions = len(sessions)
    analyzed_sessions = sum(1 for s in sessions if s["facet"])

    active_day_set = set()
    for s in sessions:
        tss = s["meta"].get("user_message_timestamps", [])
        if tss:
            for ts in tss:
                try:
                    mt = parse_date(ts)
                    if in_window(mt, start, end):
                        active_day_set.add(mt.strftime("%Y-%m-%d"))
                except (ValueError, TypeError):
                    continue
        else:
            active_day_set.add(s["date"][:10])
    active_days = len(active_day_set)

    total_lines_added = sum(s["meta"].get("lines_added", 0) for s in sessions)
    total_lines_removed = sum(s["meta"].get("lines_removed", 0) for s in sessions)
    total_files_modified = sum(s["meta"].get("files_modified", 0) for s in sessions)
    total_input_tokens = sum(s["meta"].get("input_tokens", 0) for s in sessions)
    total_output_tokens = sum(s["meta"].get("output_tokens", 0) for s in sessions)

    tool_counts = defaultdict(int)
    lang_counts = defaultdict(int)
    goal_cats = defaultdict(int)
    friction_counts = defaultdict(int)
    satisfaction_counts = defaultdict(int)
    success_counts = defaultdict(int)
    session_types = defaultdict(int)
    outcome_counts = defaultdict(int)
    helpfulness_counts = defaultdict(int)
    project_counts = defaultdict(int)
    tool_error_cats = defaultdict(int)
    hour_counts = defaultdict(int)
    response_times = []

    for s in sessions:
        m = s["meta"]
        f = s["facet"]
        for tool, cnt in m.get("tool_counts", {}).items():
            tool_counts[tool] += cnt
        for lang, cnt in m.get("languages", {}).items():
            lang_counts[lang] += cnt
        for cat, cnt in m.get("tool_error_categories", {}).items():
            tool_error_cats[cat] += cnt
        proj = m.get("project_path", "unknown").split("/")[-1]
        project_counts[proj] += 1

        rts = m.get("user_response_times", [])
        response_times.extend([t for t in rts if isinstance(t, (int, float))])

        tss = m.get("user_message_timestamps", [])
        if tss:
            for ts in tss:
                try:
                    hour_counts[str(parse_date(ts).hour)] += 1
                except (ValueError, TypeError):
                    pass
        else:
            try:
                hour_counts[str(parse_date(m["start_time"]).hour)] += m.get(
                    "user_message_count", 0
                )
            except (ValueError, TypeError, KeyError):
                pass

        if f:
            for k, v in f.get("goal_categories", {}).items():
                goal_cats[k] += v
            for k, v in f.get("friction_counts", {}).items():
                friction_counts[k] += v
            for k, v in f.get("user_satisfaction_counts", {}).items():
                satisfaction_counts[k] += v
            ps = f.get("primary_success", "")
            if ps:
                success_counts[ps] += 1
            st = f.get("session_type", "")
            if st:
                session_types[st] += 1
            outcome = f.get("outcome", "")
            if outcome:
                outcome_counts[outcome] += 1
            ch = f.get("claude_helpfulness", "")
            if ch:
                helpfulness_counts[ch] += 1

    # per-project detail
    project_sessions = defaultdict(list)
    for s in sessions:
        proj = s["meta"].get("project_path", "unknown").split("/")[-1]
        project_sessions[proj].append(
            {
                "date": s["date"],
                "duration_minutes": s["meta"].get("duration_minutes", 0),
                "user_messages": s["meta"].get("user_message_count", 0),
                "outcome": s["facet"].get("outcome", "unknown"),
                "summary": s["facet"].get("brief_summary", ""),
                "underlying_goal": s["facet"].get("underlying_goal", ""),
                "friction": list(s["facet"].get("friction_counts", {}).keys()),
                "friction_detail": s["facet"].get("friction_detail", ""),
                "primary_success": s["facet"].get("primary_success", ""),
                "session_type": s["facet"].get("session_type", ""),
                "helpfulness": s["facet"].get("claude_helpfulness", ""),
            }
        )

    sorted_rt = sorted(response_times) if response_times else [0]
    median_rt = sorted_rt[len(sorted_rt) // 2]
    avg_rt = sum(sorted_rt) / len(sorted_rt) if sorted_rt else 0

    fully = outcome_counts.get("fully_achieved", 0)
    mostly = outcome_counts.get("mostly_achieved", 0)
    total_analyzed = sum(outcome_counts.values()) or 1
    achievement_rate = round((fully + mostly) / total_analyzed * 100)

    session_summaries = []
    for s in sessions[:50]:
        summary = s["facet"].get("brief_summary", "")
        if summary:
            proj = s["meta"].get("project_path", "unknown").split("/")[-1]
            session_summaries.append(f"[{proj}] {summary}")

    friction_details = []
    for s in sessions:
        fd = s["facet"].get("friction_detail", "")
        if fd and fd.strip():
            friction_details.append(fd)
    friction_details = friction_details[:20]

    # multi-session overlap (parallel sessions / "multi-clauding")
    overlap_events = 0
    overlap_sessions = set()
    for i, s1 in enumerate(sessions):
        try:
            s1_start = parse_date(s1["meta"]["start_time"])
        except (ValueError, TypeError, KeyError):
            continue
        s1_end = s1_start + timedelta(
            minutes=max(s1["meta"].get("duration_minutes", 0), 1)
        )
        for s2 in sessions[i + 1:]:
            try:
                s2_start = parse_date(s2["meta"]["start_time"])
            except (ValueError, TypeError, KeyError):
                continue
            if s2_start < s1_end:
                overlap_events += 1
                overlap_sessions.add(s1["meta"]["session_id"])
                overlap_sessions.add(s2["meta"]["session_id"])
            else:
                break
    overlap_messages = sum(
        s["meta"].get("user_message_count", 0)
        for s in sessions
        if s["meta"]["session_id"] in overlap_sessions
    )
    overlap_pct = (
        round(overlap_messages / total_messages * 100) if total_messages > 0 else 0
    )

    out["stats"] = {
        "total_sessions": total_sessions,
        "analyzed_sessions": analyzed_sessions,
        "total_messages": total_messages,
        "total_duration_minutes": total_duration,
        "total_commits": total_commits,
        "total_pushes": total_pushes,
        "active_days": active_days,
        "msgs_per_day": round(total_messages / active_days, 1)
        if active_days > 0
        else 0,
        "achievement_rate": achievement_rate,
        "median_response_time": round(median_rt, 1),
        "avg_response_time": round(avg_rt, 1),
        "total_lines_added": total_lines_added,
        "total_lines_removed": total_lines_removed,
        "total_files_modified": total_files_modified,
        "total_input_tokens": total_input_tokens,
        "total_output_tokens": total_output_tokens,
    }
    out["charts"] = {
        "goal_categories": dict(sorted(goal_cats.items(), key=lambda x: -x[1])),
        "tool_counts": dict(sorted(tool_counts.items(), key=lambda x: -x[1])[:8]),
        "languages": dict(sorted(lang_counts.items(), key=lambda x: -x[1])[:6]),
        "session_types": dict(sorted(session_types.items(), key=lambda x: -x[1])),
        "outcomes": dict(sorted(outcome_counts.items(), key=lambda x: -x[1])),
        "friction_types": dict(sorted(friction_counts.items(), key=lambda x: -x[1])),
        "satisfaction": dict(sorted(satisfaction_counts.items(), key=lambda x: -x[1])),
        "success_types": dict(sorted(success_counts.items(), key=lambda x: -x[1])),
        "helpfulness": dict(sorted(helpfulness_counts.items(), key=lambda x: -x[1])),
        "projects": dict(sorted(project_counts.items(), key=lambda x: -x[1])),
        "hour_counts": dict(hour_counts),
        "tool_error_categories": dict(
            sorted(tool_error_cats.items(), key=lambda x: -x[1])
        ),
        "response_time_buckets": {
            "2-10s": sum(1 for t in response_times if t < 10),
            "10-30s": sum(1 for t in response_times if 10 <= t < 30),
            "30s-1m": sum(1 for t in response_times if 30 <= t < 60),
            "1-2m": sum(1 for t in response_times if 60 <= t < 120),
            "2-5m": sum(1 for t in response_times if 120 <= t < 300),
            "5-15m": sum(1 for t in response_times if 300 <= t < 900),
            ">15m": sum(1 for t in response_times if t >= 900),
        },
    }
    out["multi_clauding"] = {
        "overlap_events": overlap_events,
        "overlap_sessions": len(overlap_sessions),
        "overlap_messages_pct": overlap_pct,
    }
    out["session_summaries"] = session_summaries
    out["friction_details"] = friction_details
    out["project_sessions"] = dict(project_sessions)
    return out


# ----------------------------------------------------------------------------- claude collector (FULL)
def collect_claude(start, end):
    """Port of the original collector: session-meta + facets + JSONL fallback."""
    sessions = []
    if os.path.isdir(CLAUDE_META_DIR):
        for f in glob.glob(os.path.join(CLAUDE_META_DIR, "*.json")):
            try:
                with open(f) as fh:
                    meta = json.load(fh)
                st = parse_date(meta["start_time"])

                has_activity = False
                for ts in meta.get("user_message_timestamps", []):
                    try:
                        if in_window(parse_date(ts), start, end):
                            has_activity = True
                            break
                    except (ValueError, TypeError):
                        continue
                if not has_activity and in_window(st, start, end):
                    has_activity = True
                if not has_activity:
                    dur = meta.get("duration_minutes", 0)
                    session_end = st + timedelta(minutes=max(dur, 1))
                    if st < start and session_end >= start:
                        has_activity = True
                if not has_activity:
                    continue

                sid = meta["session_id"]
                facet = {}
                facet_path = os.path.join(CLAUDE_FACETS_DIR, sid + ".json")
                if os.path.exists(facet_path):
                    with open(facet_path) as fh2:
                        facet = json.load(fh2)

                in_window_ts = []
                effective_date = st
                for ts in meta.get("user_message_timestamps", []):
                    try:
                        mt = parse_date(ts)
                        if in_window(mt, start, end):
                            in_window_ts.append(ts)
                            if effective_date < start:
                                effective_date = mt
                    except (ValueError, TypeError):
                        continue
                if in_window_ts:
                    meta["user_message_timestamps"] = in_window_ts
                    meta["user_message_count"] = len(in_window_ts)
                sessions.append(
                    {
                        "meta": meta,
                        "facet": facet,
                        "date": effective_date.isoformat(),
                        "provider": "claude",
                    }
                )
            except (json.JSONDecodeError, KeyError, OSError):
                continue

    known = set(s["meta"]["session_id"] for s in sessions)

    # JSONL fallback for sessions missing from session-meta
    if os.path.isdir(CLAUDE_PROJECTS_DIR):
        name_map = {}
        for d in os.listdir(CLAUDE_PROJECTS_DIR):
            name_map[d] = d.replace("-", "/", 3)
        for proj_dir in os.listdir(CLAUDE_PROJECTS_DIR):
            proj_path = os.path.join(CLAUDE_PROJECTS_DIR, proj_dir)
            if not os.path.isdir(proj_path):
                continue
            for jf in glob.glob(os.path.join(proj_path, "*.jsonl")):
                sid = os.path.basename(jf).replace(".jsonl", "")
                if sid in known:
                    continue
                meta = _parse_claude_jsonl(jf, name_map.get(proj_dir, proj_dir), start, end)
                if meta:
                    facet = {}
                    fp = os.path.join(CLAUDE_FACETS_DIR, sid + ".json")
                    if os.path.exists(fp):
                        with open(fp) as fh2:
                            facet = json.load(fh2)
                    sessions.append(
                        {
                            "meta": meta,
                            "facet": facet,
                            "date": parse_date(meta["start_time"]).isoformat(),
                            "provider": "claude",
                        }
                    )
                    known.add(sid)
    return sessions


def _parse_claude_jsonl(path, project_dir, start, end):
    real_ts = []
    tool_counts = defaultdict(int)
    try:
        with open(path) as fh:
            for line in fh:
                line = line.strip()
                if not line:
                    continue
                try:
                    obj = json.loads(line)
                except json.JSONDecodeError:
                    continue
                mt = obj.get("type", "")
                if mt == "user":
                    ts = obj.get("timestamp", "")
                    is_real = False
                    msg = obj.get("message", {})
                    if isinstance(msg, dict):
                        content = msg.get("content", "")
                        if isinstance(content, str) and content.strip():
                            is_real = True
                        elif isinstance(content, list):
                            if not any(
                                c.get("type") == "tool_result"
                                for c in content
                                if isinstance(c, dict)
                            ):
                                is_real = True
                    if is_real and ts:
                        real_ts.append(ts)
                elif mt == "assistant":
                    msg = obj.get("message", {})
                    if isinstance(msg, dict):
                        for block in msg.get("content", []) or []:
                            if isinstance(block, dict) and block.get("type") == "tool_use":
                                name = block.get("name", "unknown")
                                if name.startswith("mcp__"):
                                    name = name.split("__")[-1]
                                tool_counts[name] += 1
    except (IOError, OSError):
        return None

    in_window_ts = []
    for ts in real_ts:
        try:
            mt = parse_date(ts)
            if in_window(mt, start, end):
                in_window_ts.append(mt)
        except (ValueError, TypeError):
            continue
    if not in_window_ts:
        return None

    first_ts, last_ts = min(in_window_ts), max(in_window_ts)
    return {
        "session_id": os.path.basename(path).replace(".jsonl", ""),
        "start_time": first_ts.isoformat(),
        "user_message_count": len(in_window_ts),
        "user_message_timestamps": [t.isoformat() for t in in_window_ts],
        "duration_minutes": int((last_ts - first_ts).total_seconds() / 60),
        "project_path": project_dir,
        "tool_counts": dict(tool_counts),
        "_source": "jsonl",
    }


# ----------------------------------------------------------------------------- codex collector (BASIC)
def collect_codex(start, end):
    """Parse ~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl into basic metrics.

    Format (confirmed in this codebase):
      line 1: {type:"session_meta", payload:{id,cwd,timestamp,...}}
      user msgs: {type:"event_msg", payload:{type:"user_message", message:"..."}}
      tools:     {type:"response_item", payload:{type:"function_call", name:"..."}}
                 {type:"response_item", payload:{type:"custom_tool_call", name:"..."}}
      timing:    {type:"event_msg", payload:{type:"task_complete", duration_ms, ...}}
    No facets exist for codex, so facet={} → all narrative charts come out empty.
    """
    sessions = []
    if not os.path.isdir(CODEX_SESSIONS_DIR):
        return sessions
    for jf in glob.glob(
        os.path.join(CODEX_SESSIONS_DIR, "*", "*", "*", "rollout-*.jsonl")
    ):
        meta = _parse_codex_jsonl(jf, start, end)
        if meta:
            sessions.append(
                {
                    "meta": meta,
                    "facet": {},
                    "date": parse_date(meta["start_time"]).isoformat(),
                    "provider": "codex",
                }
            )
    return sessions


def _parse_codex_jsonl(path, start, end):
    cwd = None
    sid = None
    user_ts = []
    tool_counts = defaultdict(int)
    durations_ms = []
    try:
        with open(path) as fh:
            for line in fh:
                line = line.strip()
                if not line:
                    continue
                try:
                    obj = json.loads(line)
                except json.JSONDecodeError:
                    continue
                outer = obj.get("type")
                payload = obj.get("payload", {})
                if not isinstance(payload, dict):
                    continue
                ts = obj.get("timestamp")

                if outer == "session_meta":
                    cwd = payload.get("cwd")
                    sid = payload.get("id")
                    continue

                ptype = payload.get("type")
                if outer == "event_msg" and ptype == "user_message":
                    if ts:
                        user_ts.append(ts)
                elif outer == "response_item" and ptype in (
                    "function_call",
                    "custom_tool_call",
                ):
                    name = payload.get("name", "unknown") or "unknown"
                    tool_counts[name] += 1
                elif outer == "event_msg" and ptype == "task_complete":
                    d = payload.get("duration_ms")
                    if isinstance(d, (int, float)):
                        durations_ms.append(d)
    except (IOError, OSError):
        return None

    in_window_ts = []
    for ts in user_ts:
        try:
            mt = parse_date(ts)
            if in_window(mt, start, end):
                in_window_ts.append(mt)
        except (ValueError, TypeError):
            continue
    if not in_window_ts:
        return None

    first_ts, last_ts = min(in_window_ts), max(in_window_ts)
    if not sid:
        sid = os.path.basename(path).replace(".jsonl", "")
    # codex task durations are per-turn response latency → reuse as response_times (seconds)
    response_times = [d / 1000.0 for d in durations_ms if isinstance(d, (int, float))]
    return {
        "session_id": sid,
        "start_time": first_ts.isoformat(),
        "user_message_count": len(in_window_ts),
        "user_message_timestamps": [t.isoformat() for t in in_window_ts],
        "duration_minutes": int((last_ts - first_ts).total_seconds() / 60),
        "project_path": cwd or "unknown",
        "tool_counts": dict(tool_counts),
        "user_response_times": response_times,
        "_source": "codex",
    }


# ----------------------------------------------------------------------------- agy collector (PLUGGABLE STUB)
def collect_agy(start, end):
    """Adapter STUB for agy / gemini.

    The transcript path for agy is NOT confirmed in this codebase, so we probe a
    list of candidate locations rather than hardcode a (possibly wrong) one. If a
    candidate dir holds rollout-style or *.jsonl session files, parse them with the
    generic basic parser. Otherwise return [] and let the caller emit an empty
    provider section with a note.

    To "wire up" agy once its real format is known: confirm the directory, add it
    to AGY_CANDIDATE_DIRS, and (if its schema differs) write a dedicated parser
    mirroring _parse_codex_jsonl. Do NOT guess the schema here.
    """
    sessions = []
    for d in AGY_CANDIDATE_DIRS:
        if not os.path.isdir(d):
            continue
        # recursively look for *.jsonl session transcripts
        for jf in glob.glob(os.path.join(d, "**", "*.jsonl"), recursive=True):
            meta = _parse_generic_basic_jsonl(jf, start, end, provider="agy")
            if meta:
                sessions.append(
                    {
                        "meta": meta,
                        "facet": {},
                        "date": parse_date(meta["start_time"]).isoformat(),
                        "provider": "agy",
                    }
                )
        if sessions:
            break  # first candidate that yields data wins
    return sessions


def _parse_generic_basic_jsonl(path, start, end, provider):
    """Best-effort basic parse of an UNKNOWN jsonl transcript.

    Heuristic only: counts lines that look like a user turn (role=="user" or
    type containing "user_message"), collects timestamps, and counts anything that
    looks like a tool call (a "name" alongside a tool-ish type). It is intentionally
    conservative and honest about being a heuristic. If it finds nothing it returns
    None so the provider stays empty rather than showing garbage.
    """
    user_ts = []
    tool_counts = defaultdict(int)
    cwd = None
    try:
        with open(path) as fh:
            for line in fh:
                line = line.strip()
                if not line:
                    continue
                try:
                    obj = json.loads(line)
                except json.JSONDecodeError:
                    continue
                payload = obj.get("payload", {})
                if not isinstance(payload, dict):
                    payload = {}
                ts = (
                    obj.get("timestamp")
                    or obj.get("ts")
                    or payload.get("timestamp")
                )
                cwd = cwd or obj.get("workspace") or obj.get("cwd")
                ptype = payload.get("type")
                otype = obj.get("type", "")
                role = obj.get("role") or payload.get("role")
                looks_user = (
                    role == "user"
                    or "user_message" in str(ptype)
                    or otype == "user"
                    or "display" in obj  # gemini history.jsonl shape
                )
                if looks_user and ts:
                    user_ts.append(ts)
                if "tool" in str(ptype or "").lower():
                    name = payload.get("name")
                    if name:
                        tool_counts[name] += 1
    except (IOError, OSError):
        return None

    in_window_ts = []
    for ts in user_ts:
        try:
            mt = parse_date(ts)
            if in_window(mt, start, end):
                in_window_ts.append(mt)
        except (ValueError, TypeError):
            continue
    if not in_window_ts:
        return None

    first_ts, last_ts = min(in_window_ts), max(in_window_ts)
    return {
        "session_id": os.path.basename(path),
        "start_time": first_ts.isoformat(),
        "user_message_count": len(in_window_ts),
        "user_message_timestamps": [t.isoformat() for t in in_window_ts],
        "duration_minutes": int((last_ts - first_ts).total_seconds() / 60),
        "project_path": cwd or "unknown",
        "tool_counts": dict(tool_counts),
        "_source": provider,
    }


# ----------------------------------------------------------------------------- history
def history_paths(kind, start, end):
    """Return (kind_dir, metrics_path) for this run's stored metrics."""
    kind_dir = os.path.join(HISTORY_ROOT, kind)
    fname = (
        f"metrics-{kind}-{start.strftime('%Y%m%d')}_{end.strftime('%Y%m%d')}.json"
    )
    return kind_dir, os.path.join(kind_dir, fname)


def find_previous_metrics(kind, start):
    """Find the most recent stored metrics file whose period ENDS before `start`.

    That is the previous comparable period (e.g. last week for a weekly run).
    Returns the path or None.
    """
    kind_dir = os.path.join(HISTORY_ROOT, kind)
    if not os.path.isdir(kind_dir):
        return None
    best = None
    best_end = None
    for f in glob.glob(os.path.join(kind_dir, f"metrics-{kind}-*.json")):
        base = os.path.basename(f)
        try:
            span = base[len(f"metrics-{kind}-"):-len(".json")]
            e_str = span.split("_")[1]
            f_end = datetime.strptime(e_str, "%Y%m%d")
        except (ValueError, IndexError):
            continue
        # strictly before this run's start → a prior period
        if f_end < start and (best_end is None or f_end > best_end):
            best_end = f_end
            best = f
    return best


def write_history(kind, start, end, period, providers_present, combined, per_provider):
    kind_dir, metrics_path = history_paths(kind, start, end)
    os.makedirs(kind_dir, exist_ok=True)
    # Store a compact metrics snapshot (stats + chart keys) — enough for trends,
    # without the bulky per-session narrative text.
    snapshot = {
        "period": period,
        "providers_present": providers_present,
        "combined": {"stats": combined["stats"], "charts": combined["charts"],
                     "multi_clauding": combined["multi_clauding"]},
        "per_provider": {
            p: {"stats": v["stats"], "charts": v["charts"],
                "multi_clauding": v["multi_clauding"]}
            for p, v in per_provider.items()
        },
        "written_at": datetime.now().isoformat(),
    }
    try:
        with open(metrics_path, "w") as fh:
            json.dump(snapshot, fh, indent=2)
    except OSError:
        return metrics_path, None
    return metrics_path, kind_dir


def already_generated(kind, start, end):
    """Idempotency check for the catch-up scheduler.

    A period counts as "already generated" if BOTH the rolling index.json has a
    series row for (kind, period_start, period_end) AND the period's HTML report
    file exists on disk. The series row is written by the collector; the HTML is
    written by the skill — requiring both means a half-finished run (collector ran
    but the agent never produced the report) is correctly treated as NOT done, so
    catch-up will retry it.

    Returns (is_done: bool, report_path: str|None).
    """
    kind_dir, _ = history_paths(kind, start, end)
    report_path = os.path.join(
        kind_dir,
        f"report-{kind}-{start.strftime('%Y%m%d')}_{end.strftime('%Y%m%d')}.html",
    )
    report_on_disk = os.path.exists(report_path)

    period_key = f"{kind}:{start.strftime('%Y%m%d')}_{end.strftime('%Y%m%d')}"
    in_index = False
    index_path = os.path.join(HISTORY_ROOT, "index.json")
    if os.path.exists(index_path):
        try:
            with open(index_path) as fh:
                idx = json.load(fh)
            in_index = any(
                r.get("period_key") == period_key
                for r in (idx.get("series", []) or [])
            )
        except (json.JSONDecodeError, OSError):
            in_index = False

    return (report_on_disk and in_index), (report_path if report_on_disk else None)


def update_index(kind, start, end, label, combined):
    """Maintain the single rolling index.json at the insights root.

    index.json is the WHOLE history trajectory in one small file — the only file
    the skill reads to see full-history trends, so read-cost stays ~constant as
    history grows. It holds:
      - "series": a headline-metric time series, one compact row per period/kind.
      - "action_ledger": the carry-forward action items (the skill appends/updates
        entries here; the collector only ensures the structure exists and records
        the latest headline values so the skill can mark items improved/closed).

    The collector does NOT invent action items (those come from the agent's
    analysis) — it guarantees the file exists, appends/updates this period's series
    row, and leaves the ledger for the skill to edit. This keeps the numeric series
    deterministic while the qualitative ledger stays the agent's job.
    """
    os.makedirs(HISTORY_ROOT, exist_ok=True)
    index_path = os.path.join(HISTORY_ROOT, "index.json")
    index: Dict[str, Any] = {"series": [], "action_ledger": []}
    if os.path.exists(index_path):
        try:
            with open(index_path) as fh:
                loaded = json.load(fh)
            if isinstance(loaded, dict):
                index["series"] = loaded.get("series", []) or []
                index["action_ledger"] = loaded.get("action_ledger", []) or []
        except (json.JSONDecodeError, OSError):
            pass

    st = combined["stats"]
    period_key = f"{kind}:{start.strftime('%Y%m%d')}_{end.strftime('%Y%m%d')}"
    row = {
        "period_key": period_key,
        "kind": kind,
        "label": label,
        "start": start.strftime("%Y-%m-%d"),
        "end": end.strftime("%Y-%m-%d"),
        "headline": {
            "total_sessions": st["total_sessions"],
            "total_messages": st["total_messages"],
            "active_days": st["active_days"],
            "msgs_per_day": st["msgs_per_day"],
            "achievement_rate": st["achievement_rate"],
            "median_response_time": st["median_response_time"],
            "total_duration_minutes": st["total_duration_minutes"],
            "tool_error_total": sum(combined["charts"].get(
                "tool_error_categories", {}).values()),
        },
        "provider_mix": combined.get("provider_mix", {}),
    }
    # replace any existing row for this exact period_key (idempotent re-runs)
    index["series"] = [r for r in index["series"]
                       if r.get("period_key") != period_key]
    index["series"].append(row)
    index["series"].sort(key=lambda r: (r.get("start", ""), r.get("kind", "")))
    index["updated_at"] = datetime.now().isoformat()

    try:
        with open(index_path, "w") as fh:
            json.dump(index, fh, indent=2)
    except OSError:
        return None
    return index_path


# ----------------------------------------------------------------------------- main
def main():
    ap = argparse.ArgumentParser(description="Multi-provider insights collector")
    ap.add_argument("days_back", nargs="?", type=int, default=None,
                    help="back-compat: number of days back (default 7)")
    ap.add_argument("--period", choices=["day", "week", "month"],
                    help="named period; week is Mon-Sun, month is calendar month")
    ap.add_argument("--offset", type=int, default=0,
                    help="periods back: 1 = the PREVIOUS day/week/month")
    ap.add_argument("--start", help="explicit start date YYYY-MM-DD (inclusive)")
    ap.add_argument("--end", help="explicit end date YYYY-MM-DD (inclusive)")
    ap.add_argument("--no-history", action="store_true",
                    help="do not write this run's metrics to the history dir")
    ap.add_argument("--force", action="store_true",
                    help="regenerate even if this period was already generated "
                         "(idempotency override for the catch-up scheduler)")
    args = ap.parse_args()

    kind, start, end, label = resolve_window(args)

    # --- collect every provider (each is isolated; a failure in one never blocks others)
    by_provider = {}
    for name, fn in (("claude", collect_claude), ("codex", collect_codex),
                     ("agy", collect_agy)):
        try:
            by_provider[name] = fn(start, end)
        except Exception as e:  # never let one provider crash the whole run
            sys.stderr.write(f"[insights] {name} collector failed: {e}\n")
            by_provider[name] = []

    all_sessions = [s for lst in by_provider.values() for s in lst]
    providers_present = [p for p, lst in by_provider.items() if lst]

    if not all_sessions:
        print(json.dumps({
            "error": f"No sessions found for {label} "
                     f"({start.date()}..{end.date()}) in any provider "
                     f"(claude/codex/agy).",
            "period": {"kind": kind, "start": start.strftime("%Y-%m-%d"),
                       "end": end.strftime("%Y-%m-%d"), "label": label,
                       "offset": args.offset or 0},
            "providers_present": [],
        }))
        return

    # --- per-provider notes / depth
    notes = {
        "claude": "Full signal: rich facets (goals, friction, outcomes, summaries).",
        "codex": "Basic signal: codex has NO facets — quantitative/behavioral only "
                 "(message counts, durations, tools, hour-of-day). No narrative "
                 "outcome/friction analysis is available for codex.",
        "agy": "Basic signal: agy/gemini transcript path is not yet confirmed; "
               "parsed via the pluggable adapter. No facets.",
    }
    depths = {"claude": "full", "codex": "basic", "agy": "basic"}

    per_provider = {}
    for name in ("claude", "codex", "agy"):
        sess = by_provider[name]
        if sess:
            per_provider[name] = aggregate(
                sess, name, depths[name], notes[name], start, end
            )
        else:
            note = (notes[name] + " (no sessions found this period.)")
            if name == "agy":
                note = ("No agy/gemini sessions found. The transcript path is not "
                        "confirmed in this codebase — adapter probed candidate "
                        "locations and found nothing. See references/data-and-history.md.")
            per_provider[name] = empty_aggregate(name, depths[name], note)

    # --- combined view (all providers together)
    combined = aggregate(
        all_sessions, "combined", "mixed",
        "All providers combined. Facet-derived sections reflect claude sessions "
        "only (codex/agy contribute volume, tools, timing, hours).",
        start, end,
    )
    # tag combined provider mix for the report
    combined["provider_mix"] = {
        p: len(lst) for p, lst in by_provider.items() if lst
    }

    # --- history: write this run, locate previous comparable period
    kind_dir = history_paths(kind, start, end)[0]
    summary_target = os.path.join(
        kind_dir,
        f"summary-{kind}-{start.strftime('%Y%m%d')}_{end.strftime('%Y%m%d')}.md",
    )
    report_target = os.path.join(
        kind_dir,
        f"report-{kind}-{start.strftime('%Y%m%d')}_{end.strftime('%Y%m%d')}.html",
    )
    is_done, existing_report = already_generated(kind, start, end)
    history_block = {
        "kind": kind,
        "history_dir": kind_dir,
        "metrics_path": None,          # written by THIS script (numbers)
        "previous_metrics_path": None, # prior comparable period's metrics (read for trend)
        "index_path": os.path.join(HISTORY_ROOT, "index.json"),
        "summary_target": summary_target,  # the skill writes the ≤10-sentence summary here
        "report_target": report_target,    # the skill writes the full HTML here (human-only)
        "previous_summary_path": None,
        # idempotency for the catch-up scheduler: True if this exact period was
        # already fully generated (index row + HTML on disk).
        "already_generated": is_done and not args.force,
        "existing_report_path": existing_report,
        "force": bool(args.force),
    }
    prev = find_previous_metrics(kind, start)
    history_block["previous_metrics_path"] = prev
    if prev:
        cand = os.path.join(
            os.path.dirname(prev),
            os.path.basename(prev).replace("metrics-", "summary-").replace(
                ".json", ".md"),
        )
        if os.path.exists(cand):
            history_block["previous_summary_path"] = cand
    if not args.no_history and kind != "adhoc":
        metrics_path, _ = write_history(
            kind, start, end,
            {"kind": kind, "start": start.strftime("%Y-%m-%d"),
             "end": end.strftime("%Y-%m-%d"), "label": label,
             "offset": args.offset or 0},
            providers_present, combined, per_provider,
        )
        history_block["metrics_path"] = metrics_path
        history_block["index_path"] = update_index(
            kind, start, end, label, combined) or history_block["index_path"]
    elif kind == "adhoc":
        history_block["note"] = ("adhoc/explicit ranges are not stored to history "
                                 "by default (use --period for trend tracking).")

    result = {
        "period": {
            "kind": kind,
            "start": start.strftime("%Y-%m-%d"),
            "end": end.strftime("%Y-%m-%d"),
            "label": label,
            "offset": args.offset or 0,
            "run_id": uuid.uuid4().hex[:8],
        },
        "providers_present": providers_present,
        "combined": combined,
        "per_provider": per_provider,
        "history": history_block,
    }
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
