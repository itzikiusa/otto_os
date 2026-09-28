// Original, royalty-free soundtrack for the tour film — synthesized from
// scratch (no samples, no network). Writes to public/audio/:
//   music.wav     124 BPM drums + syncopated bass + bright chord stabs, sized to the film (reads
//                 src/generated/timing.json), with short phrase breaks and a
//                 clean final fade. The instrumental edition keeps
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
  const n = Math.round(DURATION * SR), L = new Float32Array(n), R = new Float32Array(n);
  const beat = 60 / 124, bar = 4 * beat;
  // Original bright house/pop groove: D major, B minor, G major, A major.
  // Short chord stabs, a syncopated bass and dry drums replace the ambient pad.
  const chords = [[50, 62, 66, 69, 73], [47, 59, 62, 66, 69], [43, 55, 59, 62, 66], [45, 57, 61, 64, 69]];
  const energy = t => Math.min(1, .45 + t / 6) * Math.min(1, (DURATION - t) / 3);
  function note(at, duration, frequency, amp, pan, kind) {
    const start = Math.round(at * SR), length = Math.round(duration * SR);
    for (let j = 0; j < length && start + j < n; j++) {
      const t = j / SR, phase = 2 * Math.PI * frequency * t;
      const envelope = (1 - Math.exp(-t * 240)) * Math.exp(-t * (kind === 'bass' ? 7 : 12));
      const harmonic = Math.sin(phase) + .26 * Math.sin(2 * phase) + (kind === 'bass' ? .10 : .18) * Math.sin(3 * phase);
      const pump = .45 + .55 * Math.min(1, ((at + t) % beat) / .14);
      const v = harmonic * envelope * amp * energy(at) * pump;
      L[start + j] += v * (1 - pan); R[start + j] += v * (1 + pan);
    }
  }
  for (let step = 0; step * beat / 4 < DURATION - 1.5; step++) {
    const at = step * beat / 4, inBar = step % 16;
    const chord = chords[Math.floor(at / (bar * 2)) % 4];
    const phrase = Math.floor(at / (bar * 8));
    // Each eight-bar phrase briefly opens space, then brings the full groove back.
    const breakdown = Math.floor(at / bar) % 16 === 15;
    if ([0, 3, 6, 8, 11, 14].includes(inBar)) note(at, .28, midi(chord[0] - 12 + (inBar === 14 ? 12 : 0)), .19, 0, 'bass');
    if ([2, 6, 10, 14].includes(inBar)) for (const [index, pitch] of chord.slice(1).entries()) note(at, .34, midi(pitch), .040, (index - 1.5) * .17, 'chord');
    if (at > bar * 2 && !breakdown && step % 2 === 1) {
      const melody = [1, 3, 2, 4, 3, 2, 4, 2];
      note(at, .30, midi(chord[melody[(step >> 1) % 8]] + 12), .038, step % 4 === 1 ? -.3 : .3, 'lead');
    }
    const start = Math.round(at * SR);
    if (inBar % 4 === 0 && (!breakdown || inBar === 0)) {
      for (let j = 0; j < .32 * SR && start + j < n; j++) {
        const t = j / SR;
        // Integrated pitch sweep avoids a buzzy discontinuity in the kick tail.
        const phase = 2 * Math.PI * (48 * t + 95 * (1 - Math.exp(-t * 32)) / 32);
        const v = (.48 * Math.sin(phase) * Math.exp(-t * 14) + .05 * rnd() * Math.exp(-t * 160)) * energy(at);
        L[start + j] += v; R[start + j] += v;
      }
    }
    if (inBar === 4 || inBar === 12) {
      let lp = 0;
      for (let j = 0; j < .18 * SR && start + j < n; j++) {
        const t = j / SR, noise = rnd(); lp += .22 * (noise - lp);
        const clap = (noise - lp) * (.9 * Math.exp(-t * 27) + .5 * Math.exp(-Math.abs(t - .012) * 300));
        const v = (clap * .105 + Math.sin(2 * Math.PI * 185 * t) * Math.exp(-t * 35) * .06) * energy(at);
        L[start + j] += v; R[start + j] += v;
      }
    }
    if (step % 2 === 0 || phrase % 2 === 1) {
      let previous = 0;
      const open = inBar % 4 === 2, length = open ? .13 : .045;
      for (let j = 0; j < length * SR && start + j < n; j++) {
        const noise = rnd(), high = noise - previous; previous = noise;
        const v = high * Math.exp(-(j / SR) * (open ? 35 : 100)) * (open ? .029 : .019) * energy(at);
        L[start + j] += v * .85; R[start + j] += v * 1.15;
      }
    }
  }
  // A short room-like tail retains punch; no long calming pad wash.
  reverb(L, R, .075, .55);
  for (let i = 0; i < n; i++) {
    const fade = Math.min(1, i / (SR * .25), (n - i) / (SR * 2));
    L[i] = Math.tanh(L[i] * 1.15) * fade; R[i] = Math.tanh(R[i] * 1.15) * fade;
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
