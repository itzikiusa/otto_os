"""Sound-effect, ambience and music generation, loop building, loudness and publication."""

import json
import math
import os
import resource
import shutil
import time
from functools import lru_cache
from importlib.metadata import version

import numpy as np

from . import analysis
from .fetch_models import MODELS, SOURCES, free_space

SR = 48000
CEILING_DBTP = -1.0
# Loudness targets. One-shot effects are peak-normalised instead: integrated loudness of a
# short transient is not a meaningful level, and game mixers set per-event volume.
TARGET_LUFS = {"ambience": -18.0, "music": -16.0}
LICENCES = {
    "sfx": {"model": "Apache-2.0", "runtime": {"moss-sfx-mlx": "Apache-2.0", "mlx": "MIT", "mlx-lm": "MIT"}},
    "music": {"model": "MIT", "runtime": {"ace-step": "MIT", "torch": "BSD-3-Clause", "mlx": "MIT",
                                          "transformers": "Apache-2.0", "diffusers": "Apache-2.0"}},
}
MODEL_KEY = {"sfx": "moss-sfx", "music": "ace-step"}
MOSS_MAX_SEC = 30.0
ACE_MAX_SEC = 600.0
ACE_MIN_SEC = 10.0


def check(cond, message):
    if not cond:
        raise ValueError(message)


@lru_cache(maxsize=1)
def moss():
    from moss_sfx_mlx.pipeline_mlx import MossSoundEffectPipeline

    path = MODELS / "moss-sfx"
    if not (path / "mlx" / "dit.safetensors").is_file():
        raise RuntimeError("Missing MOSS-SoundEffect weights. See docs/audio.md to install them.")
    return MossSoundEffectPipeline.from_pretrained(str(path))


@lru_cache(maxsize=1)
def ace():
    from acestep.handler import AceStepHandler

    path = MODELS / "ace-step"
    if not (path / "acestep-v15-turbo" / "model.safetensors").is_file():
        raise RuntimeError("Missing ACE-Step weights. See docs/audio.md to install them.")
    os.environ["ACESTEP_CHECKPOINTS_DIR"] = str(path)
    handler = AceStepHandler()
    # Forge ships the DiT-only path. The upstream precheck also requires the optional
    # planner LM and would try to download it, so skip it; the files are verified above.
    handler._ensure_models_present = lambda **_: None
    status, ok = handler.initialize_service(project_root=str(path), config_path="acestep-v15-turbo", device="auto")
    if not ok:
        raise RuntimeError(status.strip().splitlines()[0])
    return handler


def sfx_raw(prompt, seconds, seed, progress):
    import mlx.core as mx

    def steps(it):
        it = list(it)
        for i in it:
            if i % 10 == 0:
                progress(i / len(it))
            yield i

    audio = moss()(prompt=prompt, seconds=seconds, seed=seed, progress_bar_cmd=steps)
    return np.asarray(audio.astype(mx.float32))[0].T  # (samples, 1)


def music_raw(prompt, seconds, seed, bpm, lyrics, work, progress):
    from acestep.inference import GenerationConfig, GenerationParams, generate_music

    params = GenerationParams(
        caption=prompt, lyrics=lyrics or "[Instrumental]", instrumental=not lyrics, bpm=bpm,
        duration=seconds, seed=seed, inference_steps=8, shift=3.0, thinking=False,
        use_cot_metas=False, use_cot_caption=False, use_cot_language=False, enable_normalization=False,
    )
    config = GenerationConfig(batch_size=1, use_random_seed=False, seeds=[seed], audio_format="wav")
    result = generate_music(ace(), None, params, config, save_dir=str(work),
                            progress=lambda value, desc=None, **_: progress(float(value or 0)))
    if not result.success:
        raise RuntimeError(f"ACE-Step failed: {result.error or result.status_message}")
    audio = result.audios[0]
    if audio["sample_rate"] != SR:
        raise RuntimeError(f"ACE-Step returned {audio['sample_rate']} Hz, expected {SR}")
    return audio["tensor"].float().numpy().T.astype(np.float32)  # (samples, 2)


def highpass(x):
    """20 Hz zero-phase high-pass: the SFX decoder can emit sub-audio drift that only eats headroom."""
    from scipy.signal import butter, sosfiltfilt

    return sosfiltfilt(butter(4, 20, "highpass", fs=SR, output="sos"), x, axis=0).astype(np.float32)


def best_period(x, n, beat):
    """Loop length within half a beat of n samples whose crossfade lines up best (onset envelope)."""
    hop = 512
    _, power = analysis.spectrum(x, SR, n=2048)
    env = analysis.onset_envelope(power)
    lag0, half, span = n // hop, int(beat * SR / 2 / hop), int(4 * SR / hop)
    best, score = lag0, -np.inf
    for lag in range(max(1, lag0 - half), lag0 + half + 1):
        a, b = env[:span], env[lag : lag + span]
        if len(b) < span:
            break
        s = np.corrcoef(a, b)[0, 1] if a.std() and b.std() else -np.inf
        if s > score:
            best, score = lag, s
    return best * hop


def blocks_db(x, blk=SR // 10):
    m = x.mean(axis=1)
    return 10 * np.log10(np.array([np.mean(m[i : i + blk] ** 2) for i in range(0, len(m) - blk + 1, blk)]) + 1e-12)


def content_end(x):
    """End of the last 100 ms block within 45 dB of the loudest, plus a 0.3 s release."""
    e = blocks_db(x)
    last = np.flatnonzero(e > e.max() - 45)[-1]
    return min(len(x), (last + 1) * (SR // 10) + int(0.3 * SR))


def fit(x, seconds):
    """Exact length: cut with a 2 s fade-out, or let a natural ending ring into silence."""
    n = int(round(seconds * SR))
    if len(x) >= n:
        x = x[:n].copy()
        f = min(2 * SR, n // 4)
        x[-f:] *= np.linspace(1, 0, f, dtype=np.float32)[:, None]
        return x
    return np.concatenate([x, np.zeros((n - len(x), x.shape[1]), np.float32)])


def steady_start(x, need):
    """Start of the most even stretch of `need` samples: skips model intros, fade-ins and endings."""
    blk = SR // 10
    e = blocks_db(x, blk)
    nb = -(-need // blk)
    check(len(e) >= nb, "Generated audio is too short for the requested loop")
    # The quietest 10% of 100 ms blocks in the window, so brief gaps between hits do not decide it.
    return blk * max(range(len(e) - nb + 1), key=lambda s: (np.percentile(e[s : s + nb], 10), -s))


def make_loop(x, seconds, fade, beat=None, equal_power=True):
    """Cut a seamless loop: the tail after the loop end is crossfaded into the start.

    The last output sample and the first are consecutive samples of the generated audio,
    so the wrap point is continuous by construction; the crossfade hides the content change.
    """
    f = int(fade * SR)
    n = int(round(seconds * SR))
    margin = int(beat * SR / 2) if beat else 0
    seg = x[steady_start(x, n + f + margin) :]
    if beat:
        n = best_period(seg, n, beat)
    check(n + f <= len(seg), "Generated audio is too short for the requested loop")
    t = (np.arange(f, dtype=np.float32) / f)[:, None]
    fade_in, fade_out = (np.sin(t * np.pi / 2), np.cos(t * np.pi / 2)) if equal_power else (t, 1 - t)
    out = seg[:n].copy()
    out[:f] = seg[:f] * fade_in + seg[n : n + f] * fade_out
    return out


def finish(y, kind, loop):
    """Fade one-shots, then gain to the loudness target under the true-peak ceiling."""
    y = y.astype(np.float32)
    rms = analysis.db(np.sqrt(np.mean(y**2)))
    if rms < -70:
        raise RuntimeError(f"The model returned near-silence ({rms:.1f} dBFS RMS)")
    if not loop:
        fi, fo = min(len(y) // 4, int(0.002 * SR)), min(len(y) // 4, int(0.02 * SR))
        y[:fi] *= np.linspace(0, 1, fi, dtype=np.float32)[:, None]
        y[-fo:] *= np.linspace(1, 0, fo, dtype=np.float32)[:, None]
    ceiling_gain = CEILING_DBTP - analysis.true_peak(y)
    target = TARGET_LUFS.get(kind)
    before = analysis.loudness(y, SR)
    gain = ceiling_gain if target is None or before is None else min(target - before, ceiling_gain)
    y = y * np.float32(10 ** (gain / 20))
    after = analysis.loudness(y, SR)
    return y, {
        "target_lufs": target,
        "ceiling_dbtp": CEILING_DBTP,
        "gain_db": gain,
        "loudness_lufs": after,
        "true_peak_dbtp": analysis.true_peak(y),
        # A peaky bed is held below the target by the ceiling rather than limited.
        "target_met": target is None or (after is not None and abs(after - target) <= 0.5),
    }


def peak_memory_mb():
    stats = {"peak_rss_mb": resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 2**20}
    try:
        import mlx.core as mx

        stats["mlx_peak_mb"] = mx.get_peak_memory() / 2**20
        mx.reset_peak_memory()
    except (ImportError, AttributeError):
        pass
    return stats


def render(stage, cue, work, progress):
    """Generate one cue and return (samples, manifest fields). cue keys: prompt, seconds, loop, seed, bpm, lyrics, kind."""
    free_space()
    prompt, seconds, loop, seed = cue["prompt"], float(cue["seconds"]), bool(cue["loop"]), int(cue["seed"])
    check(isinstance(prompt, str) and 0 < len(prompt.strip()) <= 512, "prompt must contain 1-512 characters")
    check(math.isfinite(seconds), "seconds must be a number")
    bpm = cue.get("bpm")
    loop_info = {"enabled": loop}
    if stage == "sfx":
        # Loops: crossfade plus 1.5 s of slack for the steadiest window to move in.
        fade = min(2.0, seconds / 4)
        extra = fade + 1.5 if loop else 0.0
        check(0.5 <= seconds and seconds + extra <= MOSS_MAX_SEC,
              f"sfx seconds must be 0.5-{MOSS_MAX_SEC:g}" + (f" ({MOSS_MAX_SEC - 3.5:g} with --loop)" if loop else ""))
        started = time.monotonic()
        progress(0.0)
        moss()
        loaded = time.monotonic()
        x = highpass(sfx_raw(prompt.strip(), round(seconds + extra, 1), seed, progress))
        if loop:
            x = make_loop(x, seconds, fade)
            loop_info.update(crossfade_sec=fade, crossfade="equal-power")
    else:
        check(bpm is None or (isinstance(bpm, int) and 30 <= bpm <= 300), "bpm must be an integer 30-300")
        beat = 60 / bpm if bpm else 0.5
        bar = 4 * beat
        # ACE-Step ends its piece early and pads the rest of the requested length with
        # silence (about 2.5 s of 12 s, 5-6 s of 25-40 s here), so ask for more and trim.
        # A loop also needs room to skip an intro and pick its steadiest stretch.
        extra = 8.0 + 0.15 * seconds + bar + beat if loop else 6.0
        check(ACE_MIN_SEC <= seconds and seconds + extra <= ACE_MAX_SEC,
              f"music seconds must be {ACE_MIN_SEC:g}-{ACE_MAX_SEC:g}" + (" including the loop margin" if loop else ""))
        started = time.monotonic()
        progress(0.0)
        ace()
        loaded = time.monotonic()
        x = highpass(music_raw(prompt.strip(), seconds + extra, seed, bpm, cue.get("lyrics"), work, progress))
        x = x[: content_end(x)]
        content = len(x) / SR
        if loop:
            x = make_loop(x, seconds, min(bar, seconds / 4), beat=beat, equal_power=False)
            loop_info.update(crossfade_sec=min(bar, seconds / 4), crossfade="linear, beat-aligned")
        else:
            x = fit(x, seconds)
    generated = time.monotonic()
    raw_len = len(x) / SR
    y, level = finish(x, cue["kind"], loop)
    if loop:
        loop_info["seam"] = analysis.seam(y, SR)
        loop_info["seam_ok"] = all(v <= analysis.SEAM_LIMIT for k, v in loop_info["seam"].items() if k.endswith("_ratio"))
    repo, revision, _ = SOURCES[MODEL_KEY[stage]]
    runtime = LICENCES[stage]["runtime"]
    return y, {
        "stage": stage,
        "tool": "forge-" + stage,
        "model": {"id": repo, "revision": revision, "licence": LICENCES[stage]["model"]},
        "runtime": {name: {"version": version(name), "licence": lic} for name, lic in runtime.items()},
        "prompt": prompt,
        "seed": seed,
        "kind": cue["kind"],
        **({"bpm": bpm, "content_sec": content} if stage == "music" else {}),
        **({"lyrics": cue["lyrics"]} if cue.get("lyrics") else {}),
        "requested_sec": seconds,
        "generated_sec": round(seconds + extra, 3),
        "duration_sec": raw_len,
        "sample_rate": SR,
        "channels": y.shape[1],
        "format": "WAV PCM 16-bit",
        "loudness": level,
        "loop": loop_info,
        "measurements": analysis.describe(y, SR),
        "timings": {"load_sec": loaded - started, "generate_sec": generated - loaded,
                    "finish_sec": time.monotonic() - generated},
        "memory": peak_memory_mb(),
    }


def write(work, name, y, manifest):
    import soundfile

    wav = work / name
    soundfile.write(str(wav), np.clip(y, -1, 1), SR, subtype="PCM_16")
    side = work / (name + ".manifest.json")
    side.write_text(json.dumps(manifest, indent=2) + "\n")
    return [wav, side]


def publish(files, dest_dir):
    """Hard-link (or copy across volumes) finished files from the work folder to dest_dir."""
    targets = [dest_dir / f.name for f in files]
    for t in targets:
        if t.exists():
            raise FileExistsError(f"Output already exists: choose a fresh path for {t}")
    done = []
    try:
        for src, dest in zip(files, targets):
            try:
                os.link(src, dest)
            except OSError:
                shutil.copy2(src, dest)
            done.append(dest)
    except BaseException:
        for dest in done:
            dest.unlink(missing_ok=True)
        raise
    return targets
