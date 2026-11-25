"""release-manifest.py: version gate, README notes, latest.json (no network, no signing)."""
import json
from pathlib import Path
import re
import subprocess
import tempfile
import unittest

REPO = Path(__file__).resolve().parents[2]
SCRIPT = REPO / "packaging/release-manifest.py"

README = """# Otto

### What's new in 0.5.4

- **Updates.** Otto checks for a new release.

### What's new in 0.5.3

- Older notes.
"""


def run(*args, cwd):
    return subprocess.run(["python3", "-I", str(SCRIPT), *args], cwd=cwd, capture_output=True, text=True)


class ReleaseManifest(unittest.TestCase):
    def tree(self, root, version="0.5.4", desktop=None, conf=None):
        (root / "apps/desktop/src-tauri").mkdir(parents=True)
        (root / "Cargo.toml").write_text(f'[workspace]\nmembers = []\n\n[workspace.package]\nversion = "{version}"\n')
        (root / "apps/desktop/src-tauri/Cargo.toml").write_text(f'[package]\nname = "otto-desktop"\nversion = "{desktop or version}"\n')
        (root / "apps/desktop/src-tauri/tauri.conf.json").write_text(json.dumps({"version": conf or version}))
        (root / "README.md").write_text(README)

    def test_version_gate_accepts_matching_tag_and_prints_version(self):
        with tempfile.TemporaryDirectory() as d:
            self.tree(Path(d))
            r = run("check-version", "v0.5.4", cwd=d)
            self.assertEqual(r.returncode, 0, r.stderr)
            self.assertEqual(r.stdout.strip(), "0.5.4")

    def test_version_gate_names_every_file_that_disagrees(self):
        with tempfile.TemporaryDirectory() as d:
            self.tree(Path(d), desktop="0.5.3", conf="0.5.2")
            r = run("check-version", "v0.5.4", cwd=d)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("apps/desktop/src-tauri/Cargo.toml has 0.5.3", r.stderr)
            self.assertIn("apps/desktop/src-tauri/tauri.conf.json has 0.5.2", r.stderr)

    def test_version_gate_rejects_a_tag_that_is_not_semver(self):
        with tempfile.TemporaryDirectory() as d:
            self.tree(Path(d))
            for tag in ("0.5.4", "v0.5", "latest", "v2026.11.1-6x"):
                self.assertNotEqual(run("check-version", tag, cwd=d).returncode, 0, tag)

    def test_prerelease_tag_matches_its_full_version(self):
        with tempfile.TemporaryDirectory() as d:
            self.tree(Path(d), version="0.5.4-rc.1")
            r = run("check-version", "v0.5.4-rc.1", cwd=d)
            self.assertEqual((r.returncode, r.stdout.strip()), (0, "0.5.4-rc.1"), r.stderr)

    def test_notes_are_the_readme_section_of_that_version_only(self):
        with tempfile.TemporaryDirectory() as d:
            self.tree(Path(d))
            r = run("notes", "0.5.4", cwd=d)
            self.assertEqual(r.returncode, 0, r.stderr)
            self.assertEqual(r.stdout, "- **Updates.** Otto checks for a new release.\n")

    def test_prerelease_notes_come_from_the_base_version(self):
        with tempfile.TemporaryDirectory() as d:
            self.tree(Path(d))
            self.assertIn("Otto checks", run("notes", "0.5.4-rc.2", cwd=d).stdout)

    def test_missing_notes_fail(self):
        with tempfile.TemporaryDirectory() as d:
            self.tree(Path(d))
            r = run("notes", "0.9.0", cwd=d)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("What's new in 0.9.0", r.stderr)

    def test_latest_json_has_the_updater_schema(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            self.tree(root)
            (root / "a.sig").write_text("dW50cnVzdGVk\n")
            r = run("latest-json", "0.5.4", "a.sig",
                    "https://github.com/o/r/releases/download/v0.5.4/Otto_0.5.4_aarch64.app.tar.gz", cwd=d)
            self.assertEqual(r.returncode, 0, r.stderr)
            m = json.loads(r.stdout)
            self.assertEqual(sorted(m), ["notes", "platforms", "pub_date", "version"])
            self.assertEqual(m["version"], "0.5.4")
            self.assertEqual(m["notes"], "- **Updates.** Otto checks for a new release.")
            self.assertRegex(m["pub_date"], r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$")
            self.assertEqual(m["platforms"], {"darwin-aarch64": {
                "signature": "dW50cnVzdGVk",
                "url": "https://github.com/o/r/releases/download/v0.5.4/Otto_0.5.4_aarch64.app.tar.gz"}})

    def test_latest_json_refuses_a_non_github_https_url(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            self.tree(root)
            (root / "a.sig").write_text("x")
            for url in ("http://github.com/o/r/a.tar.gz", "https://example.com/a.tar.gz"):
                self.assertNotEqual(run("latest-json", "0.5.4", "a.sig", url, cwd=d).returncode, 0, url)

    def test_requirement_per_binary_from_the_identity_file(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            self.tree(root)
            (root / "apps/desktop/src-tauri/release-identity.json").write_text(json.dumps(
                {"signingIdentity": "Otto Release Signing", "acceptedLeaves": ["aa11", "bb22"]}))
            r = run("requirement", "ottod", cwd=d)
            self.assertEqual(r.stdout.strip(),
                             'identifier "ottod" and (certificate leaf = H"aa11" or certificate leaf = H"bb22")')
            self.assertEqual(run("identity", cwd=d).stdout.strip(), "Otto Release Signing")

    def test_release_workflow_keeps_secrets_in_the_approved_sign_job(self):
        wf = (REPO / ".github/workflows/release.yml").read_text()
        before, sign = wf.split("\n  sign:\n")
        self.assertNotIn("secrets.", before, "the build job runs npm/cargo code: no secrets there")
        self.assertIn("environment: release", sign)
        self.assertNotIn("environment:", before)
        # One pinned Tauri CLI everywhere (--app-version needs >= 2.12).
        pins = set(re.findall(r"@tauri-apps/cli@([0-9.]+)", wf + (REPO / "packaging/release-artifacts.sh").read_text()))
        self.assertEqual(len(pins), 1, pins)

    def test_the_repo_identity_file_is_well_formed(self):
        ident = json.loads((REPO / "apps/desktop/src-tauri/release-identity.json").read_text())
        self.assertTrue(ident["signingIdentity"])
        self.assertTrue(ident["acceptedLeaves"])
        for leaf in ident["acceptedLeaves"]:
            self.assertRegex(leaf, r"^[0-9a-f]{40}$")


if __name__ == "__main__":
    unittest.main()
