"""Guard G3: every workflows-overhaul requirement ID has a test (test-plan §2.12).

docs/testing/workflows-matrix.txt lists the IDs of the test plan's
requirements-to-test matrix. A test proves one with a "proves: <ID>" comment
under crates/, ui/ or scripts/. An ID without a proof must be marked pending;
a pending ID that has one fails (drop the marker), and so does a proof naming
an ID the list does not hold (a typo proves nothing).
"""

import re
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MATRIX = ROOT / "docs/testing/workflows-matrix.txt"
SEARCH = ("crates", "ui", "scripts")
SKIP_DIRS = {"node_modules", "target", "dist", ".git", "test-results", "playwright-report"}
SUFFIXES = {".rs", ".ts", ".svelte", ".js", ".mjs", ".py"}
ID = r"[A-Z][0-9]+(?:\.[0-9a-z]+)?"
PROVES = re.compile(r"proves:\s*(" + ID + r"(?:\s*,\s*" + ID + r")*)")


def read_matrix(path: Path) -> dict[str, bool]:
    """ID -> pending."""
    out: dict[str, bool] = {}
    for line in path.read_text().splitlines():
        line = line.split("#", 1)[0].strip()
        if line:
            parts = line.split()
            out[parts[0]] = parts[1:] == ["pending"]
    return out


def proofs(root: Path, dirs=SEARCH) -> dict[str, list[str]]:
    """ID -> files whose comments say they prove it."""
    found: dict[str, list[str]] = {}
    for top in dirs:
        stack = [root / top]
        while stack:
            d = stack.pop()
            if not d.is_dir():
                continue
            for p in d.iterdir():
                if p.is_dir():
                    if p.name not in SKIP_DIRS:
                        stack.append(p)
                elif p.suffix in SUFFIXES and p.name != Path(__file__).name:
                    for m in PROVES.finditer(p.read_text(errors="ignore")):
                        for i in re.split(r"\s*,\s*", m.group(1)):
                            found.setdefault(i, []).append(str(p.relative_to(root)))
    return found


def problems(matrix: dict[str, bool], found: dict[str, list[str]]) -> list[str]:
    out = [f"{i}: no test says 'proves: {i}' (mark it pending until one does)"
           for i, pending in matrix.items() if not pending and i not in found]
    out += [f"{i}: marked pending but proven by {found[i][0]} — drop the marker"
            for i, pending in matrix.items() if pending and i in found]
    out += [f"{i}: proven in {files[0]} but not in {MATRIX.name}" for i, files in found.items() if i not in matrix]
    return out


class WorkflowMatrix(unittest.TestCase):
    def test_every_matrix_id_has_a_test_or_is_pending(self):
        matrix = read_matrix(MATRIX)
        self.assertGreater(len(matrix), 100)
        self.assertEqual(problems(matrix, proofs(ROOT)), [])

    def test_the_guard_reports_each_kind_of_gap(self):
        with tempfile.TemporaryDirectory() as tmp:
            src = Path(tmp) / "crates"
            src.mkdir()
            (src / "a.rs").write_text("/// proves: C1, C2 — files\n/// proves: Z9\n")
            matrix = {"C1": False, "C2": True, "C3": False, "C4": True}
            got = problems(matrix, proofs(Path(tmp), ("crates",)))
        self.assertEqual(sorted(g.split(":")[0] for g in got), ["C2", "C3", "Z9"])


if __name__ == "__main__":
    unittest.main()
