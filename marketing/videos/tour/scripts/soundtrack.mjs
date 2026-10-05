// Original, royalty-free score for the tour film — synthesized from scratch in
// this file (no samples, no network). Writes to public/audio/:
//   music.wav   120 BPM electronic pop in D major, sized and arranged to the edit
//               in src/generated/timing.json: intro build → drop on the first
//               chapter → verse/chorus energy changes → breakdown + riser →
//               final chorus → resolved outro. Chapter starts get a fill and a
//               crash; section starts get a riser and an impact.
//   sfx-*.wav   UI accents tuned to the key: click, whoosh, tick, riser, impact.
//
//   node scripts/soundtrack.mjs            # needs src/generated/timing.json (scripts/timing.mjs)
//
// Sound design: polyBLEP (alias-suppressed) saws; a 7-voice supersaw pad through
// a resonant state-variable low-pass, side-chained to the kick; a filtered-saw
// pluck arp with a ping-pong delay; a detuned saw lead carrying a 3+3+2 hook
// that recurs; layered kick (pitched body + click + saturation), clap with a
// short room, 808-style metallic hats with swing and accents. The mix keeps its
// weight in the mids (bass harmonics, lead, arp) so it carries on laptop
// speakers; everything below 35 Hz is filtered and no fundamental sits under 43 Hz.
import { execFileSync } from 'node:child_process';
import { mkdirSync, readFileSync, renameSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const tour = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const outDir = join(tour, 'public/audio');
mkdirSync(outDir, { recursive: true });
const FF = process.env.FFMPEG ?? 'ffmpeg';
const SR = 48000;
const timing = JSON.parse(readFileSync(join(tour, 'src/generated/timing.json'), 'utf8'));
const BPM = timing.bpm;
const BEAT = 60 / BPM;
const BAR = 4 * BEAT;
const STEP = BEAT / 4; // a 16th
const BARS = timing.totalBars;
const TAIL = 1.5; // the last chord rings past the final frame; the film fades it
const DURATION = BARS * BAR + TAIL;
const N = Math.round(DURATION * SR);

// ── tiny DSP kit ───────────────────────────────────────────────────────────
let seed = 20261005;
const rnd = () => ((seed = (seed * 1664525 + 1013904223) >>> 0) / 4294967296) * 2 - 1;
const midi = (n) => 440 * 2 ** ((n - 69) / 12);
const clamp = (x, a, b) => Math.max(a, Math.min(b, x));
const sec = (t) => Math.round(t * SR);

/** A stereo buffer ("bus"). */
const bus = () => ({ L: new Float32Array(N), R: new Float32Array(N) });
const put = (b, i, v, pan = 0) => {
  if (i < 0 || i >= N) return;
  b.L[i] += v * Math.min(1, 1 - pan);
  b.R[i] += v * Math.min(1, 1 + pan);
};

/** polyBLEP residual: removes the step discontinuity that makes naive saws alias. */
function blep(t, dt) {
  if (t < dt) {
    t /= dt;
    return t + t - t * t - 1;
  }
  if (t > 1 - dt) {
    t = (t - 1) / dt;
    return t * t + t + t + 1;
  }
  return 0;
}
class Saw {
  constructor(phase = Math.random()) {
    this.p = phase;
  }
  tick(f) {
    const dt = f / SR;
    this.p += dt;
    if (this.p >= 1) this.p -= 1;
    return 2 * this.p - 1 - blep(this.p, dt);
  }
}

/** Topology-preserving state-variable filter (stable under fast modulation). */
class SVF {
  constructor() {
    this.a = 0;
    this.b = 0;
    this.set(1000, 0.707);
  }
  set(fc, q) {
    const g = Math.tan((Math.PI * clamp(fc, 20, SR * 0.45)) / SR);
    this.k = 1 / q;
    this.a1 = 1 / (1 + g * (g + this.k));
    this.a2 = g * this.a1;
    this.a3 = g * this.a2;
  }
  run(x) {
    const v3 = x - this.b;
    const v1 = this.a1 * this.a + this.a2 * v3;
    const v2 = this.b + this.a2 * this.a + this.a3 * v3;
    this.a = 2 * v1 - this.a;
    this.b = 2 * v2 - this.b;
    this.lp = v2;
    this.bp = v1;
    this.hp = x - this.k * v1 - v2;
    return v2;
  }
}

/** RBJ biquad over a whole channel, in place. */
function biquad(x, type, f0, q = 0.707, gainDb = 0) {
  const A = 10 ** (gainDb / 40);
  const w = (2 * Math.PI * f0) / SR;
  const cs = Math.cos(w);
  const al = Math.sin(w) / (2 * q);
  let b0, b1, b2, a0, a1, a2;
  if (type === 'hp') [b0, b1, b2, a0, a1, a2] = [(1 + cs) / 2, -(1 + cs), (1 + cs) / 2, 1 + al, -2 * cs, 1 - al];
  else if (type === 'lp') [b0, b1, b2, a0, a1, a2] = [(1 - cs) / 2, 1 - cs, (1 - cs) / 2, 1 + al, -2 * cs, 1 - al];
  else if (type === 'peak') [b0, b1, b2, a0, a1, a2] = [1 + al * A, -2 * cs, 1 - al * A, 1 + al / A, -2 * cs, 1 - al / A];
  else if (type === 'lowshelf') {
    const s = 2 * Math.sqrt(A) * al;
    [b0, b1, b2] = [A * (A + 1 - (A - 1) * cs + s), 2 * A * (A - 1 - (A + 1) * cs), A * (A + 1 - (A - 1) * cs - s)];
    [a0, a1, a2] = [A + 1 + (A - 1) * cs + s, -2 * (A - 1 + (A + 1) * cs), A + 1 + (A - 1) * cs - s];
  } else if (type === 'highshelf') {
    const s = 2 * Math.sqrt(A) * al;
    [b0, b1, b2] = [A * (A + 1 + (A - 1) * cs + s), -2 * A * (A - 1 + (A + 1) * cs), A * (A + 1 + (A - 1) * cs - s)];
    [a0, a1, a2] = [A + 1 - (A - 1) * cs + s, 2 * (A - 1 - (A + 1) * cs), A + 1 - (A - 1) * cs - s];
  } else throw new Error(type);
  let x1 = 0, x2 = 0, y1 = 0, y2 = 0;
  for (let i = 0; i < x.length; i++) {
    const y = (b0 * x[i] + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2) / a0;
    x2 = x1;
    x1 = x[i];
    y2 = y1;
    y1 = y;
    x[i] = y;
  }
}
const eq = (b, ...args) => {
  biquad(b.L, ...args);
  biquad(b.R, ...args);
};

/** Freeverb-style stereo reverb on a send bus (returns the wet signal only). */
function reverb(src, { size = 0.82, damp = 0.35, pre = 0.02 } = {}) {
  const combs = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
  const aps = [556, 441, 341, 225];
  const scale = SR / 44100;
  const out = bus();
  const preN = sec(pre);
  for (const [ch, spread] of [['L', 0], ['R', 23]]) {
    const x = src[ch];
    const y = out[ch];
    for (const c of combs) {
      const d = Math.round((c + spread) * scale);
      const buf = new Float32Array(d);
      let idx = 0;
      let store = 0;
      for (let i = 0; i < N; i++) {
        const o = buf[idx];
        store = o * (1 - damp) + store * damp;
        buf[idx] = (i >= preN ? x[i - preN] : 0) * 0.015 + store * size;
        idx = idx + 1 === d ? 0 : idx + 1;
        y[i] += o;
      }
    }
    for (const a of aps) {
      const d = Math.round((a + spread) * scale);
      const buf = new Float32Array(d);
      let idx = 0;
      for (let i = 0; i < N; i++) {
        const b = buf[idx];
        const o = -y[i] + b;
        buf[idx] = y[i] + b * 0.5;
        idx = idx + 1 === d ? 0 : idx + 1;
        y[i] = o;
      }
    }
  }
  return out;
}

/** Ping-pong delay with a darkening feedback path (returns wet only). */
function pingpong(src, time, feedback = 0.38, tone = 3500) {
  const d = sec(time);
  const out = bus();
  const bl = new Float32Array(d);
  const br = new Float32Array(d);
  const fl = new SVF();
  const fr = new SVF();
  fl.set(tone, 0.6);
  fr.set(tone, 0.6);
  let idx = 0;
  for (let i = 0; i < N; i++) {
    const yl = bl[idx];
    const yr = br[idx];
    const mono = (src.L[i] + src.R[i]) * 0.5;
    // Left echoes feed the right line and vice versa: the repeats bounce.
    bl[idx] = mono + fr.run(yr) * feedback;
    br[idx] = fl.run(yl) * feedback;
    out.L[i] = yl;
    out.R[i] = yr;
    idx = idx + 1 === d ? 0 : idx + 1;
  }
  return out;
}

function mixInto(dst, src, g = 1, duck = null) {
  for (let i = 0; i < N; i++) {
    const k = duck ? g * duck[i] : g;
    dst.L[i] += src.L[i] * k;
    dst.R[i] += src.R[i] * k;
  }
}

function writeWav(file, L, R, { normalizePeak = 0.89 } = {}) {
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
  const g = peak > normalizePeak ? normalizePeak / peak : 1;
  for (let i = 0; i < n; i++) {
    buf.writeInt16LE(Math.round(clamp(L[i] * g, -1, 1) * 32767), 44 + i * 4);
    buf.writeInt16LE(Math.round(clamp(R[i] * g, -1, 1) * 32767), 46 + i * 4);
  }
  writeFileSync(file, buf);
  console.log(`[soundtrack] ${file.replace(tour + '/', '')} ${(n / SR).toFixed(2)}s peak ${peak.toFixed(2)}`);
}

// ── harmony + the hook ────────────────────────────────────────────────────
// vi–IV–I–V in D major (Bm G D A), one chord per bar. Voicings sit in the
// mids and move by step; bass roots are B1 G1 D2 A1 (lowest 49 Hz).
const CHORDS = [
  { root: 35, pad: [54, 59, 62, 66], arp: [66, 71, 74, 78] }, // Bm   F#3 B3 D4 F#4
  { root: 31, pad: [55, 59, 62, 67], arp: [67, 71, 74, 79] }, // G    G3 B3 D4 G4
  { root: 38, pad: [54, 57, 62, 66], arp: [66, 69, 74, 78] }, // D    F#3 A3 D4 F#4
  { root: 33, pad: [52, 57, 61, 64], arp: [64, 69, 73, 76] }, // A    E3 A3 C#4 E4
];
// Outro resolves vi–IV–V–I and holds the tonic under the logo.
const OUTRO_CHORDS = [0, 1, 3, 2, 2, 2];
// The hook: [16th step, length in steps, midi] over four bars. A is the call
// (the 3+3+2 figure is the signature), B the answer.
const HOOK_A = [
  [0, 3, 78], [3, 3, 78], [6, 2, 81], [8, 4, 83], [12, 2, 81], [14, 2, 78],
  [16, 3, 83], [19, 3, 81], [22, 6, 78], [30, 2, 76],
  [32, 3, 78], [35, 3, 78], [38, 2, 81], [40, 4, 86], [44, 2, 85], [46, 2, 81],
  [48, 3, 85], [51, 3, 83], [54, 10, 81],
];
const HOOK_B = [
  [0, 3, 83], [3, 3, 81], [6, 2, 78], [8, 4, 81], [12, 2, 86], [14, 2, 85],
  [16, 3, 83], [19, 3, 81], [22, 2, 79], [24, 6, 78],
  [32, 3, 78], [35, 3, 81], [38, 2, 86], [40, 4, 85], [44, 2, 88], [46, 2, 86],
  [48, 6, 85], [54, 2, 83], [56, 8, 81],
];
// A diatonic third below, for the final chorus.
const SCALE = [62, 64, 66, 67, 69, 71, 73]; // D E F# G A B C# (pitch classes via %12)
function thirdBelow(n) {
  const pcs = SCALE.map((s) => s % 12);
  for (let d = 3; d <= 4; d++) if (pcs.includes((n - d + 120) % 12)) return n - d;
  return n - 3;
}

// ── the arrangement (from the edit) ───────────────────────────────────────
const sections = [];
for (const c of timing.chapters) {
  if (c.music) sections.push({ type: c.music, start: c.startBar, chapter: c.id });
}
sections.forEach((s, i) => (s.end = sections[i + 1]?.start ?? BARS));
const sectionAt = (bar) => sections.find((s) => bar >= s.start && bar < s.end) ?? sections.at(-1);
const chapterStarts = timing.chapters.map((c) => c.startBar).filter((b) => b > 0);
const sectionStarts = new Set(sections.map((s) => s.start).filter((b) => b > 0));
const chordAt = (bar) => {
  const s = sectionAt(bar);
  if (s.type === 'outro') return CHORDS[OUTRO_CHORDS[Math.min(bar - s.start, OUTRO_CHORDS.length - 1)]];
  return CHORDS[((bar % 4) + 4) % 4];
};
const tBar = (bar) => bar * BAR;
const swing = (step) => (step % 2 === 1 ? STEP * 0.12 : 0); // a light 16th shuffle
console.log(`[soundtrack] ${BARS} bars at ${BPM} BPM; sections: ${sections.map((s) => `${s.type}@${s.start}`).join(' ')}`);

// Per-section energy (0..1) drives levels and the pad filter.
const ENERGY = { intro: 0.3, chorus: 1, verse: 0.62, breakdown: 0.25, final: 1.1, outro: 0.55 };

// ── instruments ───────────────────────────────────────────────────────────
const drums = bus();
const kickBus = bus();
const bassBus = bus();
const padBus = bus();
const arpBus = bus();
const leadBus = bus();
const fxBus = bus();
const kicks = [];

function kick(at, vel = 1) {
  kicks.push(at);
  const s0 = sec(at);
  let ph = 0;
  const click = new SVF();
  click.set(3800, 0.9);
  for (let j = 0; j < sec(0.42); j++) {
    const t = j / SR;
    const f = 50 + 120 * Math.exp(-t * 34) + 40 * Math.exp(-t * 9);
    ph += (2 * Math.PI * f) / SR;
    const amp = t < 0.025 ? 1 : Math.exp(-(t - 0.025) * 9.5);
    let v = Math.sin(ph) * amp * 1.5;
    v = Math.tanh(v * 1.6) / Math.tanh(1.6); // harmonics → audible on small speakers
    click.run(rnd());
    v += click.bp * Math.exp(-t * 260) * 0.9 + Math.sin(2 * Math.PI * 1500 * t) * Math.exp(-t * 140) * 0.12;
    put(kickBus, s0 + j, v * vel * 0.7);
  }
}

function clap(at, vel = 1, body = true) {
  const s0 = sec(at);
  const bp = new SVF();
  const bp2 = new SVF();
  bp.set(1250, 1.1);
  bp2.set(1250, 1.1);
  const bursts = [0, 0.009, 0.019, 0.03];
  for (let j = 0; j < sec(0.32); j++) {
    const t = j / SR;
    let env = 0;
    for (const b of bursts) if (t >= b) env = Math.max(env, Math.exp(-(t - b) * (b === 0.03 ? 24 : 170)));
    bp.run(rnd());
    bp2.run(rnd());
    let l = bp.bp * env * 1.4;
    let r = bp2.bp * env * 1.4;
    if (body) {
      const tone = Math.sin(2 * Math.PI * (185 + 40 * Math.exp(-t * 60)) * t) * Math.exp(-t * 32) * 0.3;
      l += tone;
      r += tone;
    }
    if (s0 + j < N) {
      drums.L[s0 + j] += l * vel * 0.5;
      drums.R[s0 + j] += r * vel * 0.5;
    }
  }
}

// 808-style metallic hat: six detuned squares through a band-pass + high-pass.
const HAT_F = [205.3, 304.4, 369.6, 522.7, 540, 800].map((f) => f * 1.45);
function hat(at, vel, open = false, pan = 0.18) {
  const s0 = sec(at);
  const len = sec(open ? 0.32 : 0.06);
  const bp = new SVF();
  bp.set(9500, 0.9);
  let prev = 0;
  for (let j = 0; j < len; j++) {
    const t = j / SR;
    let m = 0;
    for (const f of HAT_F) m += (t * f) % 1 < 0.5 ? 1 : -1;
    const x = m / 6 * 0.6 + rnd() * 0.5;
    bp.run(x);
    const hp = bp.bp - prev * 0.6;
    prev = bp.bp;
    const env = Math.exp(-t * (open ? 11 : 75)) * Math.min(1, t / 0.0008);
    put(drums, s0 + j, hp * env * vel * 0.8, pan);
  }
}

function crash(at, vel = 1) {
  const s0 = sec(at);
  const hl = new SVF();
  const hr = new SVF();
  hl.set(6200, 0.7);
  hr.set(6200, 0.7);
  for (let j = 0; j < sec(2.6); j++) {
    const t = j / SR;
    let m = 0;
    for (const f of HAT_F) m += (t * f * 1.7) % 1 < 0.5 ? 1 : -1;
    hl.run(rnd() + m * 0.05);
    hr.run(rnd() + m * 0.05);
    const env = Math.exp(-t * 1.9) * Math.min(1, t / 0.002) * (0.6 + 0.4 * Math.exp(-t * 14));
    if (s0 + j < N) {
      fxBus.L[s0 + j] += hl.hp * env * vel * 0.16;
      fxBus.R[s0 + j] += hr.hp * env * vel * 0.16;
    }
  }
}

/** A cymbal swell into a downbeat (`len` seconds, ending exactly at `at`). */
function swell(at, len, vel = 1) {
  const s1 = sec(at);
  const s0 = s1 - sec(len);
  const f = new SVF();
  for (let i = Math.max(0, s0); i < s1; i++) {
    const p = (i - s0) / (s1 - s0);
    if ((i & 31) === 0) f.set(1500 + 9000 * p * p, 0.8);
    f.run(rnd());
    put(fxBus, i, f.hp * p ** 3 * vel * 0.2, Math.sin(p * 9) * 0.3);
  }
}

/** Noise riser + rising supersaw tone over `bars` bars into a section start. */
function riser(atBar, bars, vel = 1) {
  const s1 = sec(tBar(atBar));
  const s0 = s1 - sec(bars * BAR);
  const fl = new SVF();
  const fr = new SVF();
  const saws = [new Saw(), new Saw(), new Saw()];
  for (let i = Math.max(0, s0); i < s1; i++) {
    const p = (i - s0) / (s1 - s0);
    if ((i & 31) === 0) {
      fl.set(350 * 2 ** (p * 4.8), 2.2);
      fr.set(370 * 2 ** (p * 4.8), 2.2);
    }
    fl.run(rnd());
    fr.run(rnd());
    const f = 220 * 2 ** (p * 2);
    const tone = (saws[0].tick(f) + saws[1].tick(f * 1.006) + saws[2].tick(f * 0.994)) / 3;
    const g = p ** 2.2 * vel;
    fxBus.L[i] += (fl.bp * 0.5 + tone * 0.05) * g * 0.45;
    fxBus.R[i] += (fr.bp * 0.5 + tone * 0.05) * g * 0.45;
  }
}

/** The hit on a section start: sub drop (≥45 Hz) + a bright noise burst. */
function impact(at, vel = 1) {
  const s0 = sec(at);
  let ph = 0;
  const f2 = new SVF();
  f2.set(2600, 0.7);
  for (let j = 0; j < sec(1.8); j++) {
    const t = j / SR;
    const f = 45 + 75 * Math.exp(-t * 6);
    ph += (2 * Math.PI * f) / SR;
    f2.run(rnd());
    const v = Math.tanh(Math.sin(ph) * 1.4) * Math.exp(-t * 2.6) * 0.5 + f2.bp * Math.exp(-t * 9) * 0.35;
    put(fxBus, s0 + j, v * vel);
  }
  crash(at, vel);
}

function snareRoll(fromBar, toBar, vel = 1) {
  const t0 = tBar(fromBar);
  const t1 = tBar(toBar);
  let t = t0;
  while (t < t1 - 1e-6) {
    const p = (t - t0) / (t1 - t0);
    const div = p < 0.5 ? STEP * 2 : p < 0.85 ? STEP : STEP / 2;
    clap(t, (0.25 + 0.75 * p * p) * vel, false);
    t += div;
  }
}

function bassNote(at, dur, note, vel, bright = 1) {
  const f = midi(note);
  const s0 = sec(at);
  const len = sec(dur + 0.03);
  const o1 = new Saw();
  const o2 = new Saw();
  const flt = new SVF();
  for (let j = 0; j < len; j++) {
    const t = j / SR;
    if ((j & 15) === 0) flt.set((140 + 950 * bright * Math.exp(-t * 13)) * (f / 55) ** 0.5, 1.1);
    const raw = (o1.tick(f) + o2.tick(f * 1.004)) * 0.5;
    const sub = Math.sin(2 * Math.PI * f * t) * 0.35;
    const env = Math.min(1, t / 0.003) * (t < dur ? 1 - 0.25 * Math.min(1, t / 0.12) : 0.75 * Math.exp(-(t - dur) * 160));
    const v = Math.tanh((flt.run(raw) * 1.2 + sub) * 1.3) * env * vel;
    put(bassBus, s0 + j, v * 0.42);
  }
}

// Supersaw pad, one chord per bar, filtered as a whole afterwards.
const DETUNE = [-0.0115, -0.0068, -0.0024, 0, 0.0026, 0.0071, 0.012];
function padChord(at, dur, notes, vel) {
  const s0 = sec(at);
  const len = sec(dur + 0.12);
  for (const note of notes) {
    const f = midi(note);
    const voices = DETUNE.map((d, k) => ({ osc: new Saw(), f: f * (1 + d), pan: ((k - 3) / 3) * 0.85 }));
    for (let j = 0; j < len; j++) {
      const t = j / SR;
      const env = Math.min(1, t / 0.015) * (t < dur ? 1 : Math.exp(-(t - dur) * 45));
      let l = 0;
      let r = 0;
      for (const v of voices) {
        const x = v.osc.tick(v.f);
        l += x * (1 - v.pan);
        r += x * (1 + v.pan);
      }
      const i = s0 + j;
      if (i < N) {
        padBus.L[i] += l * env * vel * 0.03;
        padBus.R[i] += r * env * vel * 0.03;
      }
    }
  }
}

function pluck(at, note, vel, pan = 0, decay = 9, cutoff = 5200, target = arpBus) {
  const f = midi(note);
  const s0 = sec(at);
  const o1 = new Saw();
  const o2 = new Saw();
  const flt = new SVF();
  for (let j = 0; j < sec(0.5); j++) {
    const t = j / SR;
    if ((j & 15) === 0) flt.set(320 + cutoff * Math.exp(-t * 32), 2.1);
    const raw = o1.tick(f) * 0.6 + o2.tick(f * 2.002) * 0.25;
    const env = Math.min(1, t / 0.0015) * Math.exp(-t * decay);
    put(target, s0 + j, flt.run(raw) * env * vel * 0.32, pan);
  }
}

function leadNote(at, dur, note, vel, prevNote, gain = 1) {
  const f1 = midi(note);
  const f0 = prevNote ? midi(prevNote) : f1;
  const s0 = sec(at);
  const len = sec(dur + 0.25);
  const a = new Saw();
  const b = new Saw();
  const c = new Saw();
  const flt = new SVF();
  for (let j = 0; j < len; j++) {
    const t = j / SR;
    const glide = Math.min(1, t / 0.035);
    const vib = t > 0.22 ? 1 + 0.0055 * Math.sin(2 * Math.PI * 5.6 * (t - 0.22)) * Math.min(1, (t - 0.22) / 0.2) : 1;
    const f = (f0 + (f1 - f0) * glide) * vib;
    if ((j & 15) === 0) flt.set(1900 + 4200 * Math.exp(-t * 7), 1.25);
    const raw = a.tick(f * 1.0046) * 0.5 + b.tick(f * 0.9954) * 0.5 + c.tick(f * 0.5) * 0.22;
    const env = Math.min(1, t / 0.006) * (t < dur ? 0.78 + 0.22 * Math.exp(-t * 6) : 0.78 * Math.exp(-(t - dur) * 22));
    const bell = Math.sin(2 * Math.PI * f * 2 * t) * Math.exp(-t * 14) * 0.18; // a little glass on the attack
    put(leadBus, s0 + j, (flt.run(raw) + bell) * env * vel * 0.3 * gain, 0);
  }
}

function playHook(hook, startBar, { vel = 1, harmony = false, octave = 0, as = 'lead' } = {}) {
  let prev = null;
  let prevEnd = -1;
  for (const [step, len, n0] of hook) {
    const n = n0 + octave;
    const at = tBar(startBar) + step * STEP + swing(step);
    const dur = len * STEP * 0.92;
    if (as === 'lead') {
      leadNote(at, dur, n, vel * (step % 4 === 0 ? 1 : 0.9), step === prevEnd ? prev : null);
      if (harmony) leadNote(at, dur, thirdBelow(n), vel * 0.55, null);
    } else pluck(at, n, vel, 0, 5.5, 4200, leadBus);
    prev = n;
    prevEnd = step + len;
  }
}

// ── write the parts ───────────────────────────────────────────────────────
for (let bar = 0; bar < BARS; bar++) {
  const s = sectionAt(bar);
  const rel = bar - s.start;
  const len = s.end - s.start;
  const ch = chordAt(bar);
  const at = tBar(bar);
  const lastBarOfSection = rel === len - 1;
  const nextSection = sections.find((x) => x.start === s.end);

  // Pad: every bar; level follows the energy.
  const padVel = { intro: 0.75, chorus: 1, verse: 0.6, breakdown: 0.62, final: 1.1, outro: 0.85 }[s.type];
  const holdOut = s.type === 'outro' && rel >= 3;
  if (!(s.type === 'outro' && rel > 3)) padChord(at, holdOut ? BAR * 3 + TAIL - 0.2 : BAR, ch.pad, padVel);

  // Kick + clap + hats.
  const fourFloor = s.type === 'chorus' || s.type === 'final' || s.type === 'verse' || (s.type === 'intro' && rel >= 2) || (s.type === 'outro' && rel < 3);
  const gapBeforeDrop = nextSection && (nextSection.type === 'chorus' || nextSection.type === 'final') && lastBarOfSection && s.type !== 'verse';
  for (let b = 0; b < 4; b++) {
    const bt = at + b * BEAT;
    if (gapBeforeDrop && b === 3) continue; // half a beat of air before the drop
    if (fourFloor) kick(bt, s.type === 'intro' ? 0.75 : 1);
  }
  if (s.type === 'outro' && rel === 3) kick(at, 1);
  if (fourFloor && s.type !== 'intro') {
    clap(at + BEAT, s.type === 'verse' ? 0.8 : 1);
    clap(at + 3 * BEAT, s.type === 'verse' ? 0.8 : 1);
  }
  if (fourFloor && s.type !== 'intro') {
    for (let st = 0; st < 16; st++) {
      const t = at + st * STEP + swing(st);
      const accent = st % 4 === 0 ? 0.55 : st % 4 === 2 ? 0.9 : 0.42;
      if (s.type === 'verse') {
        if (st % 2 === 0 || st % 4 === 3) hat(t, accent * 0.8, false, st % 2 ? 0.22 : -0.12);
      } else {
        if (st % 4 === 2) hat(t, 0.75, true, 0.1);
        else hat(t, accent, false, st % 2 ? 0.25 : -0.15);
      }
    }
  }
  if (s.type === 'intro' && rel >= 1) for (let st = 2; st < 16; st += 4) hat(at + st * STEP, 0.35 + 0.15 * rel, false);

  // Bass.
  if (s.type === 'chorus' || s.type === 'final' || (s.type === 'outro' && rel < 3)) {
    for (let e = 0; e < 8; e++) {
      const note = ch.root + (e === 7 ? 12 : e === 3 && bar % 2 ? 12 : 0);
      bassNote(at + e * BEAT * 0.5, BEAT * 0.42, note, e % 2 ? 1 : 0.85, s.type === 'final' ? 1.2 : 1);
    }
  } else if (s.type === 'verse') {
    for (const [st, oct, l] of [[0, 0, 2.5], [3, 0, 2], [6, 12, 1.6], [8, 0, 1.6], [10, 0, 2], [14, 12, 1.6]]) bassNote(at + st * STEP + swing(st), STEP * l, ch.root + oct, 0.9, 0.85);
  } else if (s.type === 'outro' && rel === 3) {
    bassNote(at, BAR * 2.4, ch.root + 12, 0.8, 0.5);
  } else if (s.type === 'breakdown' && rel >= 2) {
    bassNote(at, BAR * 0.95, ch.root + 12, 0.5, 0.4);
  }

  // Arp: 16ths over the chord, up an octave from the pad.
  if (!(s.type === 'outro' && rel > 4)) {
    const pat = [0, 1, 2, 3, 2, 1, 2, 3, 0, 1, 2, 3, 2, 3, 1, 2];
    const arpVel = { intro: 0.55 + 0.15 * rel, chorus: 0.55, verse: 0.95, breakdown: 0.5, final: 0.6, outro: 0.7 - 0.12 * Math.max(0, rel - 2) }[s.type];
    for (let st = 0; st < 16; st++) {
      const accent = st % 4 === 0 ? 1 : st % 2 === 0 ? 0.8 : 0.62;
      pluck(at + st * STEP + swing(st), ch.arp[pat[st]], arpVel * accent, st % 2 ? 0.35 : -0.35);
    }
  }
}

// The hook: lead in the choruses, a pluck in the breakdown and the outro.
for (const s of sections) {
  if (s.type === 'chorus' || s.type === 'final') {
    let k = 0;
    // Leave the last 4 bars of a 20-bar chorus to the arp (a post-chorus breath).
    const stopAt = s.end - s.start >= 20 ? s.end - 4 : s.end;
    for (let bar = s.start; bar + 4 <= stopAt; bar += 4, k++) playHook(k % 2 ? HOOK_B : HOOK_A, bar, { vel: s.type === 'final' ? 1.08 : 1, harmony: s.type === 'final' });
    if (stopAt < s.end) playHook(HOOK_A.slice(0, 6), stopAt, { vel: 0.5, as: 'pluck', octave: 12 });
  } else if (s.type === 'breakdown') {
    playHook(HOOK_A, s.start, { vel: 0.75, as: 'pluck' });
  } else if (s.type === 'verse') {
    // An answer figure on the pluck every second loop keeps the hook in the ear.
    for (let bar = s.start + 2; bar < s.end; bar += 4) playHook(HOOK_A.slice(0, 6), bar, { vel: 0.55, as: 'pluck', octave: 12 });
  } else if (s.type === 'outro') {
    playHook(HOOK_A.slice(0, 10), s.start, { vel: 0.85, as: 'pluck' });
    // The last word: the hook's tag on the tonic, as the logo lands.
    leadNote(tBar(s.start + 3), BAR * 1.6, 78, 0.7, null);
    leadNote(tBar(s.start + 3), BAR * 1.6, 74, 0.45, null);
  }
}

// Transitions: risers + impacts into sections, fills + crashes on chapter starts.
for (const b of chapterStarts) {
  const s = sectionAt(b);
  if (sectionStarts.has(b)) {
    if (s.type === 'chorus' || s.type === 'final') {
      riser(b, s.type === 'final' ? 2 : b === sections[1].start ? 4 : 2, s.type === 'final' ? 1.1 : 0.9);
      snareRoll(b - (s.type === 'final' ? 2 : 1), b - 0.125, 0.9);
      impact(tBar(b), 1);
    } else if (s.type === 'outro') {
      swell(tBar(b), BEAT * 2, 1);
      crash(tBar(b), 0.9);
    } else {
      swell(tBar(b), BEAT, 0.8);
      crash(tBar(b), 0.8);
    }
  } else {
    // A one-beat clap fill and a short swell, then a crash on the chapter's downbeat.
    const t = tBar(b) - BEAT;
    for (let k = 0; k < 4; k++) clap(t + k * STEP, 0.35 + 0.15 * k, false);
    swell(tBar(b), BEAT, 0.6);
    crash(tBar(b), 0.55);
  }
}
// The tonic hit under the outro's logo lockup.
{
  const o = sections.find((s) => s.type === 'outro');
  if (o) impact(tBar(o.start + 3), 0.8);
}

// ── mix ───────────────────────────────────────────────────────────────────
// Pad filter: the intro opens it (the build), each section has its brightness,
// a slow LFO keeps it alive; the breakdown sweeps with more resonance.
{
  const fl = new SVF();
  const fr = new SVF();
  for (let i = 0; i < N; i++) {
    if ((i & 31) === 0) {
      const t = i / SR;
      const bar = t / BAR;
      const s = sectionAt(Math.min(BARS - 1, Math.floor(bar)));
      const p = (bar - s.start) / (s.end - s.start);
      const lfo = 1 + 0.12 * Math.sin(2 * Math.PI * t * 0.125);
      let fc;
      let q = 1.5;
      if (s.type === 'intro') fc = 320 * 2 ** (p * 3.4);
      else if (s.type === 'breakdown') {
        fc = 700 * 2 ** (p * 2.6);
        q = 2.6;
      } else fc = { chorus: 4200, verse: 1700, final: 5200, outro: 2600 * (1 - 0.5 * p) }[s.type];
      fl.set(fc * lfo, q);
      fr.set(fc * lfo * 1.03, q);
    }
    padBus.L[i] = fl.run(padBus.L[i]);
    padBus.R[i] = fr.run(padBus.R[i]);
  }
  eq(padBus, 'hp', 170, 0.7); // keep the pad out of the bass
  eq(arpBus, 'hp', 220, 0.7);
  eq(leadBus, 'hp', 240, 0.7);
  eq(bassBus, 'hp', 38, 0.7);
  eq(bassBus, 'peak', 160, 1.0, -2.5); // clear room for the kick's punch
  eq(kickBus, 'hp', 36, 0.7);
  eq(fxBus, 'hp', 40, 0.7);
}

// Side-chain: a unit "pump" curve from the kick triggers, scaled per stem.
const pump = new Float32Array(N).fill(1);
for (const k of kicks) {
  const s0 = sec(k);
  for (let j = 0; j < sec(0.24) && s0 + j < N; j++) {
    const t = j / SR;
    const dip = t < 0.004 ? t / 0.004 : (1 - (t - 0.004) / 0.236) ** 2;
    pump[s0 + j] = Math.min(pump[s0 + j], 1 - dip);
  }
}
const duck = (depth) => pump.map((g) => 1 - depth * (1 - g));

const master = bus();
mixInto(master, kickBus, 1.0);
mixInto(master, drums, 1.15);
mixInto(master, bassBus, 0.85, duck(0.7));
mixInto(master, padBus, 1.7, duck(0.62));
mixInto(master, arpBus, 1.4, duck(0.35));
mixInto(master, leadBus, 1.6, duck(0.15));
mixInto(master, fxBus, 0.9);
// Sends: a room for the drums, a hall for the musical parts, echoes for arp + lead.
{
  const send = bus();
  mixInto(send, drums, 0.35);
  mixInto(send, padBus, 0.25);
  mixInto(send, arpBus, 0.5);
  mixInto(send, leadBus, 0.55);
  mixInto(send, fxBus, 0.4);
  eq(send, 'hp', 350, 0.7);
  eq(send, 'lp', 9000, 0.7);
  const wet = reverb(send, { size: 0.84, damp: 0.4, pre: 0.025 });
  mixInto(master, wet, 0.55, duck(0.3));
  const echoSend = bus();
  mixInto(echoSend, arpBus, 0.4);
  mixInto(echoSend, leadBus, 0.35);
  eq(echoSend, 'hp', 400, 0.7);
  const echo = pingpong(echoSend, BEAT * 0.75, 0.36, 3800);
  mixInto(master, echo, 0.5, duck(0.4));
}
// Master: rumble filter, a touch of air, glue compression, soft limiting.
eq(master, 'hp', 34, 0.7);
eq(master, 'hp', 34, 0.7);
eq(master, 'lowshelf', 220, 0.7, -1.0);
eq(master, 'peak', 2800, 0.8, 1.2);
eq(master, 'highshelf', 8000, 0.7, 2.0);
{
  let env = 0;
  const att = Math.exp(-1 / (0.008 * SR));
  const rel = Math.exp(-1 / (0.18 * SR));
  const thr = 0.35;
  const drive = 0.5; // headroom into the glue stage, so the sections keep their contrast
  for (let i = 0; i < N; i++) {
    master.L[i] *= drive;
    master.R[i] *= drive;
    const x = Math.max(Math.abs(master.L[i]), Math.abs(master.R[i]));
    env = x > env ? att * env + (1 - att) * x : rel * env + (1 - rel) * x;
    const g = env > thr ? (thr + (env - thr) / 2.2) / env : 1; // ~2.2:1 above threshold
    const fade = Math.min(1, i / (SR * 0.02), (N - i) / (SR * TAIL));
    master.L[i] = Math.tanh(master.L[i] * g * 1.25) * fade;
    master.R[i] = Math.tanh(master.R[i] * g * 1.25) * fade;
  }
}
writeWav(join(outDir, 'music.wav'), master.L, master.R, { normalizePeak: 0.95 });

// Two-pass linear loudness normalization (keeps the arrangement's dynamics).
{
  const src = join(outDir, 'music.wav');
  const tmp = join(outDir, 'music.norm.wav');
  const target = 'I=-16:TP=-1.5:LRA=11';
  const report = execFileSync('sh', ['-c', `"${FF}" -hide_banner -i "${src}" -af loudnorm=${target}:print_format=json -f null - 2>&1`], { encoding: 'utf8' });
  const m = JSON.parse(report.slice(report.lastIndexOf('{'), report.lastIndexOf('}') + 1));
  execFileSync(FF, ['-y', '-v', 'error', '-i', src, '-af', `loudnorm=${target}:measured_I=${m.input_i}:measured_TP=${m.input_tp}:measured_LRA=${m.input_lra}:measured_thresh=${m.input_thresh}:offset=${m.target_offset}:linear=true`, '-ar', '48000', '-ac', '2', tmp]);
  renameSync(tmp, src);
  console.log(`[soundtrack] music.wav: measured ${m.input_i} LUFS / ${m.input_tp} dBTP → −16 LUFS (linear)`);
}

// ── sfx (tuned to D major so they sit inside the score) ───────────────────
function sfx(name, secs, fn) {
  const n = Math.round(secs * SR);
  const L = new Float32Array(n);
  const R = new Float32Array(n);
  fn(L, R, n);
  writeWav(join(outDir, `sfx-${name}.wav`), L, R);
}
sfx('click', 0.08, (L, R, n) => {
  for (let i = 0; i < n; i++) {
    const t = i / SR;
    const v = (Math.sin(2 * Math.PI * midi(93) * t) * 0.5 + rnd() * 0.12) * Math.exp(-t * 110) * 0.45;
    L[i] = v;
    R[i] = v;
  }
});
// Whoosh for whip-pans: band-passed noise sweeping up then down, panned across.
sfx('whoosh', 0.5, (L, R, n) => {
  const f = new SVF();
  for (let i = 0; i < n; i++) {
    const p = i / n;
    if ((i & 15) === 0) f.set(600 + 5200 * Math.sin(Math.PI * p) ** 2, 1.4);
    f.run(rnd());
    const env = Math.sin(Math.PI * p ** 0.7) ** 2;
    L[i] = f.bp * env * 0.5 * (1.1 - p);
    R[i] = f.bp * env * 0.5 * (0.1 + p);
  }
});
// Tick for callouts: a soft two-note glass chime, A6 → D7.
sfx('tick', 0.45, (L, R, n) => {
  for (let i = 0; i < n; i++) {
    const t = i / SR;
    const a = Math.sin(2 * Math.PI * midi(93) * t) * Math.exp(-t * 20);
    const b = t > 0.05 ? Math.sin(2 * Math.PI * midi(98) * (t - 0.05)) * Math.exp(-(t - 0.05) * 16) : 0;
    const v = (a + b * 0.8) * 0.14 * Math.min(1, t / 0.002);
    L[i] = v;
    R[i] = v;
  }
});
sfx('riser', 2, (L, R, n) => {
  const f = new SVF();
  for (let i = 0; i < n; i++) {
    const p = i / n;
    if ((i & 31) === 0) f.set(400 * 2 ** (p * 4), 2);
    f.run(rnd());
    L[i] = R[i] = f.bp * p ** 2 * 0.4;
  }
});
sfx('impact', 1.6, (L, R, n) => {
  let ph = 0;
  for (let i = 0; i < n; i++) {
    const t = i / SR;
    ph += (2 * Math.PI * (45 + 70 * Math.exp(-t * 7))) / SR;
    L[i] = R[i] = Math.tanh(Math.sin(ph) * 1.4) * Math.exp(-t * 3) * 0.6;
  }
});
