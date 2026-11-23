"""Spectrograms and a listening page for generated audio.

usage: forge/audio/.venv/bin/python tests/audio/evidence.py DIR [DIR ...]
Writes out/audio-evidence/index.html, one PNG per WAV and copies of the WAVs.
"""

import html
import json
import shutil
import sys
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402
import numpy as np  # noqa: E402
import soundfile  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
from forge.audio import analysis  # noqa: E402

OUT = ROOT / "out" / "audio-evidence"


def naive_seam(y, sr):
    """Control: the same content joined at an arbitrary point without a crossfade."""
    return analysis.seam(y[: len(y) // 2 + 1234], sr)


def spectrogram(y, sr, title, loop, path):
    mono = y.mean(axis=1)
    if loop:  # Two passes, so the loop point sits in the middle of the picture.
        mono = np.concatenate([mono, mono])
    fig, (wave, spec) = plt.subplots(2, 1, figsize=(10, 4.2), sharex=True,
                                     gridspec_kw={"height_ratios": [1, 3]})
    t = np.arange(mono.size) / sr
    wave.plot(t[::16], mono[::16], lw=0.4, color="#34495e")
    wave.set_ylim(-1, 1)
    wave.set_title(title, fontsize=9)
    spec.specgram(mono, NFFT=2048, Fs=sr, noverlap=1536, cmap="magma", vmin=-140, vmax=-20)
    spec.set_ylabel("Hz")
    spec.set_xlabel("seconds" + (" (file played twice; dashed line = loop point)" if loop else ""))
    if loop:
        for ax in (wave, spec):
            ax.axvline(len(y) / sr, color="#2ecc71", ls="--", lw=1)
    fig.tight_layout()
    fig.savefig(path, dpi=110)
    plt.close(fig)


def fmt(v, unit=""):
    if v is None:
        return "–"
    return f"{v:.2f}{unit}" if isinstance(v, float) else f"{v}{unit}"


def build(dirs):
    OUT.mkdir(parents=True, exist_ok=True)
    rows, report = [], []
    for wav in sorted(p for d in dirs for p in Path(d).resolve().glob("*.wav")):
        side = Path(str(wav) + ".manifest.json")
        if not side.exists():
            continue
        manifest = json.loads(side.read_text())
        y, sr = soundfile.read(str(wav), dtype="float32", always_2d=True)
        name = f"{Path(wav).parent.name}-{wav.stem}"
        loop = manifest["loop"]["enabled"]
        m = analysis.describe(y, sr)
        seam = analysis.seam(y, sr)  # for a non-loop file this is a control: its wrap is not crossfaded
        control = naive_seam(y, sr) if loop else None
        spectrogram(y, sr, f"{wav.name}: {manifest['prompt'][:90]}", loop, OUT / f"{name}.png")
        shutil.copy2(wav, OUT / f"{name}.wav")
        report.append({"file": str(wav.relative_to(ROOT)), "prompt": manifest["prompt"], "kind": manifest["kind"],
                       "seed": manifest["seed"], "bpm": manifest.get("bpm"), "measurements": m, "seam": seam,
                       "naive_cut_control": control, "timings": manifest["timings"], "memory": manifest["memory"],
                       "model": manifest["model"]["id"]})
        ratios = "<br>".join(f"{k} {v:.2f}" for k, v in seam.items() if k.endswith("_ratio"))
        seam_txt = (ratios + "<br><i>naive cut: " + ", ".join(f"{v:.2f}" for k, v in control.items()
                                                               if k.endswith("_ratio")) + "</i>") if loop \
            else f"<i>not a loop; wrapped as a control:</i><br>{ratios}"
        rows.append(f"""<tr><td><b>{html.escape(wav.name)}</b><br><small>{html.escape(manifest['kind'])}, seed {manifest['seed']}
{', ' + str(manifest['bpm']) + ' bpm requested' if manifest.get('bpm') else ''}<br>{html.escape(manifest['model']['id'])}</small>
<p>{html.escape(manifest['prompt'])}</p><audio controls {'loop ' if loop else ''}preload="none" src="{name}.wav"></audio></td>
<td><img src="{name}.png" width="560" alt="spectrogram of {html.escape(wav.name)}"></td>
<td>{fmt(m['duration_sec'], ' s')}<br>{fmt(m['loudness_lufs'], ' LUFS')}<br>{fmt(m['true_peak_dbtp'], ' dBTP')}<br>
centroid {fmt(m['spectral_centroid_hz'], ' Hz')}<br>bandwidth {fmt(m['spectral_bandwidth_hz'], ' Hz')}<br>
onsets {fmt(m['onsets_per_sec'], '/s')}<br>tempo {fmt(m['tempo_bpm'], ' bpm')}<br>
generate {fmt(manifest['timings']['generate_sec'], ' s')}</td><td>{seam_txt}</td></tr>""")
    (OUT / "measurements.json").write_text(json.dumps(report, indent=2) + "\n")
    (OUT / "index.html").write_text(f"""<!doctype html><meta charset="utf-8"><title>Forge audio: listening check</title>
<style>body{{font:14px system-ui;margin:24px;color:#222}}td{{vertical-align:top;padding:8px;border-top:1px solid #ddd}}
p{{max-width:28em}}small{{color:#666}}</style>
<h1>Forge audio: listening check</h1>
<p>These files were measured, not listened to. Please listen on headphones and on speakers: is the content what the
prompt asks for, are there artefacts, and do the loops (players set to repeat) wrap without a click or a jump?
Loudness is ITU-R BS.1770 integrated (beds -18 LUFS, music -16 LUFS, one-shots peak-normalised to -1 dBTP).
Seam ratios compare the loop point with the file's own 99.9th-percentile sample step and slope and 99th-percentile
change in band spectrum between adjacent 250 ms windows; 1.0 or less means the seam is not an outlier. Two
controls: the naive-cut figures join the same loop at an arbitrary point without a crossfade, and files that are not
loops are wrapped end-to-start as they are.</p>
<table><tr><th>File and prompt</th><th>Waveform and spectrogram</th><th>Measures</th><th>Loop seam</th></tr>
{''.join(rows)}</table>
""")
    return report


if __name__ == "__main__":
    build(sys.argv[1:])
    print(OUT / "index.html")
