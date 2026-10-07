"""Exercise the CI installer with fake apt/timeout executables, never host apt."""

import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("ci_apt", Path(__file__).with_name("ci-apt-install.py"))
installer = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(installer)


class InstallTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.apt = self.root / "apt"
        (self.apt / "sources.list.d").mkdir(parents=True)
        self.mirrors = self.apt / "apt-mirrors.txt"
        self.mirrors.write_text("http://azure.archive.ubuntu.com/ubuntu\nhttps://security.ubuntu.com/ubuntu\n")
        self.source = self.apt / "sources.list.d/ubuntu.sources"
        self.source.write_text("Types: deb\nURIs: mirror+file:/etc/apt/apt-mirrors.txt\nSuites: noble noble-updates noble-security\nComponents: main universe\nSigned-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\n")
        (self.apt / "sources.list").write_text("deb https://azure.archive.ubuntu.com/ubuntu noble main\n")
        self.unrelated = self.apt / "sources.list.d/unrelated.list"
        self.unrelated.write_text("deb https://azure.archive.ubuntu.com.example/ubuntu stable main\n")
        self.log = self.root / "commands.jsonl"
        bin_dir = self.root / "bin"
        bin_dir.mkdir()
        timeout = bin_dir / "timeout"
        timeout.write_text("#!/usr/bin/env python3\nimport os,sys,json\nwith open(os.environ['COMMAND_LOG'], 'a') as f: f.write(json.dumps({'timeout':sys.argv[1:3]})+'\\n')\nos.execvp(sys.argv[3],sys.argv[3:])\n")
        timeout.chmod(0o755)
        apt = bin_dir / "apt-get"
        apt.write_text("""#!/usr/bin/env python3
import json, os, sys
from pathlib import Path
args = sys.argv[1:]
phase = 'update' if 'update' in args else 'download' if '--download-only' in args else 'install'
azure = 'http://azure.archive.ubuntu.com/' in Path(os.environ['MIRRORS']).read_text()
with open(os.environ['COMMAND_LOG'], 'a') as f:
    f.write(json.dumps({'phase': phase, 'args': args, 'azure': azure, 'frontend': os.environ.get('DEBIAN_FRONTEND')})+'\\n')
mode = os.environ['APT_SCENARIO']
if mode == 'all_fail' and phase == 'download': sys.exit(100)
if mode == 'update_fail' and phase == 'update': sys.exit(100)
if mode == 'slow_download' and phase == 'download' and azure: sys.exit(124)
if mode == 'slow_update' and phase == 'update' and azure: sys.exit(124)
if mode == 'install_fail' and phase == 'install': sys.exit(100)
""")
        apt.chmod(0o755)
        self.env = {"PATH": f"{bin_dir}:{os.environ['PATH']}", "COMMAND_LOG": str(self.log), "MIRRORS": str(self.mirrors)}

    def run_install(self, scenario, *, no_recommends=False):
        with patch.dict(os.environ, self.env | {"APT_SCENARIO": scenario}):
            result = installer.install(["redis-server", "ffmpeg"], self.apt, no_recommends=no_recommends)
        rows = [json.loads(line) for line in self.log.read_text().splitlines()]
        commands = [row for row in rows if "phase" in row]
        # Every external phase has a deadline + kill grace, including configure.
        deadlines = [row["timeout"] for row in rows if "timeout" in row]
        self.assertEqual(len(deadlines), len(commands))
        for grace, deadline in deadlines:
            self.assertEqual(grace, "--kill-after=10s")
            self.assertGreater(int(deadline.removesuffix("s")), 0)
            self.assertLessEqual(int(deadline.removesuffix("s")), 300)
        for command in commands:
            self.assertEqual(command["frontend"], "noninteractive")
        return result, commands

    def test_success_does_not_change_mirrors_and_installs_only_cached_packages(self):
        original = self.mirrors.read_text()
        result, commands = self.run_install("success")
        self.assertEqual(result, 0)
        self.assertEqual([c["phase"] for c in commands], ["update", "download", "install"])
        self.assertEqual(self.mirrors.read_text(), original)
        self.assertIn("--no-download", commands[-1]["args"])
        self.assertNotIn("--ignore-missing", commands[-1]["args"])
        self.assertNotIn("--no-install-recommends", commands[-1]["args"])
        self.assertEqual(commands[-1]["args"][-2:], ["redis-server", "ffmpeg"])

    def test_recommendation_policy_is_the_same_for_download_and_install(self):
        result, commands = self.run_install("success", no_recommends=True)
        self.assertEqual(result, 0)
        for command in commands[1:]:
            self.assertIn("--no-install-recommends", command["args"])

    def test_slow_download_retries_only_exact_azure_mirror_preserving_security_and_suites(self):
        sources = self.source.read_text()
        unrelated = self.unrelated.read_text()
        result, commands = self.run_install("slow_download")
        self.assertEqual(result, 0)
        self.assertEqual([c["phase"] for c in commands], ["update", "download", "update", "download", "install"])
        self.assertEqual([c["azure"] for c in commands], [True, True, False, False, False])
        self.assertEqual(self.mirrors.read_text(), "https://archive.ubuntu.com/ubuntu\nhttps://security.ubuntu.com/ubuntu\n")
        self.assertEqual((self.apt / "sources.list").read_text(), "deb https://archive.ubuntu.com/ubuntu noble main\n")
        self.assertEqual(self.source.read_text(), sources)
        self.assertEqual(self.unrelated.read_text(), unrelated)

    def test_slow_metadata_falls_back_before_attempting_download(self):
        result, commands = self.run_install("slow_update")
        self.assertEqual(result, 0)
        self.assertEqual([c["phase"] for c in commands], ["update", "update", "download", "install"])
        self.assertIn("APT::Update::Error-Mode=any", commands[0]["args"])

    def test_both_download_attempts_fail_without_installing(self):
        result, commands = self.run_install("all_fail")
        self.assertEqual(result, 100)
        self.assertEqual([c["phase"] for c in commands], ["update", "download", "update", "download"])

    def test_both_metadata_attempts_fail_without_downloading_or_installing(self):
        result, commands = self.run_install("update_fail")
        self.assertEqual(result, 100)
        self.assertEqual([c["phase"] for c in commands], ["update", "update"])

    def test_install_failure_propagates_without_retrying_configuration(self):
        result, commands = self.run_install("install_fail")
        self.assertEqual(result, 100)
        self.assertEqual([c["phase"] for c in commands], ["update", "download", "install"])


if __name__ == "__main__":
    unittest.main()
