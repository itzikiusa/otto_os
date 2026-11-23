"""Offline speech generation and staged WAV/sidecar publication."""

import json
import math
import os
import signal
import subprocess
import tempfile
import textwrap
import time
import wave
from dataclasses import asdict, dataclass
from functools import lru_cache
from importlib.metadata import version
from pathlib import Path

from .fetch_models import MODELS, SOURCES, free_space
from .lipsync import align

CHARACTERS = json.loads(Path(__file__).with_name("characters.json").read_text())
ALIASES = {
    "alloy": "af_alloy",
    "ash": "am_michael",
    "coral": "af_bella",
    "echo": "am_echo",
    "fable": "bm_fable",
    "nova": "af_nova",
    "onyx": "am_onyx",
    "sage": "af_kore",
    "shimmer": "af_sky",
}
FORMATS = {
    "wav": "audio/wav",
    "mp3": "audio/mpeg",
    "aac": "audio/mp4",
    "aiff": "audio/aiff",
}


def stock_voices():
    return sorted(
        p.stem for p in (MODELS / "kokoro" / "voices").glob("[ab][fm]_*.safetensors")
    )


def voices():
    return [
        {
            "id": name,
            "name": name.title(),
            "gender": preset["gender"],
            "preview_text": preset["description"],
        }
        for name, preset in CHARACTERS.items()
    ] + [
        {
            "id": v,
            "name": v,
            "gender": "female" if v[1] == "f" else "male",
            "locale": "en_GB" if v.startswith("b") else "en_US",
        }
        for v in stock_voices()
    ]


@dataclass(frozen=True)
class Speaker:
    engine: str
    voice: str
    speed: float
    description: str


def speaker(voice, engine=None, speed=1.0, description=None):
    if not isinstance(voice, str) or not voice:
        raise ValueError("A voice or character ID is required")
    if (
        type(speed) not in (int, float)
        or not math.isfinite(speed)
        or not 0.5 <= speed <= 2
    ):
        raise ValueError("speed must be a number between 0.5 and 2")
    if description is not None and (
        not isinstance(description, str)
        or not description.strip()
        or len(description) > 1500
    ):
        raise ValueError("description must contain 1–1500 characters")
    requested_description = description
    preset = CHARACTERS.get(voice, {})
    engine = engine or preset.get("engine", "kokoro")
    if engine == "qwen" and (preset or description):
        # A character, or any voice name with a description: Qwen designs it from the words.
        voice = "designed"
    else:
        voice = preset.get("voice", ALIASES.get(voice, voice))
    description = description or preset.get("description", "")
    speed = float(speed) * preset.get("speed", 1.0)
    if engine == "kokoro":
        if requested_description:
            raise ValueError("Text voice design requires --engine qwen")
        if any(v not in stock_voices() for v in voice.split(",")):
            raise ValueError(
                "Unknown Kokoro voice. See /v1/audio/voices for installed speakers."
            )
    elif engine == "qwen":
        if voice != "designed" or not description:
            raise ValueError(
                'Use a designed character or voice "designed" with a description'
            )
    else:
        raise ValueError("engine must be kokoro or qwen")
    return Speaker(engine, voice, speed, description)


@lru_cache(maxsize=2)
def load(engine):
    from mlx_audio.tts.utils import load_model

    path = MODELS / engine
    if not (path / "config.json").is_file():
        raise RuntimeError(
            f"Missing {engine} model. Follow docs/voice.md to install it."
        )
    return load_model(str(path))


def encode(wav, target, fmt, speed=1.0):
    cmd = ["ffmpeg", "-nostdin", "-v", "error", "-y", "-i", str(wav)]
    if speed != 1.0:
        # Factor the tempo into supported intervals, including preset × request extremes.
        tempo = math.sqrt(speed)
        cmd.extend(["-af", f"atempo={tempo},atempo={tempo}"])
    cmd.extend(
        {
            "wav": ["-c:a", "pcm_s16le"],
            "mp3": ["-c:a", "libmp3lame"],
            "aac": ["-c:a", "aac", "-movflags", "+faststart", "-f", "ipod"],
            "aiff": ["-c:a", "pcm_s16be"],
        }[fmt]
    )
    # Keep the stage in its caller's foreground group for Control-C. Isolate
    # the encoder group so SIGTERM directed just at the stage also stops it.
    with subprocess.Popen([*cmd, str(target)], start_new_session=True) as process:
        try:
            if process.wait():
                raise RuntimeError(f"FFmpeg failed to encode {fmt}")
        except BaseException:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait()
            raise


def synthesize(text, selected, wav):
    if not isinstance(text, str) or not text.strip() or len(text) > 10000:
        raise ValueError("input must contain 1–10000 characters")
    free_space()
    import mlx.core as mx
    import numpy as np

    model = load(selected.engine)
    if selected.engine == "kokoro":
        results = model.generate(
            text,
            voice=selected.voice,
            lang_code=selected.voice[0],
            speed=selected.speed,
        )
    else:
        mx.random.seed(24)
        results = (
            segment
            for line in textwrap.wrap(text, width=480, break_on_hyphens=False)
            for segment in model.generate_voice_design(
                text=line, instruct=selected.description, language="english"
            )
        )
    frames = 0
    with wave.open(str(wav), "wb") as out:
        rate = model.sample_rate
        out.setparams((1, 2, rate, 0, "NONE", "not compressed"))
        for segment in results:
            mx.eval(segment.audio)
            samples = np.asarray(segment.audio, dtype=np.float32).squeeze()
            if samples.ndim != 1 or not np.isfinite(samples).all():
                raise RuntimeError("The speech engine returned invalid audio")
            out.writeframes((np.clip(samples, -1, 1) * 32767).astype("<i2").tobytes())
            frames += samples.size
    if frames == 0:
        raise ValueError("No speakable text in input")
    if selected.engine != "kokoro" and selected.speed != 1.0:
        adjusted = wav.with_name("tempo.wav")
        encode(wav, adjusted, "wav", selected.speed)
        adjusted.replace(wav)
    with wave.open(str(wav), "rb") as out:
        return out.getnframes() / out.getframerate()


def write_clip(output, text, selected, visemes, progress):
    started = time.monotonic()
    output = Path(output)
    output.parent.mkdir(parents=True, exist_ok=True)
    if any(
        Path(str(output) + suffix).exists()
        for suffix in ("", ".visemes.json", ".manifest.json")
    ):
        raise FileExistsError(
            f"Output already exists: choose a fresh path for {output}"
        )
    with tempfile.TemporaryDirectory(dir=output.parent) as tmp:
        wav = Path(tmp) / "speech.wav"
        progress(15, "Synthesizing dialogue")
        duration = synthesize(text, selected, wav)
        rendered = time.monotonic()
        targets = {}
        if visemes:
            progress(75, "Aligning phonemes to the final waveform")
            timeline = align(wav, text, duration)
            cues = Path(tmp) / "visemes.json"
            cues.write_text(json.dumps(timeline, indent=2) + "\n")
            targets[cues] = Path(str(output) + ".visemes.json")
        repo, revision = SOURCES[selected.engine]
        manifest = Path(tmp) / "manifest.json"
        tools = {"mlx-audio": version("mlx-audio"), "mlx": version("mlx")}
        licences = {"mlx-audio": "MIT", "mlx": "MIT", repo: "Apache-2.0"}
        models = [{"id": repo, "revision": revision}]
        if selected.engine == "kokoro":
            tools["misaki"] = version("misaki")
            licences.update(
                {
                    "misaki": "Apache-2.0",
                    "en_core_web_sm": "MIT",
                    "phonemizer-fork": "GPL-3.0-or-later",
                    "eSpeak NG": "GPL-3.0-or-later",
                }
            )
            models.append(
                {"id": "en_core_web_sm", "version": version("en-core-web-sm")}
            )
        elif selected.speed != 1:
            tools["ffmpeg"] = subprocess.check_output(
                ["ffmpeg", "-version"], text=True
            ).splitlines()[0]
            licences["ffmpeg"] = "System executable; licence depends on installed build"
        timings = {"synthesis_sec": rendered - started}
        if visemes:
            tools["pocketsphinx"] = version("pocketsphinx")
            models.append(
                {
                    "id": "PocketSphinx US English acoustic model",
                    "version": version("pocketsphinx"),
                }
            )
            licences["PocketSphinx US English acoustic model"] = (
                "BSD-2-Clause (Alpha Cephei)"
            )
            licences["PocketSphinx"] = "BSD-2-Clause (CMU) with bundled notices"
            timings["alignment_sec"] = time.monotonic() - rendered
        progress(95, "Writing clip and provenance")
        manifest.write_text(
            json.dumps(
                dict(
                    tools=tools,
                    models=models,
                    stage="voice",
                    **({"seed": 24} if selected.engine == "qwen" else {}),
                    output=str(output),
                    licences=licences,
                    timings=timings,
                    speaker=asdict(selected),
                    duration_sec=duration,
                ),
                indent=2,
            )
            + "\n"
        )
        targets[manifest] = Path(str(output) + ".manifest.json")
        targets[wav] = output
        linked = []
        try:
            for src, dest in targets.items():
                os.link(src, dest)
                linked.append(dest)
        except BaseException:
            for dest in linked:
                dest.unlink()
            raise
    return duration
