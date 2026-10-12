"""scripts/disk-guard.sh: the 30 GB floor check and the build watchdog.

Floors are pushed to absurd values (0 / 10^6 GB) so the result does not
depend on the machine's real free space. Nothing here deletes anything —
neither may the guard.
"""
import os
import subprocess
import tempfile
import time
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
GUARD = REPO / "scripts/disk-guard.sh"


def guard(*args, floor):
    return subprocess.run(
        ["/bin/bash", str(GUARD), *args],
        env={**os.environ, "DISK_FLOOR_GB": str(floor)},
        capture_output=True, text=True, timeout=60,
    )


class DiskGuard(unittest.TestCase):
    def test_check_passes_above_the_floor_and_refuses_below(self):
        ok = guard("check", "1", "t", floor=0)
        self.assertEqual(ok.returncode, 0, ok.stderr)
        no = guard("check", "1", "t", floor=10**6)
        self.assertEqual(no.returncode, 1)
        self.assertIn("not starting", no.stderr)
        self.assertIn("nothing was deleted", no.stderr)

    def test_disabled_guard_is_a_no_op(self):
        r = subprocess.run(["/bin/bash", str(GUARD), "check", "1"],
                           env={**os.environ, "DISK_GUARD": "0", "DISK_FLOOR_GB": str(10**6)},
                           capture_output=True, text=True, timeout=60)
        self.assertEqual(r.returncode, 0)

    def test_watch_stops_the_build_tree_but_not_its_parent(self):
        with tempfile.TemporaryDirectory() as d:
            marker = Path(d) / "survived"
            # Parent "build script": starts the watchdog on itself, then a
            # long child (the "cargo"); the child must die, the parent must
            # live on to report it.
            script = (
                f'DISK_FLOOR_GB={10**6} bash "{GUARD}" watch $$ t &\n'
                "sleep 30 & S=$!\n"
                "wait $S; echo \"child=$?\"\n"
                f'touch "{marker}"\n'
            )
            start = time.monotonic()
            r = subprocess.run(["/bin/bash", "-c", script], capture_output=True, text=True, timeout=60)
            self.assertLess(time.monotonic() - start, 20, "the watchdog stopped the build")
            self.assertIn("child=143", r.stdout)
            self.assertTrue(marker.exists(), "the parent shell kept running")
            self.assertIn("stopping the build", r.stderr)


if __name__ == "__main__":
    unittest.main()
