import argparse
import json
import os
import signal
import subprocess
import sys
import tempfile
from collections import OrderedDict
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
from uuid import uuid4

from .engine import CHARACTERS, FORMATS, encode, speaker, synthesize, voices, write_clip
from .lipsync import align


def emit(**event):
    print(json.dumps(event), file=events, flush=True)


def progress(pct, message):
    emit(stage="voice", pct=pct, message=message)


class Parser(argparse.ArgumentParser):
    def error(self, message):
        raise ValueError(message)


class SpeechHandler(BaseHTTPRequestHandler):
    def log_message(self, format, *args):
        pass

    def reply(self, status, content, mime="application/json", visemes_id=None):
        data = json.dumps(content).encode() if mime == "application/json" else content
        self.send_response(status)
        self.send_header("Content-Type", mime)
        self.send_header("Content-Length", str(len(data)))
        if visemes_id is not None:
            self.send_header("X-Otto-Visemes-Id", visemes_id)
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        if self.path == "/health":
            return self.reply(200, {"ok": True})
        if self.path in ("/v1/audio/voices", "/audio/voices"):
            return self.reply(200, {"voices": voices()})
        if self.path.startswith("/v1/audio/visemes/"):
            token = self.path.removeprefix("/v1/audio/visemes/")
            timeline = self.server.timelines.get(token)
            if timeline is None:
                return self.reply(404, {"error": "Unknown viseme ID"})
            return self.reply(200, timeline)
        self.reply(404, {"error": "Unknown route"})

    def do_POST(self):
        if self.path not in ("/v1/audio/speech", "/audio/speech"):
            return self.reply(404, {"error": "Unknown route"})
        port = self.server.server_port
        if self.headers.get("Host") not in (f"127.0.0.1:{port}", f"localhost:{port}"):
            return self.reply(403, {"error": "Use the local endpoint"})
        if self.headers.get_content_type() != "application/json":
            return self.reply(415, {"error": "Send application/json"})
        try:
            size = int(self.headers.get("Content-Length", 0))
            if not 0 < size <= 160_000:
                return self.reply(
                    413, {"error": "Speech request too large or missing length"}
                )
            body = json.loads(self.rfile.read(size))
            if not isinstance(body, dict):
                raise TypeError("Expected a JSON object")
            if any(key in body for key in ("ref_audio", "reference_audio", "ref_text")):
                raise ValueError("Voice design accepts a text description only")
            model = body.get("model", "tts-1")
            if model not in ("kokoro", "tts-1", "tts-1-hd", "qwen"):
                raise ValueError("Unknown speech model")
            fmt = body.get("response_format", "mp3")
            if not isinstance(fmt, str) or fmt not in FORMATS:
                raise ValueError(
                    "Supported response_format values: wav, mp3, aac, aiff"
                )
            voice = body.get("voice", "alloy")
            instructions = body.get("instructions")
            # Model "qwen" designs every character from its description; a stock
            # Kokoro ID stays Kokoro. Instructions design the named voice from them.
            designed = instructions is not None or (
                model == "qwen" and (voice in CHARACTERS or voice == "designed")
            )
            selected = speaker(
                voice, "qwen" if designed else None, body.get("speed", 1.0), instructions
            )
            with tempfile.TemporaryDirectory() as tmp:
                wav = Path(tmp) / "line.wav"
                text = body.get("input")
                duration = synthesize(text, selected, wav)
                result = wav
                if fmt != "wav":
                    result = wav.with_suffix("." + fmt)
                    encode(wav, result, fmt)
                if result.stat().st_size > 25 * 1024**2:
                    return self.reply(
                        413,
                        {
                            "error": "Rendered audio exceeds Otto’s 25 MiB limit. Split the text."
                        },
                    )
                data = result.read_bytes()
                visemes_id = None
                if self.path == "/v1/audio/speech" and self.headers.get("X-Otto-Visemes") == "1":
                    try:
                        timeline = align(wav, text, duration)
                    except Exception:
                        # Alignment is optional. The encoded audio is already complete.
                        pass
                    else:
                        visemes_id = uuid4().hex
                        self.server.timelines[visemes_id] = timeline
                        if len(self.server.timelines) > 32:
                            self.server.timelines.popitem(last=False)
        except (TypeError, ValueError) as e:
            return self.reply(400, {"error": str(e)})
        except (RuntimeError, OSError, subprocess.SubprocessError) as e:
            return self.reply(500, {"error": str(e)})
        self.reply(200, data, FORMATS[fmt], visemes_id)


def main():
    parser = Parser(prog="forge-voice")
    commands = parser.add_subparsers(dest="command", required=True)
    speak = commands.add_parser("speak")
    speak.add_argument("--text", required=True)
    speak.add_argument("--voice", default="af_heart")
    speak.add_argument("--engine", choices=["kokoro", "qwen"])
    speak.add_argument(
        "--description", help="Describe a fictional voice for VoiceDesign."
    )
    speak.add_argument("--speed", type=float, default=1.0)
    speak.add_argument("--format", choices=["wav"], default="wav")
    speak.add_argument("--visemes", action="store_true")
    speak.add_argument("--output", required=True, type=Path)
    serve = commands.add_parser("serve")
    serve.add_argument("--port", type=int, default=7811)
    args = parser.parse_args()
    try:
        if args.command == "serve":
            with HTTPServer(("127.0.0.1", args.port), SpeechHandler) as server:
                server.timelines = OrderedDict()
                progress(100, "Listening on loopback")
                print(f"http://127.0.0.1:{server.server_port}", flush=True)
                server.serve_forever()
        else:
            selected = speaker(args.voice, args.engine, args.speed, args.description)
            progress(0, "Preparing voice")
            duration = write_clip(
                args.output, args.text, selected, args.visemes, progress
            )
            progress(100, f"Wrote {args.output} ({duration:.3f} s)")
    except BrokenPipeError:
        raise RuntimeError("Output consumer disconnected") from None


def cancelled(signum, frame):
    raise KeyboardInterrupt


if __name__ == "__main__":
    events = os.fdopen(os.dup(2), "w")
    # Torch, tokenizers and FFmpeg can all write non-JSON diagnostics to fd 2.
    with open(os.devnull, "w") as quiet:
        os.dup2(quiet.fileno(), 2)
    for sig in (signal.SIGTERM, signal.SIGINT):
        signal.signal(sig, cancelled)
    try:
        main()
    except KeyboardInterrupt:
        emit(error="Cancelled")
        sys.exit(130)
    except Exception as exc:  # noqa: BLE001 — public CLI failures are JSONL
        emit(error=str(exc) or type(exc).__name__)
        sys.exit(1)
