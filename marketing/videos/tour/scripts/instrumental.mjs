// Replace a previously rendered tour's audio, preserving its video and timing.
// Usage: node scripts/instrumental.mjs /path/to/otto-tour.mp4
import { execFileSync } from 'node:child_process';
import { mkdirSync, readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const tour = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const input = process.argv[2];
if (!input) throw new Error('Provide the existing tour MP4 as the first argument.');
const timing = JSON.parse(readFileSync(join(tour, 'src/generated/timing.json'), 'utf8'));
const out = join(tour, 'out');
mkdirSync(out, { recursive: true });
const final = join(out, 'otto-tour-instrumental-20260925.mp4');
if (resolve(input) === final) throw new Error('Input must differ from the instrumental output.');
execFileSync(process.env.FFMPEG ?? 'ffmpeg', [
  '-y', '-v', 'error', '-i', resolve(input), '-i', join(tour, 'public/audio/music.wav'),
  '-map', '0:v:0', '-map', '1:a:0', '-c:v', 'copy',
  '-af', `afade=t=in:d=0.67,afade=t=out:st=${timing.totalSeconds - 2}:d=2,loudnorm=I=-16:TP=-1.5:LRA=11`,
  '-c:a', 'aac', '-b:a', '192k', '-ar', '48000', '-t', String(timing.totalSeconds),
  '-movflags', '+faststart', final,
], { stdio: 'inherit' });
console.log(final);
