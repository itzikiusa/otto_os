#!/usr/bin/env python3
"""Isolated real-Git regression for the production AskPass script; no real tokens.

Run: python3 crates/otto-git/tests/credential_destination.py
All remotes are loopback challenge servers; user Git config is disabled.
"""
import base64
import http.server
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
import unittest


class Challenge(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.server.authorizations.append(self.headers.get("Authorization"))
        if getattr(self.server, "redirect", None):
            self.send_response(302)
            self.send_header("Location", self.server.redirect + self.path)
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        self.send_response(401)
        self.send_header("WWW-Authenticate", 'Basic realm="fixture"')
        self.send_header("Content-Length", "0")
        self.end_headers()

    def log_message(self, *_):
        pass


class CredentialDestination(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="otto-e16-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.servers = []
        for _ in range(2):
            server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Challenge)
            server.authorizations = []
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            self.addCleanup(server.server_close)
            self.addCleanup(server.shutdown)
            self.servers.append(server)
        self.origins = [f"http://127.0.0.1:{s.server_port}" for s in self.servers]
        helper = self.root / "askpass.sh"
        shutil.copyfile(Path(__file__).parents[1] / "src" / "askpass.sh", helper)
        helper.chmod(0o700)
        self.env = {k: v for k, v in os.environ.items() if not k.startswith(("GIT_", "OTTO_GIT_"))}
        self.env.update(HOME=str(self.root), GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL="/dev/null",
                        GIT_TERMINAL_PROMPT="0", GIT_ASKPASS=str(helper), LC_ALL="C",
                        GIT_CONFIG_COUNT="1", GIT_CONFIG_KEY_0="credential.helper", GIT_CONFIG_VALUE_0="",
                        OTTO_GIT_ORIGINS=self.origins[0], OTTO_GIT_TOKEN="E16-dummy",
                        GIT_AUTHOR_NAME="Fixture", GIT_AUTHOR_EMAIL="fixture@example.test",
                        GIT_COMMITTER_NAME="Fixture", GIT_COMMITTER_EMAIL="fixture@example.test")
        self.git("init", "-q", "-b", "main", check=True)
        self.git("-c", "commit.gpgsign=false", "commit", "--allow-empty", "-qm", "fixture", check=True)
        self.git("remote", "add", "origin", self.origins[0] + "/team/app.git", check=True)
        self.git("config", "branch.main.remote", "origin", check=True)
        self.git("config", "branch.main.merge", "refs/heads/main", check=True)

    def git(self, *args, check=False):
        return subprocess.run(["git", *args], cwd=self.root, env=self.env,
                              capture_output=True, text=True, timeout=10, check=check)

    def assert_confined(self, *operation):
        self.git(*operation)
        self.assertTrue(self.servers[1].authorizations, "operation must reach changed destination")
        self.assertFalse(any(self.servers[1].authorizations), "unrelated server received authentication")

    def test_matching_destination_receives_dummy_credential(self):
        self.git("fetch", "--prune")
        expected = "Basic " + base64.b64encode(b"x-token-auth:E16-dummy").decode()
        self.assertIn(expected, self.servers[0].authorizations)

    def test_changed_origin_fetch(self):
        self.git("remote", "set-url", "origin", self.origins[1] + "/team/app.git", check=True)
        self.assert_confined("fetch", "--prune")

    def test_changed_pushurl(self):
        self.git("remote", "set-url", "--push", "origin", self.origins[1] + "/team/app.git", check=True)
        self.assert_confined("push")

    def test_push_remote(self):
        self.git("remote", "add", "other", self.origins[1] + "/team/app.git", check=True)
        self.git("config", "branch.main.pushRemote", "other", check=True)
        self.assert_confined("push")

    def test_instead_of(self):
        self.git("config", f"url.{self.origins[1]}/.insteadOf", self.origins[0] + "/", check=True)
        self.assert_confined("fetch", "--prune")

    def test_push_instead_of(self):
        self.git("config", f"url.{self.origins[1]}/.pushInsteadOf", self.origins[0] + "/", check=True)
        self.assert_confined("push")

    def test_cross_authority_redirect(self):
        self.servers[0].redirect = self.origins[1]
        self.assert_confined("fetch", "--prune")

    def test_same_authority_alternate_remote_remains_usable(self):
        self.git("remote", "add", "other", self.origins[0] + "/team/other.git", check=True)
        self.git("config", "branch.main.pushRemote", "other", check=True)
        self.git("push")
        expected = "Basic " + base64.b64encode(b"x-token-auth:E16-dummy").decode()
        self.assertIn(expected, self.servers[0].authorizations)


if __name__ == "__main__":
    unittest.main(verbosity=2)
