#!/usr/bin/env python3
"""Lightweight D2 diagram quality checker.

This is intentionally conservative. It does not fully parse D2; it catches common
maintainability/readability issues before final review.
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

EDGE_RE = re.compile(r"(?P<src>[A-Za-z0-9_.\-]+)\s*(?:->|--|<-|<->)\s*(?P<dst>[A-Za-z0-9_.\-]+)(?P<rest>.*)")
LABEL_RE = re.compile(r":\s*\"?([^\"{#]+)")

WEAK_EDGE_LABELS = {"uses", "data", "call", "calls", "thing", "stuff", "link", "connects"}


def strip_comment(line: str) -> str:
    in_quote = False
    out = []
    i = 0
    while i < len(line):
        ch = line[i]
        if ch == '"':
            in_quote = not in_quote
        if ch == "#" and not in_quote:
            break
        out.append(ch)
        i += 1
    return "".join(out).rstrip()


def main() -> int:
    if len(sys.argv) != 2:
        print("Usage: d2_doctor.py <diagram.d2>", file=sys.stderr)
        return 2

    path = Path(sys.argv[1])
    if not path.exists():
        print(f"ERROR: file not found: {path}", file=sys.stderr)
        return 2

    text = path.read_text(encoding="utf-8")
    lines = text.splitlines()
    warnings: list[str] = []

    if "direction:" not in text and "shape: sequence_diagram" not in text:
        warnings.append("No global `direction:` found. Add `direction: right` or `direction: down` for predictable layout.")

    edge_count = 0
    unlabeled_edges = 0
    weak_edges = 0
    long_labels = 0

    for idx, raw in enumerate(lines, start=1):
        line = strip_comment(raw).strip()
        if not line or line.startswith("#"):
            continue

        if len(line) > 140:
            warnings.append(f"Line {idx}: very long line; consider breaking label/note for maintainability.")

        m = EDGE_RE.search(line)
        if m:
            edge_count += 1
            rest = m.group("rest")
            label_match = LABEL_RE.search(rest)
            if not label_match:
                unlabeled_edges += 1
                warnings.append(f"Line {idx}: edge appears unlabeled. Important edges should explain the relationship.")
            else:
                label = label_match.group(1).strip().lower()
                if label in WEAK_EDGE_LABELS:
                    weak_edges += 1
                    warnings.append(f"Line {idx}: weak edge label `{label}`. Use a precise verb/action.")
                if len(label) > 60:
                    long_labels += 1
                    warnings.append(f"Line {idx}: long edge label. Consider a short label plus a note block.")

    if edge_count > 25:
        warnings.append(f"Diagram has {edge_count} edges. Consider splitting into multiple focused diagrams.")

    if unlabeled_edges > 0 and unlabeled_edges / max(edge_count, 1) > 0.25:
        warnings.append("More than 25% of edges are unlabeled. Add labels or simplify.")

    if "TODO" in text or "???" in text:
        warnings.append("Found TODO/??? markers. Convert them to explicit assumption notes or resolve them.")

    if not warnings:
        print("OK: no obvious D2 quality issues found.")
        return 0

    print("D2 Doctor warnings:")
    for w in warnings:
        print(f"- {w}")
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
