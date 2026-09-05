#!/usr/bin/env python3
"""Redact agent transcripts into shareable test fixtures.

    scripts/redact-transcript.py --out crates/otto-transcript/fixtures/claude \
        --head 120 --name 01-basic ~/.claude/projects/<slug>/<sid>.jsonl

Keeps the SHAPE of every record (keys, enum-ish values, ids as stable
placeholders so tool_use/tool_result linkage survives) and destroys the CONTENT:

  * every prose/code/output string is word-redacted (letters -> x, digits -> 0)
    except a small allowlist of structural tokens the parser keys on
    (`<system-reminder>`, `Process exited with code 0`, `# AGENTS.md instructions`,
    PR URL shapes ...); JSON-in-a-string (`arguments`, `toolUseResult`) is
    parsed, redacted recursively and re-serialized so keys survive;
  * absolute paths -> `/repo/<hash>.<ext>` (deterministic per path), `$HOME`
    never appears; emails -> user@example.com; token-looking strings -> REDACTED;
  * base64 image payloads -> a 1x1 PNG; `signature`/`encrypted_content` -> stubs;
  * uuids / tool ids / request ids / call ids -> per-file stable placeholders.

A Claude session's `<sid>/subagents/` dir (`.jsonl` + `.meta.json` sidecars) is
redacted alongside when `--subagents` is given. Never commit raw transcripts.
"""

import argparse
import base64
import hashlib
import json
import os
import re
import sys

PNG_1X1 = (
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg=="
)

# Structural tokens the parser keys on: kept verbatim inside redacted prose.
ALLOW = set(
    """
system-reminder task-notification task-id tool-use-id output-file status summary
command-name command-message command-args local-command-stdout local-command-caveat
Image Process exited with code Output Chunk ID Wall time seconds Script completed
Success Updated the following files Error Begin Patch End Update File Add Delete Move
AGENTS md instructions for environment_context permissions collaboration_mode INSTRUCTIONS
https github com pull bitbucket org pull-requests merge_requests gitlab
Task created successfully Updated task
system reminder task notification tool use id output file command name message args local stdout caveat
pull requests merge requests
""".split()
)

# Keys whose values are schema, not content: kept verbatim.
KEEP_KEYS = {
    "type", "role", "subtype", "operation", "kind", "model", "provider", "tool", "server",
    "hook_event_name", "name", "mode", "permissionMode", "phase", "source", "status",
    "originator", "thread_source", "model_provider", "cli_version", "version", "entrypoint",
    "userType", "level", "stop_reason", "media_type", "isSidechain", "isMeta", "is_error",
    "success", "interrupted", "isImage", "noOutputExpected", "agentType", "spawnDepth",
    "approval_policy", "collaboration_mode_kind", "reason", "service_tier", "gitBranch",
    "exit_code", "durationMs", "duration_ms", "ordinal", "position",
}
# Keys holding ids that must stay LINKED (same input -> same placeholder).
ID_KEYS = {
    "uuid", "parentUuid", "sessionId", "session_id", "requestId", "id", "tool_use_id",
    "call_id", "toolUseId", "toolUseID", "agentId", "parentAgentId", "promptId", "leafUuid",
    "messageId", "snapshotMessageId", "turn_id", "thread_id", "root_turn_id", "response_id",
    "process_id", "sourceToolAssistantUUID", "taskId", "backgroundTaskId", "task_id",
    "bridgeSessionId", "ownerAccountUuid", "ownerOrganizationUuid",
}
STUB_KEYS = {"signature": "sig", "encrypted_content": "enc"}

EMAIL_RE = re.compile(r"[\w.+-]+@[\w-]+\.[\w.-]+")
TOKEN_RE = re.compile(
    r"(sk-[A-Za-z0-9_-]{10,}|xox[abprs]-[A-Za-z0-9-]{10,}|gh[pousr]_[A-Za-z0-9]{20,}|"
    r"AKIA[A-Z0-9]{12,}|Bearer\s+[A-Za-z0-9._-]{16,}|[A-Fa-f0-9]{40,}|[A-Za-z0-9+/=_-]{64,})"
)
PATH_RE = re.compile(r"(?<![\w/])(?:/[\w.@+~-]+){2,}")
WORD_RE = re.compile(r"[A-Za-z0-9_]+")
UUID_RE = re.compile(r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}")


class Redactor:
    def __init__(self):
        self.ids = {}
        self.paths = {}

    def map_id(self, s):
        if not isinstance(s, str) or not s:
            return s
        if s in self.ids:
            return self.ids[s]
        n = len(self.ids) + 1
        if UUID_RE.fullmatch(s):
            out = f"00000000-0000-4000-8000-{n:012d}"
        else:
            m = re.match(r"^([A-Za-z]+[_-])", s)
            prefix = m.group(1) if m else "id_"
            out = f"{prefix}{n:06d}"
        self.ids[s] = out
        return out

    def map_path(self, p):
        if p in self.paths:
            return self.paths[p]
        ext = os.path.splitext(p)[1]
        if len(ext) > 8:
            ext = ""
        h = hashlib.sha1(p.encode()).hexdigest()[:8]
        out = f"/repo/{h}{ext}"
        self.paths[p] = out
        return out

    def words(self, s):
        def sub(m):
            w = m.group(0)
            if w in ALLOW or len(w) <= 2:
                return w
            return "".join("0" if c.isdigit() else ("X" if c.isupper() else "x") for c in w)

        return WORD_RE.sub(sub, s)

    # Redacted prose keeps its shape, not its length: anything past this is noise.
    TEXT_CAP = 1500

    def text(self, s):
        if not s:
            return s
        if len(s) > self.TEXT_CAP:
            # Cap each span between pseudo-tags separately so a `<system-reminder>`
            # never loses its closing tag to the cut.
            parts = re.split(r"(</?[a-z][a-z-]*>)", s)
            s = "".join(p if i % 2 else (p[: self.TEXT_CAP] + "…" if len(p) > self.TEXT_CAP else p) for i, p in enumerate(parts))
        # JSON-in-a-string: keep keys, redact values.
        st = s.lstrip()
        if st[:1] in "{[":
            try:
                return json.dumps(self.value(json.loads(s), None), separators=(",", ":"))
            except Exception:
                pass
        s = UUID_RE.sub(lambda m: self.map_id(m.group(0)), s)
        s = EMAIL_RE.sub("user@example.com", s)
        s = TOKEN_RE.sub("REDACTED", s)
        s = PATH_RE.sub(lambda m: self.map_path(m.group(0)), s)
        # Redact words outside the placeholders we just inserted.
        parts = re.split(r"(/repo/[0-9a-f]{8}[.\w]*|REDACTED|user@example\.com|00000000-0000-4000-8000-\d{12})", s)
        return "".join(p if i % 2 else self.words(p) for i, p in enumerate(parts))

    def value(self, v, key):
        if isinstance(v, dict):
            out = {}
            for k, x in v.items():
                if k == "source" and isinstance(x, dict) and x.get("type") == "base64":
                    out[k] = {**x, "data": PNG_1X1}
                    continue
                out[k] = self.value(x, k)
            return out
        if isinstance(v, list):
            return [self.value(x, key) for x in v]
        if isinstance(v, str):
            if key in STUB_KEYS:
                return STUB_KEYS[key]
            if key in ID_KEYS:
                return self.map_id(v)
            if key in KEEP_KEYS:
                return v
            if key in ("cwd", "filePath", "file_path", "path", "notebook_path", "trackingPath", "realParentDir", "workdir"):
                return self.map_path(v.replace("file://", "")) if v.startswith(("/", "file://")) else self.text(v)
            if key == "timestamp":
                return v
            return self.text(v)
        if isinstance(v, (int, float)) and key in ID_KEYS:
            return self.map_id(str(v))
        return v

    def record(self, line):
        line = line.strip()
        if not line:
            return None
        try:
            v = json.loads(line)
        except Exception:
            return None  # never emit an unparseable original line
        if isinstance(v, dict):
            # `changes` objects are keyed by absolute path.
            def fix_changes(node):
                if isinstance(node, dict):
                    if "changes" in node and isinstance(node["changes"], dict):
                        node["changes"] = {self.map_path(k): fix_changes(val) for k, val in node["changes"].items()}
                    return {k: fix_changes(val) for k, val in node.items()}
                if isinstance(node, list):
                    return [fix_changes(x) for x in node]
                return node

            v = fix_changes(v)
        return json.dumps(self.value(v, None), separators=(",", ":"), ensure_ascii=False)


def redact_file(r, src, dst, head, skip=0):
    n = 0
    with open(src, errors="replace") as f, open(dst, "w") as out:
        for i, line in enumerate(f):
            if i < skip:
                continue
            rec = r.record(line)
            if rec is None:
                continue
            out.write(rec + "\n")
            n += 1
            if head and n >= head:
                break
    return n


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("src", help="transcript .jsonl")
    ap.add_argument("--out", required=True, help="output directory")
    ap.add_argument("--name", required=True, help="fixture base name (no extension)")
    ap.add_argument("--head", type=int, default=0, help="keep only the first N records (after --skip)")
    ap.add_argument("--skip", type=int, default=0, help="skip the first N records (a window into a long file)")
    ap.add_argument("--subagents", action="store_true", help="also redact <sid>/subagents/* next to the source")
    ap.add_argument("--sub-head", type=int, default=40, help="records per subagent file")
    args = ap.parse_args()

    os.makedirs(args.out, exist_ok=True)
    r = Redactor()
    dst = os.path.join(args.out, args.name + ".jsonl")
    n = redact_file(r, args.src, dst, args.head, args.skip)
    print(f"{dst}: {n} records", file=sys.stderr)

    if args.subagents:
        stem = os.path.splitext(os.path.basename(args.src))[0]
        sub_src = os.path.join(os.path.dirname(args.src), stem, "subagents")
        sub_dst = os.path.join(args.out, args.name, "subagents")
        if os.path.isdir(sub_src):
            os.makedirs(sub_dst, exist_ok=True)
            for fn in sorted(os.listdir(sub_src)):
                p = os.path.join(sub_src, fn)
                if fn.endswith(".meta.json"):
                    agent = fn[len("agent-") : -len(".meta.json")]
                    meta = json.load(open(p))
                    meta = r.value(meta, None)
                    meta["description"] = r.text(meta.get("description", ""))
                    if "parentAgentId" in meta:
                        meta["parentAgentId"] = r.map_id(meta["parentAgentId"])
                    new_agent = r.map_id(agent)
                    with open(os.path.join(sub_dst, f"agent-{new_agent}.meta.json"), "w") as out:
                        json.dump(meta, out, separators=(",", ":"))
                elif fn.endswith(".jsonl"):
                    agent = fn[len("agent-") : -len(".jsonl")]
                    new_agent = r.map_id(agent)
                    m = redact_file(r, p, os.path.join(sub_dst, f"agent-{new_agent}.jsonl"), args.sub_head)
                    print(f"  subagent {new_agent}: {m} records", file=sys.stderr)


if __name__ == "__main__":
    main()
