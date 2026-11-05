// Rebuild public/capture/rooms/ from the PUBLISHED genuine Rooms capture
// (the 2026-09-28 walkthrough, script/rooms-validation.md) when the raw clips
// of scripts/capture-rooms.mjs are not on this machine. Nothing is re-shot or
// re-synthesized: each clip is cut verbatim out of that movie at its caption
// cue, center-cropped above the movie's burned-in caption band (1721×968, the
// largest 16:9 window clear of it) and scaled back to 1920×1080.
//
//   node scripts/reuse-rooms.mjs [path/to/otto-tour-rooms-20260928.mp4]
//
// Without a path the movie is fetched from the `walkthroughs` release. Its
// SHA-256 must match the one recorded in script/rooms-validation.md, so the
// footage is provably the validated capture. Writes provenance.json with
// `reused` set; validate.mjs then checks the hash instead of the recap
// evidence (which the original capture validated, see rooms-validation.md).
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const tour = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const SOURCE = 'otto-tour-rooms-20260928.mp4';
const ffmpeg = process.env.FFMPEG ?? 'ffmpeg';
const rooms = join(tour, 'public/capture/rooms');
const cache = join(tour, '.cache');
mkdirSync(rooms, { recursive: true });
mkdirSync(cache, { recursive: true });

const recorded = readFileSync(join(tour, 'script/rooms-validation.md'), 'utf8').match(/^SHA-256: `([0-9a-f]{64})`/m)?.[1];
if (!recorded) throw new Error('script/rooms-validation.md records no SHA-256');

let src = process.argv[2];
if (!src) {
  src = join(cache, SOURCE);
  if (!existsSync(src)) execFileSync('gh', ['release', 'download', 'walkthroughs', '-p', SOURCE, '-D', cache, '--clobber'], { stdio: 'inherit' });
}
const sha = createHash('sha256').update(readFileSync(src)).digest('hex');
if (sha !== recorded) throw new Error(`${src}: SHA-256 ${sha} is not the validated capture (${recorded})`);

// The movie's Rooms cues (otto-tour-rooms.vtt): one clip per cue.
const CUES = [
  ['01-start', 165.2, 6.5],
  ['02-admit', 171.7, 6.5],
  ['03-consent', 178.2, 6.5],
  ['04-screen', 184.7, 6.5],
  ['05-draw', 191.2, 6.5],
  ['06-presenters', 197.7, 6.5],
  ['07-request-control', 204.2, 6.5],
  ['08-take-back', 210.7, 6.5],
  ['09-transcript', 217.2, 8],
  ['10-evidence', 225.2, 6.53],
  ['11-summary', 231.73, 10],
];
// The caption band starts at y=968; keep a 16:9 window above it.
const CROP = 'crop=1721:968:100:0,scale=1920:1080:flags=lanczos,setsar=1';
for (const [id, start, len] of CUES) {
  execFileSync(ffmpeg, ['-hide_banner', '-loglevel', 'error', '-y', '-ss', String(start), '-t', String(len), '-i', src, '-an', '-vf', CROP, '-c:v', 'libx264', '-preset', 'slow', '-crf', '16', '-pix_fmt', 'yuv420p', '-r', '30', join(rooms, `${id}.mp4`)]);
  console.log(`[rooms] ${id}.mp4 ← ${SOURCE} @ ${start}s`);
}
writeFileSync(
  join(rooms, 'provenance.json'),
  JSON.stringify(
    {
      reused: true,
      source: SOURCE,
      release: 'walkthroughs',
      sha256: sha,
      validation: 'script/rooms-validation.md',
      transform: CROP,
      clips: CUES.map(([id, start, seconds]) => ({ id, start, seconds })),
      // The original capture's browser runtime errors (rooms-validation.md: zero).
      runtimeErrors: [],
    },
    null,
    2,
  ) + '\n',
);
console.log(`[rooms] ${CUES.length} clips from the validated capture (sha256 ${sha.slice(0, 12)}…)`);
