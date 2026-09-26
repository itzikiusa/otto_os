// Original, royalty-free soundtrack for the tour film — synthesized from
// scratch (no samples, no network). Writes to public/audio/:
//   music.wav     ambient pad + soft pulse, sized to the film (reads
//                 src/generated/timing.json), with an intro rise and an outro
//                 swell/ring-out. The instrumental edition keeps
//                 the score present throughout; captions carry the instructions.
//   sfx-*.wav     small UI accents: click, whoosh, tick, riser, impact.
//
//   node scripts/soundtrack.mjs [--duration <sec>]
import { mkdirSync, readFileSync, writeFileSync, existsSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const tour = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const outDir = join(tour, 'public/audio');
mkdirSync(outDir, { recursive: true });
const SR = 48000;

function arg(name, dflt) {
  const i = process.argv.indexOf(`--${name}`);
  return i > 0 ? process.argv[i + 1] : dflt;
}
const timingFile = join(tour, 'src/generated/timing.json');
const timing = existsSync(timingFile) ? JSON.parse(readFileSync(timingFile, 'utf8')) : null;
const DURATION = Number(arg('duration', timing ? timing.totalSeconds + 1 : 150));

// ── tiny DSP kit ───────────────────────────────────────────────────────────
let seed = 1234567;
const rnd = () => ((seed = (seed * 1664525 + 1013904223) >>> 0) / 4294967296) * 2 - 1;
const midi = (n) => 440 * 2 ** ((n - 69) / 12);
const clamp = (x, a, b) => Math.max(a, Math.min(b, x));
const smooth = (x) => x * x * (3 - 2 * x);

function writeWav(file, L, R) {
  const n = L.length;
  const buf = Buffer.alloc(44 + n * 4);
  buf.write('RIFF', 0);
  buf.writeUInt32LE(36 + n * 4, 4);
  buf.write('WAVE', 8);
  buf.write('fmt ', 12);
  buf.writeUInt32LE(16, 16);
  buf.writeUInt16LE(1, 20);
  buf.writeUInt16LE(2, 22);
  buf.writeUInt32LE(SR, 24);
  buf.writeUInt32LE(SR * 4, 28);
  buf.writeUInt16LE(4, 32);
  buf.writeUInt16LE(16, 34);
  buf.write('data', 36);
  buf.writeUInt32LE(n * 4, 40);
  let peak = 0;
  for (let i = 0; i < n; i++) peak = Math.max(peak, Math.abs(L[i]), Math.abs(R[i]));
  const g = peak > 0.89 ? 0.89 / peak : 1;
  for (let i = 0; i < n; i++) {
    buf.writeInt16LE(Math.round(clamp(L[i] * g, -1, 1) * 32767), 44 + i * 4);
    buf.writeInt16LE(Math.round(clamp(R[i] * g, -1, 1) * 32767), 46 + i * 4);
  }
  writeFileSync(file, buf);
  console.log(`[soundtrack] ${file.replace(tour + '/', '')} ${(n / SR).toFixed(2)}s peak ${peak.toFixed(2)}`);
}

/** Schroeder-style stereo reverb (4 combs + 2 allpasses per side). */
function reverb(L, R, mix = 0.28, size = 1) {
  const combs = [1557, 1617, 1491, 1422].map((d) => Math.round(d * size * (SR / 44100)));
  const aps = [225, 556].map((d) => Math.round(d * (SR / 44100)));
  const run = (x, spread) => {
    const out = new Float32Array(x.length);
    for (const d0 of combs) {
      const d = d0 + spread;
      const buf = new Float32Array(d);
      let idx = 0;
      let lp = 0;
      for (let i = 0; i < x.length; i++) {
        const y = buf[idx];
        lp = y * 0.7 + lp * 0.3;
        buf[idx] = x[i] + lp * 0.83;
        idx = (idx + 1) % d;
        out[i] += y * 0.25;
      }
    }
    for (const d of aps) {
      const buf = new Float32Array(d);
      let idx = 0;
      for (let i = 0; i < out.length; i++) {
        const b = buf[idx];
        const y = -out[i] * 0.5 + b;
        buf[idx] = out[i] + b * 0.5;
        idx = (idx + 1) % d;
        out[i] = y;
      }
    }
    return out;
  };
  const wl = run(L, 0);
  const wr = run(R, 23);
  for (let i = 0; i < L.length; i++) {
    L[i] = L[i] * (1 - mix) + wl[i] * mix;
    R[i] = R[i] * (1 - mix) + wr[i] * mix;
  }
}

// ── music ──────────────────────────────────────────────────────────────────
function music() {
  const n = Math.round(DURATION * SR);
  const L = new Float32Array(n);
  const R = new Float32Array(n);
  const BPM = 112;
  const beat = 60 / BPM;
  const bar = beat * 4;
  // Dmaj9 – Bm9 – Gmaj7(#11) – A6sus : warm, optimistic, unresolved loop.
  const chords = [
    [50, 57, 62, 64, 66, 69],
    [47, 54, 59, 61, 62, 66],
    [43, 50, 55, 59, 61, 66],
    [45, 52, 57, 59, 62, 64],
  ];
  const chordLen = bar * 2;
  const INTRO = 6.5;
  const OUTRO = Math.max(8, DURATION * 0.06);

  // Section energy 0..1 over time: intro rise, body, outro swell + ring-out.
  const energy = (t) => {
    if (t < INTRO) return 0.35 + 0.65 * smooth(t / INTRO);
    if (t > DURATION - OUTRO) return 1 - 0.85 * smooth((t - (DURATION - OUTRO)) / OUTRO);
    return 1;
  };
  const pulseOn = (t) => (t < INTRO - 1.2 ? 0 : t > DURATION - OUTRO + 1 ? 0 : 1);

  // Pad: per chord tone, 3 slightly detuned partial stacks, slow crossfades.
  for (let c = 0; c * chordLen < DURATION + chordLen; c++) {
    const notes = chords[c % chords.length];
    const t0 = c * chordLen - 0.8;
    const t1 = t0 + chordLen + 1.6;
    const i0 = Math.max(0, Math.round(t0 * SR));
    const i1 = Math.min(n, Math.round(t1 * SR));
    for (const [k, note] of notes.entries()) {
      const f = midi(note + 12 * (k < 2 ? 0 : 0));
      const amp = (k === 0 ? 0.09 : 0.055) / Math.sqrt(notes.length);
      const det = [0.9965, 1, 1.0037];
      const ph = det.map(() => (rnd() + 1) * Math.PI);
      for (let i = i0; i < i1; i++) {
        const t = i / SR;
        const local = (t - t0) / (t1 - t0);
        const env = Math.sin(Math.PI * clamp(local, 0, 1)) ** 1.4;
        const bright = 0.35 + 0.65 * energy(t);
        let sl = 0;
        let sr = 0;
        for (let d = 0; d < 3; d++) {
          const w = 2 * Math.PI * f * det[d] * t + ph[d];
          const s = Math.sin(w) + 0.32 * bright * Math.sin(2 * w) + 0.12 * bright * Math.sin(3 * w);
          if (d === 0) sl += s;
          else if (d === 2) sr += s;
          else {
            sl += 0.6 * s;
            sr += 0.6 * s;
          }
        }
        const trem = 1 + 0.08 * Math.sin(2 * Math.PI * 0.21 * t + k);
        const e = env * amp * trem * (0.55 + 0.45 * energy(t));
        L[i] += sl * e;
        R[i] += sr * e;
      }
    }
    // Sub bass on the root.
    const root = midi(notes[0] - 12);
    for (let i = i0; i < i1; i++) {
      const t = i / SR;
      const local = (t - t0) / (t1 - t0);
      const env = Math.sin(Math.PI * clamp(local, 0, 1)) ** 2;
      const s = Math.sin(2 * Math.PI * root * t) * 0.07 * env * energy(t);
      L[i] += s;
      R[i] += s;
    }
  }

  // Pulse: soft plucked arpeggio on 8ths + a felt kick on 1 and 3 + airy hats.
  const eighth = beat / 2;
  for (let s = 0; s * eighth < DURATION; s++) {
    const t = s * eighth;
    if (!pulseOn(t)) continue;
    const notes = chords[Math.floor(t / chordLen) % chords.length];
    const pattern = [2, 4, 3, 5, 2, 5, 4, 3];
    const note = notes[pattern[s % 8]] + 12;
    const f = midi(note);
    const i0 = Math.round(t * SR);
    const len = Math.round(0.42 * SR);
    const amp = (s % 2 ? 0.035 : 0.05) * energy(t);
    const pan = s % 4 < 2 ? 0.35 : -0.35;
    for (let j = 0; j < len && i0 + j < n; j++) {
      const tt = j / SR;
      const env = Math.exp(-tt * 9) * (1 - Math.exp(-tt * 400));
      const w = 2 * Math.PI * f * tt;
      const v = (Math.sin(w) + 0.25 * Math.sin(2 * w) * Math.exp(-tt * 14)) * env * amp;
      L[i0 + j] += v * (1 - pan);
      R[i0 + j] += v * (1 + pan);
    }
    if (s % 4 === 0) {
      const klen = Math.round(0.28 * SR);
      for (let j = 0; j < klen && i0 + j < n; j++) {
        const tt = j / SR;
        const fk = 48 + 70 * Math.exp(-tt * 28);
        const v = Math.sin(2 * Math.PI * fk * tt) * Math.exp(-tt * 11) * 0.12 * energy(t);
        L[i0 + j] += v;
        R[i0 + j] += v;
      }
    }
    if (s % 2 === 1) {
      const hlen = Math.round(0.06 * SR);
      let hp = 0;
      let prev = 0;
      for (let j = 0; j < hlen && i0 + j < n; j++) {
        const x = rnd();
        hp = 0.92 * (hp + x - prev);
        prev = x;
        const v = hp * Math.exp(-(j / SR) * 60) * 0.012 * energy(t);
        L[i0 + j] += v * 0.8;
        R[i0 + j] += v * 1.2;
      }
    }
  }

  // Intro riser (filtered noise swell) and a final chord ring.
  const riseLen = Math.round(INTRO * SR);
  let lp = 0;
  for (let i = 0; i < riseLen && i < n; i++) {
    const t = i / SR;
    const k = 0.02 + 0.3 * (t / INTRO) ** 2;
    lp += k * (rnd() - lp);
    const v = lp * 0.06 * smooth(t / INTRO) * (t < INTRO - 0.15 ? 1 : 0);
    L[i] += v;
    R[i] += v;
  }
  const endNotes = [50, 57, 62, 66, 69, 74];
  const e0 = Math.round((DURATION - OUTRO * 0.8) * SR);
  for (let i = e0; i < n; i++) {
    const tt = (i - e0) / SR;
    const env = (1 - Math.exp(-tt * 2)) * Math.exp(-tt * 0.35);
    let v = 0;
    for (const nn of endNotes) v += Math.sin(2 * Math.PI * midi(nn) * tt) / endNotes.length;
    L[i] += v * 0.08 * env;
    R[i] += v * 0.08 * env;
  }

  reverb(L, R, 0.3, 1.15);
  // Gentle fades at both ends.
  const fi = Math.round(0.5 * SR);
  const fo = Math.round(2.5 * SR);
  for (let i = 0; i < fi; i++) {
    L[i] *= i / fi;
    R[i] *= i / fi;
  }
  for (let i = 0; i < fo; i++) {
    const g = i / fo;
    L[n - 1 - i] *= g;
    R[n - 1 - i] *= g;
  }
  writeWav(join(outDir, 'music.wav'), L, R);
}

// ── sfx ────────────────────────────────────────────────────────────────────
function sfx(name, secs, fn) {
  const n = Math.round(secs * SR);
  const L = new Float32Array(n);
  const R = new Float32Array(n);
  fn(L, R, n);
  writeWav(join(outDir, `sfx-${name}.wav`), L, R);
}

function effects() {
  // Soft UI click: a damped 2.2 kHz blip + a touch of noise.
  sfx('click', 0.12, (L, R, n) => {
    for (let i = 0; i < n; i++) {
      const t = i / SR;
      const v = (Math.sin(2 * Math.PI * 2200 * t) * 0.5 + rnd() * 0.15) * Math.exp(-t * 90) * 0.5;
      L[i] = v;
      R[i] = v;
    }
  });
  // Whoosh: band-passed noise swept up then down, panned across.
  sfx('whoosh', 0.7, (L, R, n) => {
    let lp = 0;
    let hp = 0;
    let prev = 0;
    for (let i = 0; i < n; i++) {
      const t = i / SR;
      const p = t / 0.7;
      const k = 0.02 + 0.25 * Math.sin(Math.PI * p);
      lp += k * (rnd() - lp);
      hp = 0.97 * (hp + lp - prev);
      prev = lp;
      const env = Math.sin(Math.PI * p) ** 2;
      const v = hp * env * 0.9;
      L[i] = v * (1 - p);
      R[i] = v * p;
    }
    reverb(L, R, 0.2, 0.6);
  });
  // Tick: a tiny glassy two-note chime for callouts.
  sfx('tick', 0.5, (L, R, n) => {
    for (let i = 0; i < n; i++) {
      const t = i / SR;
      const a = Math.sin(2 * Math.PI * midi(86) * t) * Math.exp(-t * 18);
      const b = t > 0.06 ? Math.sin(2 * Math.PI * midi(93) * (t - 0.06)) * Math.exp(-(t - 0.06) * 14) : 0;
      const v = (a + b) * 0.16;
      L[i] = v;
      R[i] = v;
    }
    reverb(L, R, 0.3, 0.7);
  });
  // Riser into the logo reveal.
  sfx('riser', 2.2, (L, R, n) => {
    let lp = 0;
    for (let i = 0; i < n; i++) {
      const t = i / SR;
      const p = t / 2.2;
      lp += (0.01 + 0.4 * p * p) * (rnd() - lp);
      const tone = Math.sin(2 * Math.PI * (220 + 660 * p * p) * t) * 0.2;
      const v = (lp * 0.8 + tone) * p ** 2 * 0.5;
      L[i] = v;
      R[i] = v;
    }
  });
  // Impact: warm low hit + shimmer for the title / outro logo.
  sfx('impact', 2.5, (L, R, n) => {
    for (let i = 0; i < n; i++) {
      const t = i / SR;
      const f = 42 + 60 * Math.exp(-t * 18);
      let v = Math.sin(2 * Math.PI * f * t) * Math.exp(-t * 3.2) * 0.6;
      for (const nn of [74, 81, 86]) v += Math.sin(2 * Math.PI * midi(nn) * t) * Math.exp(-t * 2.2) * 0.05;
      L[i] = v;
      R[i] = v;
    }
    reverb(L, R, 0.35, 1.2);
  });
}

effects();
music();

// Normalize the instrumental score; the final film is mastered to −16 LUFS.
{
  const { execFileSync } = await import('node:child_process');
  const { renameSync } = await import('node:fs');
  const FF = process.env.FFMPEG ?? 'ffmpeg';
  const src = join(outDir, 'music.wav');
  const tmp = join(outDir, 'music.norm.wav');
  execFileSync(FF, ['-y', '-v', 'error', '-i', src, '-af', 'loudnorm=I=-20:TP=-3:LRA=11', '-ar', '48000', '-ac', '2', tmp]);
  renameSync(tmp, src);
  console.log('[soundtrack] music.wav normalized to −20 LUFS');
}
