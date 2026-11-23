"""Real Apple Silicon integration tests. Run under tools/heavy.sh after model setup."""

import io
import json
import os
import subprocess
import tempfile
import time
import unittest
import urllib.error
import urllib.request
import wave
from array import array
from contextlib import contextmanager
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CLI = ROOT / "bin" / "forge-voice"
LINE = "Take the lantern and follow me across the bridge."


class VoiceStage(unittest.TestCase):
    def duration(self, data):
        with wave.open(io.BytesIO(data)) as w:
            self.assertEqual((w.getnchannels(), w.getsampwidth()), (1, 2))
            seconds = w.getnframes() / w.getframerate()
            samples = array("h", w.readframes(w.getnframes()))
        self.assertGreater(seconds, 1.0)
        self.assertLess(seconds, 12.0)
        self.assertGreater(sum(abs(s) for s in samples) / len(samples), 100)
        return seconds

    def check_timeline(self, timeline, duration):
        cursor = 0
        cues = timeline["cues"]
        self.assertGreater(len({c["value"] for c in cues}), 2)
        for cue in cues:
            self.assertEqual(set(cue), {"start", "end", "value"})
            self.assertAlmostEqual(cursor, cue["start"], places=6)
            self.assertIn(cue["value"], list("ABCDEFGHX"))
            self.assertGreater(cue["end"], cue["start"])
            cursor = cue["end"]
        self.assertAlmostEqual(cursor, duration, places=6)
        self.assertAlmostEqual(timeline["duration_sec"], duration, places=6)
        self.assertEqual(set(timeline), {"duration_sec", "cues"})

    def test_cli_visemes(self):
        self.assertTrue(CLI.is_file(), "forge-voice CLI missing")
        with tempfile.TemporaryDirectory() as tmp:
            wav = Path(tmp) / "gate.wav"
            run = subprocess.run(
                [
                    CLI,
                    "speak",
                    "--voice",
                    "keeper",
                    "--text",
                    "Veldrith, take 2 lanterns and follow me across the bridge.",
                    "--speed",
                    "1.1",
                    "--format",
                    "wav",
                    "--output",
                    wav,
                    "--visemes",
                ],
                check=True,
                capture_output=True,
                text=True,
            )
            events = [json.loads(line) for line in run.stderr.splitlines()]
            self.assertEqual(events[-1]["pct"], 100)
            self.assertTrue(all(e["stage"] == "voice" for e in events))
            duration = self.duration(wav.read_bytes())
            timeline = json.loads(Path(str(wav) + ".visemes.json").read_text())
            self.check_timeline(timeline, duration)
            manifest = json.loads(Path(str(wav) + ".manifest.json").read_text())
            self.assertTrue({"models", "tools", "licences", "timings"} <= set(manifest))

    def test_failure_protocol(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "missing.wav"
            run = subprocess.run(
                [CLI, "speak", "--voice", "invalid", "--text", LINE, "--output", path],
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(run.returncode, 0)
            self.assertIn("error", json.loads(run.stderr.splitlines()[-1]))
            self.assertEqual(list(path.parent.iterdir()), [])

    def test_designed_voice_cancellation(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            # Stand in for a long FFmpeg tempo pass so cancellation hits a real child.
            pidfile = root / "child.pid"
            ffmpeg = root / "ffmpeg"
            ffmpeg.write_text(
                '#!/bin/sh\necho $$ > "$CHILD_PID_FILE"\nexec /bin/sleep 180\n'
            )
            ffmpeg.chmod(0o755)
            out = root / "ash.wav"
            proc = subprocess.Popen(
                [
                    CLI,
                    "speak",
                    "--engine",
                    "qwen",
                    "--voice",
                    "designed",
                    "--description",
                    "A warm low storyteller",
                    "--speed",
                    "0.9",
                    "--text",
                    LINE,
                    "--output",
                    out,
                ],
                env={
                    **os.environ,
                    "CHILD_PID_FILE": str(pidfile),
                    "PATH": tmp + ":" + os.environ["PATH"],
                },
                stderr=subprocess.PIPE,
                text=True,
            )
            try:
                end = time.monotonic() + 90
                while (
                    not pidfile.exists()
                    and time.monotonic() < end
                    and proc.poll() is None
                ):
                    time.sleep(0.1)
                self.assertTrue(pidfile.exists(), "The stage did not reach transcoding")
                child = int(pidfile.read_text())
                proc.terminate()
                _, err = proc.communicate(timeout=10)
                self.assertNotEqual(proc.returncode, 0)
                self.assertEqual(
                    json.loads(err.splitlines()[-1]), {"error": "Cancelled"}
                )
                with self.assertRaises(ProcessLookupError):
                    os.kill(child, 0)
                self.assertFalse(out.exists())
            finally:
                if proc.poll() is None:
                    proc.terminate()
                    proc.wait(timeout=10)

    @contextmanager
    def speech_server(self):
        self.assertTrue(CLI.is_file(), "forge-voice CLI missing")
        server = subprocess.Popen(
            [CLI, "serve", "--port", "0"], stdout=subprocess.PIPE, text=True
        )
        try:
            base = server.stdout.readline().strip()
            self.assertRegex(base, r"^http://127\.0\.0\.1:\d+$")

            yield base
        finally:
            server.terminate()
            try:
                server.wait(timeout=10)
            except subprocess.TimeoutExpired:
                server.kill()
                server.wait()
            server.stdout.close()


    def test_http_visemes(self):
        with self.speech_server() as base:
            def speech(header=None, line=LINE, fmt="wav"):
                headers = {"Content-Type": "application/json"}
                if header is not None:
                    headers["X-Otto-Visemes"] = header
                req = urllib.request.Request(
                    base + "/v1/audio/speech", headers=headers,
                    data=json.dumps({"model": "kokoro", "voice": "keeper", "input": line,
                                     "response_format": fmt, "speed": 1.0}).encode(),
                )
                with urllib.request.urlopen(req, timeout=90) as response:
                    self.assertEqual(response.status, 200)
                    self.assertEqual(response.headers.get_content_type(),
                                     "audio/mpeg" if fmt == "mp3" else "audio/wav")
                    return response.headers.get("X-Otto-Visemes-Id"), response.read()

            def cues(viseme_id):
                with urllib.request.urlopen(base + "/v1/audio/visemes/" + viseme_id,
                                            timeout=10) as response:
                    self.assertEqual(response.headers.get_content_type(), "application/json")
                    return json.load(response)

            def missing(viseme_id):
                with self.assertRaises(urllib.error.HTTPError) as error:
                    cues(viseme_id)
                self.assertEqual(error.exception.code, 404)

            missing("never-issued")
            first, wav = speech("1")
            self.assertTrue(first, "Missing X-Otto-Visemes-Id on aligned audio")
            first_duration = self.duration(wav)
            self.check_timeline(cues(first), first_duration)

            ids = [first]
            for _ in range(31):
                token, _ = speech("1", "Take the lantern.")
                self.assertTrue(token)
                ids.append(token)
            # Ordinary speech and failed alignments must not displace successful ones.
            for header, line in [(None, LINE), ("0", LINE), ("1", "& + @ %")]:
                token, wav = speech(header, line)
                self.assertIsNone(token)
                self.duration(wav)
            self.check_timeline(cues(first), first_duration)
            last, mp3 = speech("1", fmt="mp3")
            self.assertTrue(last)
            self.assertTrue(mp3)
            self.assertEqual(len(set(ids + [last])), 33)
            missing(first)
            for token in ids[1:] + [last]:
                self.assertTrue(cues(token)["cues"])

    def test_otto_driver(self):
        with self.speech_server() as base:
            def get(path):
                with urllib.request.urlopen(base + path, timeout=15) as response:
                    return json.load(response)

            def speech(voice, fmt, model="kokoro", speed=1):
                payload = {
                    "model": model,
                    "input": LINE,
                    "voice": voice,
                    "response_format": fmt,
                    "speed": speed,
                }
                req = urllib.request.Request(
                    base + "/v1/audio/speech",
                    data=json.dumps(payload).encode(),
                    headers={"Content-Type": "application/json"},
                )
                with urllib.request.urlopen(req, timeout=90) as response:
                    return response.headers.get_content_type(), response.read()

            self.assertTrue(get("/health")["ok"])
            ids = {v["id"] for v in get("/v1/audio/voices")["voices"]}
            self.assertTrue({"keeper", "ember", "af_heart"}.issubset(ids))
            for fmt, mime, codec in [
                ("wav", "audio/wav", "pcm_s16le"),
                ("mp3", "audio/mpeg", "mp3"),
                ("aac", "audio/mp4", "aac"),
                ("aiff", "audio/aiff", "pcm_s16be"),
            ]:
                kind, data = speech("keeper", fmt)
                self.assertEqual(kind, mime)
                probe = (
                    subprocess.check_output(
                        [
                            "ffprobe",
                            "-v",
                            "error",
                            "-show_entries",
                            "stream=codec_name",
                            "-of",
                            "csv=p=0",
                            "pipe:0",
                        ],
                        input=data,
                    )
                    .decode()
                    .strip()
                )
                self.assertEqual(probe, codec)
                decoded = subprocess.check_output(
                    ["ffmpeg", "-v", "error", "-i", "pipe:0", "-f", "s16le", "pipe:1"],
                    input=data,
                )
                self.assertGreater(len(decoded), 24000)
                if fmt == "wav":
                    self.duration(data)
            self.duration(speech("ember", "wav")[1])
            slow = self.duration(speech("alloy", "wav", "tts-1", 0.8)[1])
            fast = self.duration(speech("alloy", "wav", "tts-1", 1.3)[1])
            self.assertLess(fast, slow)
            with self.assertRaises(urllib.error.HTTPError) as err:
                speech("made-up", "wav")
            self.assertEqual(err.exception.code, 400)

    def test_designed_over_http(self):
        """Otto's driver with Default model "qwen", and a voice designed from instructions."""
        with self.speech_server() as base:
            with urllib.request.urlopen(base + "/v1/audio/voices", timeout=15) as response:
                listed = {v["id"]: v for v in json.load(response)["voices"]}
            self.assertEqual(listed["keeper"]["gender"], "male")
            self.assertEqual(listed["scout"]["gender"], "female")
            self.assertEqual(listed["bm_george"]["gender"], "male")

            def speech(**payload):
                req = urllib.request.Request(
                    base + "/v1/audio/speech",
                    data=json.dumps({"input": "You say that every night.",
                                     "response_format": "wav", "speed": 1.0, **payload}).encode(),
                    headers={"Content-Type": "application/json", "X-Otto-Visemes": "1"},
                )
                with urllib.request.urlopen(req, timeout=90) as response:
                    return response.headers.get("X-Otto-Visemes-Id"), response.read()

            token, wav = speech(model="qwen", voice="keeper")
            self.duration(wav)
            self.assertTrue(token, "Designed speech should still get mouth cues")
            fox = "Little red fox. Quick, bright and cheeky, like a friend trying to hide a smile."
            _, first = speech(model="kokoro", voice="fox", instructions=fox)
            _, again = speech(model="kokoro", voice="fox", instructions=fox)
            self.duration(first)
            # Same description and line: the same voice and take come back.
            self.assertEqual(first, again)
            self.assertNotEqual(first, speech(model="kokoro", voice="scout")[1])


if __name__ == "__main__":
    unittest.main()
