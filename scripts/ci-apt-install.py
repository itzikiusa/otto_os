#!/usr/bin/env python3
"""Install CI's Ubuntu dependencies with bounded acquisition and one fallback.

The hosted runner's Azure mirror can drip-feed downloads indefinitely. Bound
metadata and download-only phases separately so retrying never interrupts dpkg.
Only after acquiring every package do we run a bounded, offline installation.
Run with sudo; this modifies only the disposable runner's apt configuration.
"""

import argparse
import os
from pathlib import Path
import re
import subprocess
import sys


def fallback_mirror(apt_dir: Path) -> None:
    """Replace the failing Azure endpoint, preserving suites and signing keys."""
    paths = [apt_dir / "sources.list", apt_dir / "apt-mirrors.txt"]
    paths.extend((apt_dir / "sources.list.d").glob("*.list"))
    paths.extend((apt_dir / "sources.list.d").glob("*.sources"))
    for path in paths:
        if not path.is_file():
            continue
        original = path.read_text()
        changed = re.sub(r"https?://azure\.archive\.ubuntu\.com(?=/)", "https://archive.ubuntu.com", original)
        if changed != original:
            path.write_text(changed)
            print(f"Using Ubuntu's official archive in {path}", flush=True)


def install(packages: list[str], apt_dir: Path = Path("/etc/apt"), *, no_recommends: bool = False) -> int:
    env = dict(os.environ, DEBIAN_FRONTEND="noninteractive")
    options = ["-o", "Acquire::http::Timeout=30", "-o", "Acquire::https::Timeout=30", "-o", "Acquire::Retries=1"]
    install_options = ["install", "-y", *(["--no-install-recommends"] if no_recommends else [])]

    def run(seconds: int, *args: str) -> int:
        # GNU timeout kills the entire apt process group after the grace period.
        # Unlike Acquire::*::Timeout this also bounds slow, continuous transfers.
        command = ["timeout", "--kill-after=10s", f"{seconds}s", "apt-get", *options, *args]
        print("+ " + " ".join(command), flush=True)
        return subprocess.run(command, env=env, check=False).returncode

    for attempt in range(2):
        status = run(180, "-o", "APT::Update::Error-Mode=any", "update")
        if status == 0:
            status = run(300, *install_options, "--download-only", *packages)
        if status == 0:
            # Never hide missing packages, or start another network acquisition
            # while configuring. A failed install is terminal, not retried.
            return run(300, *install_options, "--no-download", *packages)
        if attempt == 0:
            print(f"::warning::apt acquisition exited {status}; retrying once with the official Ubuntu archive", flush=True)
            fallback_mirror(apt_dir)
    print(f"::error::apt dependencies unavailable after both acquisition attempts (exit {status})", flush=True)
    return status


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--no-install-recommends", action="store_true")
    parser.add_argument("packages", nargs="+")
    args = parser.parse_args()
    sys.exit(install(args.packages, no_recommends=args.no_install_recommends))
