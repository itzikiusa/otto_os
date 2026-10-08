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


def prepare_external_installer(apt_dir: Path = Path("/etc/apt")) -> None:
    """Bound apt used inside tools such as Playwright, without invoking dpkg.

    Those tools own their package list. Use the known-good fallback immediately
    and persist acquisition options for their sudo apt subprocess. The caller
    must also bound the complete tool invocation to stop continuous slow reads.
    """
    fallback_mirror(apt_dir)
    settings = apt_dir / "apt.conf.d/99-otto-ci-acquisition"
    settings.parent.mkdir(parents=True, exist_ok=True)
    settings.write_text(
        'Acquire::http::Timeout "30";\n'
        'Acquire::https::Timeout "30";\n'
        'Acquire::Retries "1";\n'
        'APT::Update::Error-Mode "any";\n'
    )


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
    parser.add_argument("--prepare-external", action="store_true", help="prepare apt for a bounded external installer, without installing packages")
    parser.add_argument("packages", nargs="*")
    args = parser.parse_args()
    if args.prepare_external:
        if args.packages or args.no_install_recommends:
            parser.error("--prepare-external does not take packages or install options")
        prepare_external_installer()
        sys.exit(0)
    if not args.packages:
        parser.error("at least one package is required")
    sys.exit(install(args.packages, no_recommends=args.no_install_recommends))
