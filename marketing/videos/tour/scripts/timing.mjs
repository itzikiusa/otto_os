// The edit timeline, on the music's bar grid. script/chapters.json gives each
// chapter a length in bars (120 BPM → a beat is exactly 15 frames at 30 fps, a
// bar 60), so every chapter start, cut and caption lands on the beat. Writes:
//   src/generated/timing.json        the timeline Remotion and the score both read (commit it)
//   src/generated/otto-tour-<ed>.vtt WebVTT captions, one or more short lines per chapter
//
//   node scripts/timing.mjs
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const tour = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const plan = JSON.parse(readFileSync(join(tour, 'script/chapters.json'), 'utf8'));
const fps = plan.fps;
const beatFrames = (60 / plan.bpm) * fps;
if (!Number.isInteger(beatFrames)) throw new Error(`${plan.bpm} BPM at ${fps} fps is not a whole number of frames per beat`);
const barFrames = beatFrames * 4;

let bar = 0;
const chapters = plan.chapters.map((c) => {
  if (!Number.isInteger(c.bars) || c.bars < 1) throw new Error(`chapter ${c.id}: bars must be a positive integer`);
  const startFrame = bar * barFrames;
  const frames = c.bars * barFrames;
  // Captions: equal shares of the chapter, or explicit beat lengths (one per line).
  const beats = c.captionBeats ?? c.captions.map(() => (c.bars * 4) / c.captions.length);
  if (beats.length !== c.captions.length) throw new Error(`chapter ${c.id}: captionBeats must match captions`);
  if (Math.abs(beats.reduce((a, b) => a + b, 0) - c.bars * 4) > 1e-6) throw new Error(`chapter ${c.id}: captionBeats must sum to ${c.bars * 4}`);
  let at = startFrame / fps;
  const lines = c.captions.map((text, i) => {
    const len = (beats[i] * beatFrames) / fps;
    const line = { text, start: round(at + (i === 0 ? 0.25 : 0)), end: round(at + len - 0.15) };
    at += len;
    return line;
  });
  const out = { id: c.id, section: c.section, title: c.title, music: c.music ?? null, startBar: bar, bars: c.bars, startFrame, frames, lines };
  bar += c.bars;
  return out;
});

const totalFrames = bar * barFrames;
const timing = {
  edition: plan.edition,
  bpm: plan.bpm,
  fps,
  beatFrames,
  barFrames,
  totalBars: bar,
  totalFrames,
  totalSeconds: totalFrames / fps,
  chapters,
};
const gen = join(tour, 'src/generated');
mkdirSync(gen, { recursive: true });
writeFileSync(join(gen, 'timing.json'), JSON.stringify(timing, null, 2) + '\n');

const clock = (sec) => {
  const ms = Math.round(sec * 1000);
  const p = (n, w = 2) => String(n).padStart(w, '0');
  return `${p(Math.floor(ms / 3600000))}:${p(Math.floor(ms / 60000) % 60)}:${p(Math.floor(ms / 1000) % 60)}.${p(ms % 1000, 3)}`;
};
const cues = chapters.flatMap((c) => c.lines).map((l, i) => `${i + 1}\n${clock(l.start)} --> ${clock(l.end)}\n${l.text}`);
writeFileSync(join(gen, `otto-tour-${plan.edition}.vtt`), `WEBVTT\n\n${cues.join('\n\n')}\n`);

console.log(`[timing] ${chapters.length} chapters, ${bar} bars, ${(totalFrames / fps).toFixed(1)}s at ${plan.bpm} BPM`);
for (const c of chapters) console.log(`  ${String(c.startBar).padStart(3)}  ${(c.startFrame / fps).toFixed(1).padStart(6)}s  ${c.id}${c.music ? `  ♪ ${c.music}` : ''}`);

function round(x) {
  return Math.round(x * 1000) / 1000;
}
