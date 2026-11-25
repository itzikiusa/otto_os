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

    def test_sign_job_runs_no_fetched_code_and_pins_actions(self):
        wf = (REPO / ".github/workflows/release.yml").read_text()
        sign = wf.split("\n  sign:\n")[1]
        self.assertNotIn("npx", sign)
        self.assertNotIn("setup-node", sign)
        self.assertNotIn("npx", (REPO / "packaging/release-artifacts.sh").read_text())
        self.assertIn("npm ci --ignore-scripts", sign)
        for uses in re.findall(r"uses: (\S+)", wf):
            self.assertRegex(uses, r"@[0-9a-f]{40}$", uses)
        # The signer comes from a committed lockfile, pinned to one version.
        lock = json.loads((REPO / "packaging/signer/package-lock.json").read_text())
        self.assertTrue(all("integrity" in p for k, p in lock["packages"].items() if k))

    def test_release_workflow_gates_tag_ci_and_cleans_secrets(self):
        wf = (REPO / ".github/workflows/release.yml").read_text()
        gate, rest = wf.split("\n  build:\n")
        self.assertIn("--branch main --event push", gate)
        self.assertIn("git/ref/tags/$TAG", gate)
        self.assertIn(r"^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$", gate)
        build, sign = rest.split("\n  sign:\n")
        self.assertIn("check-release-config", build)
        self.assertIn("check-release-config", sign)
        self.assertIn("umask 077", sign)
        cleanup = sign.split("if: always()")[1]
        self.assertIn("release.p12", cleanup.split("- name:")[0])
        # The keychain is gone before the update key is used.
        self.assertLess(sign.index("delete-keychain"), sign.index("TAURI_SIGNING_PRIVATE_KEY"))

    def test_release_workflow_keeps_secrets_in_the_approved_sign_job(self):
        wf = (REPO / ".github/workflows/release.yml").read_text()
        before, sign = wf.split("\n  sign:\n")
        self.assertNotIn("secrets.", before, "the build job runs npm/cargo code: no secrets there")
        self.assertIn("environment: release", sign)
        self.assertNotIn("environment:", before)
        # One Tauri CLI, pinned exactly in packaging/signer (--app-version needs >= 2.12).
        self.assertNotIn("@tauri-apps/cli@", wf)
        pkg = json.loads((REPO / "packaging/signer/package.json").read_text())
        self.assertRegex(pkg["dependencies"]["@tauri-apps/cli"], r"^2\.(1[2-9]|[2-9]\d)\.\d+$")

    def release_config(self, root, test=False, pubkey_id="0123456789ABCDEF"):
        import base64
        pub = base64.b64encode(f"untrusted comment: minisign public key: {pubkey_id}\nRWQ=\n".encode()).decode()
        (root / "apps/desktop/src-tauri/tauri.conf.json").write_text(json.dumps(
            {"version": "0.5.4", "plugins": {"updater": {"pubkey": pub}}}))
        (root / "apps/desktop/src-tauri/release-identity.json").write_text(json.dumps(
            {"test": test, "signingIdentity": "Otto Release Signing", "acceptedLeaves": ["aa" * 20]}))

    def test_release_config_with_real_values_passes(self):
        with tempfile.TemporaryDirectory() as d:
            self.tree(Path(d))
            self.release_config(Path(d))
            r = run("check-release-config", cwd=d)
            self.assertEqual(r.returncode, 0, r.stderr)

    def test_a_test_identity_can_never_be_released(self):
        with tempfile.TemporaryDirectory() as d:
            self.tree(Path(d))
            self.release_config(Path(d), test=True)
            r = run("check-release-config", cwd=d)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("TEST", r.stderr)

    def test_the_test_update_key_can_never_be_released(self):
        with tempfile.TemporaryDirectory() as d:
            self.tree(Path(d))
            self.release_config(Path(d), pubkey_id="287DEA2ED05D88F2")
            r = run("check-release-config", cwd=d)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("TEST update key", r.stderr)

    def test_the_test_update_key_payload_is_refused_under_any_comment(self):
        import base64
        payload = "RWTyiF3QLup9KFyJGN8hznaMtX/zcuCEU/GZgVSIqZ5XDDir1/SNLm8G"
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            self.tree(root)
            for text in (f"untrusted comment: minisign public key: 0123456789ABCDEF\n{payload}\n",
                         f"untrusted comment: my release key\r\n {payload} \r\n"):
                self.release_config(root)
                conf = root / "apps/desktop/src-tauri/tauri.conf.json"
                c = json.loads(conf.read_text())
                c["plugins"]["updater"]["pubkey"] = base64.b64encode(text.encode()).decode()
                conf.write_text(json.dumps(c))
                r = run("check-release-config", cwd=d)
                self.assertNotEqual(r.returncode, 0, text)
                self.assertIn("TEST update key", r.stderr)

    def test_the_test_key_id_is_refused_with_stray_whitespace(self):
        with tempfile.TemporaryDirectory() as d:
            self.tree(Path(d))
            self.release_config(Path(d), pubkey_id="287DEA2ED05D88F2 \r")
            self.assertNotEqual(run("check-release-config", cwd=d).returncode, 0)

    def test_the_test_leaf_is_refused_even_when_marked_not_test(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            self.tree(root)
            self.release_config(root)
            (root / "apps/desktop/src-tauri/release-identity.json").write_text(json.dumps(
                {"test": False, "signingIdentity": "Otto Release Signing",
                 "acceptedLeaves": ["aa" * 20, "E2FD4736E55446F39C4151E258A6F260B1AE40CC"]}))
            r = run("check-release-config", cwd=d)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("TEST certificate", r.stderr)

    def test_the_repo_config_today_is_refused(self):
        # Until the owner's real identity and key replace the TEST ones.
        r = run("check-release-config", cwd=REPO)
        self.assertNotEqual(r.returncode, 0)

    def test_release_artifacts_refuses_a_test_config_before_signing_anything(self):
        r = subprocess.run(["bash", str(REPO / "packaging/release-artifacts.sh"), "bundle",
                            "/nonexistent/Otto.app", "0.5.4", "/nonexistent/out"],
                           capture_output=True, text=True)
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("release-manifest:", r.stderr)
        self.assertNotIn("codesign", r.stderr)

    def test_the_repo_identity_file_is_well_formed(self):
        ident = json.loads((REPO / "apps/desktop/src-tauri/release-identity.json").read_text())
        self.assertTrue(ident["signingIdentity"])
        self.assertTrue(ident["acceptedLeaves"])
        for leaf in ident["acceptedLeaves"]:
            self.assertRegex(leaf, r"^[0-9a-f]{40}$")


if __name__ == "__main__":
    unittest.main()
