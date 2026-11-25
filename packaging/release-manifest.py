#!/usr/bin/env python3
"""Release metadata for .github/workflows/release.yml (run from the repo root).

  check-version <tag>            tag v<semver> must equal the three version files; prints it
  notes <version>                the README's "### What's new in <version>" body
  latest-json <v> <sig> <url>    the updater manifest (tauri-plugin-updater format)
  identity                       the release signing identity name
  requirement <identifier>       codesign -R requirement: identifier + accepted leaf certs
"""
from datetime import datetime, timezone
import json
from pathlib import Path
import re
import sys
import tomllib
from urllib.parse import urlparse

DESKTOP = Path("apps/desktop/src-tauri")
SEMVER = re.compile(r"\d+\.\d+\.\d+(?:-[0-9A-Za-z.]+)?")
# The hosts a release asset URL may point at (the app checks the same list).
ASSET_HOSTS = {"github.com"}


def fail(msg):
    sys.exit(f"release-manifest: {msg}")


def check_version(tag):
    if not tag.startswith("v") or not SEMVER.fullmatch(tag[1:]):
        fail(f"tag {tag!r} is not v<semver>")
    want = tag[1:]
    found = {
        "Cargo.toml": tomllib.loads(Path("Cargo.toml").read_text())["workspace"]["package"]["version"],
        f"{DESKTOP}/Cargo.toml": tomllib.loads((DESKTOP / "Cargo.toml").read_text())["package"]["version"],
        f"{DESKTOP}/tauri.conf.json": json.loads((DESKTOP / "tauri.conf.json").read_text())["version"],
    }
    wrong = [f"{f} has {v}" for f, v in found.items() if v != want]
    if wrong:
        fail(f"tag {tag} does not match: " + "; ".join(wrong))
    return want


def notes(version):
    base = version.split("-")[0]
    m = re.search(rf"^### What's new in {re.escape(base)}\n(.*?)(?=^#{{1,3}} |\Z)",
                  Path("README.md").read_text(), re.M | re.S)
    if not m or not m.group(1).strip():
        fail(f"README.md has no \"### What's new in {base}\" section")
    return m.group(1).strip() + "\n"


def latest_json(version, sig_file, url):
    u = urlparse(url)
    if u.scheme != "https" or u.hostname not in ASSET_HOSTS:
        fail(f"asset URL must be https on {sorted(ASSET_HOSTS)}: {url}")
    return json.dumps({
        "version": version,
        "notes": notes(version).strip(),
        "pub_date": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "platforms": {"darwin-aarch64": {"signature": Path(sig_file).read_text().strip(), "url": url}},
    }, indent=2)


def identity():
    return json.loads((DESKTOP / "release-identity.json").read_text())


def requirement(identifier):
    leaves = " or ".join(f'certificate leaf = H"{h}"' for h in identity()["acceptedLeaves"])
    return f'identifier "{identifier}" and ({leaves})'


def main(argv):
    cmd, args = (argv[0], argv[1:]) if argv else ("", [])
    if cmd == "check-version" and len(args) == 1:
        print(check_version(args[0]))
    elif cmd == "notes" and len(args) == 1:
        sys.stdout.write(notes(args[0]))
    elif cmd == "latest-json" and len(args) == 3:
        print(latest_json(*args))
    elif cmd == "identity" and not args:
        print(identity()["signingIdentity"])
    elif cmd == "requirement" and len(args) == 1:
        print(requirement(args[0]))
    else:
        fail(__doc__)


if __name__ == "__main__":
    main(sys.argv[1:])
