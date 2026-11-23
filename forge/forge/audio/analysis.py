"""Objective audio measurements shared by the stage, its tests and the evidence page.

Arrays are float32, shaped (samples, channels). Only numpy, scipy and pyloudnorm.
"""

import numpy as np
import pyloudnorm
from scipy.signal import resample_poly, stft


def db(x):
    return float(20 * np.log10(max(float(x), 1e-12)))


def loudness(y, sr):
    """Integrated loudness (ITU-R BS.1770-4, LUFS); None below one 400 ms gating block."""
    if len(y) < int(0.4 * sr):
        return None
    value = pyloudnorm.Meter(sr).integrated_loudness(y)
    return float(value) if np.isfinite(value) else None


def true_peak(y):
    """4x oversampled peak in dBTP, the BS.1770 true-peak approximation."""
    return db(np.abs(resample_poly(y, 4, 1, axis=0)).max())


def spectrum(y, sr, n=2048):
    """Mono power spectrogram (freqs, frames) with ~10.7 ms hops at 48 kHz."""
    f, _, z = stft(y.mean(axis=1), sr, nperseg=n, noverlap=n - 512)
    return f, np.abs(z) ** 2


def onset_envelope(power):
    """Half-wave rectified spectral flux of the log-power spectrogram, one value per hop."""
    log = np.log1p(1000 * power / (power.max() + 1e-20))
    return np.maximum(np.diff(log, axis=1), 0).sum(axis=0)


def onsets_per_second(power, sr, hop=512):
    env = onset_envelope(power)
    if env.size < 3:
        return 0.0
    # Peaks above a moving median plus a fraction of the envelope's spread.
    k = max(1, int(0.25 * sr / hop))
    padded = np.pad(env, k, mode="edge")
    median = np.array([np.median(padded[i : i + 2 * k + 1]) for i in range(env.size)])
    gate = median + 0.5 * env.std()
    peaks = (env[1:-1] > gate[1:-1]) & (env[1:-1] >= env[:-2]) & (env[1:-1] > env[2:])
    # At most one onset per 50 ms.
    idx, last = [], -1e9
    for i in np.flatnonzero(peaks) + 1:
        if i - last >= 0.05 * sr / hop:
            idx.append(i)
            last = i
    return len(idx) / (env.size * hop / sr)


def tempo(power, sr, hop=512, lo=50, hi=220):
    """Global tempo: onset-envelope autocorrelation summed over 4 beat multiples, weighted toward 120 BPM."""
    env = onset_envelope(power)
    env = env - env.mean()
    if env.size < 4 * sr / hop or not env.any():
        return None
    ac = np.correlate(env, env, "full")[env.size - 1 :]
    bpm = np.arange(lo, hi + 0.25, 0.25)
    lag = 60 * sr / hop / bpm
    score = sum(np.interp(k * lag, np.arange(ac.size), ac, right=0) for k in (1, 2, 3, 4))
    return float(bpm[np.argmax(score * np.exp(-0.5 * np.log2(bpm / 120) ** 2))])


def describe(y, sr):
    """The numbers reported for every file."""
    f, power = spectrum(y, sr)
    weights = power.sum(axis=1)
    centroid = float((f * weights).sum() / weights.sum()) if weights.sum() else 0.0
    bandwidth = float(np.sqrt((((f - centroid) ** 2) * weights).sum() / weights.sum())) if weights.sum() else 0.0
    return {
        "duration_sec": len(y) / sr,
        "loudness_lufs": loudness(y, sr),
        "true_peak_dbtp": true_peak(y),
        "sample_peak_dbfs": db(np.abs(y).max()),
        "rms_dbfs": db(np.sqrt(np.mean(y**2))),
        "clipped_samples": int((np.abs(y) >= 0.999).sum()),
        "spectral_centroid_hz": centroid,
        "spectral_bandwidth_hz": bandwidth,
        "onsets_per_sec": onsets_per_second(power, sr),
        "tempo_bpm": tempo(power, sr),
    }


def band_db(power, bands=24):
    """Log-spaced band levels in dB (about 20 Hz to Nyquist at 48 kHz) of a power spectrum (bins first)."""
    edges = np.unique(np.geomspace(1, power.shape[0], bands + 1).astype(int))
    return 10 * np.log10(np.array([power[a:b].mean(axis=0) for a, b in zip(edges, edges[1:])]) + 1e-20)


def profile(y, sr):
    """Mean band levels over the clip, for comparing two clips."""
    return band_db(spectrum(y, sr)[1].mean(axis=1), 48)


def seam(y, sr, window=0.25, hop=0.05):
    """How the loop point (last sample -> first sample) compares with the file's own variation.

    Each ratio divides the seam value by a high percentile of the same quantity measured
    everywhere else in the file, so 1.0 means "as large as the file's own rare extremes":
    the 99.9th percentile of sample steps and slopes (a click is far outside it) and the
    99th percentile of band-spectrum changes between adjacent 250 ms windows (a jump in
    texture, level or tone is larger than the content's own changes).
    """
    wrapped = np.concatenate([y[-2:], y[:2]])
    d1 = np.abs(np.diff(y, axis=0))
    d2 = np.abs(np.diff(y, n=2, axis=0))
    jump = np.abs(wrapped[2] - wrapped[1]).max()
    slope = np.abs(np.diff(wrapped, n=2, axis=0)).max()
    w, h = int(window * sr), int(hop * sr)
    mono = y.mean(axis=1)

    def bands(frame):
        return band_db(np.abs(np.fft.rfft(frame * np.hanning(w))) ** 2)

    spec = np.array([bands(mono[i : i + w]) for i in range(0, len(mono) - w + 1, h)])
    k = w // h  # the window that starts where this one ends
    steps = np.abs(spec[k:] - spec[:-k]).mean(axis=1)
    across = np.abs(bands(mono[-w:]) - bands(mono[:w])).mean()
    return {
        "jump": float(jump),
        "jump_ratio": float(jump / (np.percentile(d1, 99.9) + 1e-12)),
        "slope": float(slope),
        "slope_ratio": float(slope / (np.percentile(d2, 99.9) + 1e-12)),
        "spectral_db": float(across),
        "spectral_ratio": float(across / (np.percentile(steps, 99) + 1e-12)),
    }


SEAM_LIMIT = 1.0  # every ratio above must stay at or below this (docs/audio.md)
