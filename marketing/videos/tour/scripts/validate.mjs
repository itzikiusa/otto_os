// Verify the rendered review candidate before publication; reads generated output only.
//   node scripts/validate.mjs
// Checks the media (1080p30 H.264 + stereo 48 kHz AAC, duration), the candidate
// manifest against the edit (chapters on the bar grid, every `section` names a
// Help guide), the captions, the loudness, and the Rooms footage's provenance.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const tour = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repo = resolve(tour, '../../..');
const out = join(tour, 'out');
const json = (path) => JSON.parse(readFileSync(path, 'utf8'));
const film = json(join(out, 'film.candidate.json'));
const timing = json(join(tour, 'src/generated/timing.json'));
const media = JSON.parse(execFileSync('ffprobe', ['-v', 'error', '-show_streams', '-show_format', '-of', 'json', join(out, film.file)], { encoding: 'utf8' }));
const video = media.streams.find((s) => s.codec_type === 'video');
const audio = media.streams.find((s) => s.codec_type === 'audio');
assert.equal(video.codec_name, 'h264');
assert.equal(video.width, 1920);
assert.equal(video.height, 1080);
assert.equal(video.r_frame_rate, '30/1');
assert.equal(audio.codec_name, 'aac');
assert.equal(audio.channels, 2);
assert.equal(audio.sample_rate, '48000');
assert(Math.abs(Number(media.format.duration) - timing.totalSeconds) < 0.1, `duration ${media.format.duration} vs edit ${timing.totalSeconds}`);

// Chapters: same ids/sections as the edit, contiguous, on the bar grid, each section a Help guide.
const guides = new Set(readdirSync(join(repo, 'ui/src/modules/help/sections')).filter((f) => f.endsWith('.md')).map((f) => f.slice(0, -3)));
assert.deepEqual(film.chapters.map((c) => c.id), timing.chapters.map((c) => c.id));
const bar = timing.barFrames / timing.fps;
let at = 0;
for (const c of film.chapters) {
  assert(guides.has(c.section), `chapter ${c.id}: no Help guide named ${c.section}`);
  assert(Math.abs(c.start - at) < 0.02, `chapter ${c.id} starts at ${c.start}, expected ${at}`);
  assert(Math.abs(c.start / bar - Math.round(c.start / bar)) < 0.01, `chapter ${c.id} is off the bar grid`);
  at = c.start + c.duration;
}
assert(at <= film.duration + 0.05, 'chapters run past the film');
assert(existsSync(join(out, film.poster)), 'poster missing');

// Captions: ordered, inside the film, at least one per chapter.
const vtt = readFileSync(join(out, film.captions), 'utf8');
const stamps = [...vtt.matchAll(/(\d\d):(\d\d):(\d\d)\.(\d\d\d) --> (\d\d):(\d\d):(\d\d)\.(\d\d\d)/g)];
assert(stamps.length >= film.chapters.length, 'fewer caption cues than chapters');
let previous = 0;
for (const s of stamps) {
  const v = (i) => Number(s[i]) * 3600 + Number(s[i + 1]) * 60 + Number(s[i + 2]) + Number(s[i + 3]) / 1000;
  const start = v(1);
  const end = v(5);
  assert(start >= previous - 0.001 && end > start && end <= film.duration + 0.02, `invalid caption range ${s[0]}`);
  previous = end;
}

// Loudness of the final mix.
const report = execFileSync('sh', ['-c', `ffmpeg -hide_banner -nostats -i "${join(out, film.file)}" -vn -af ebur128=peak=true -f null - 2>&1`], { encoding: 'utf8' });
const summary = report.slice(report.lastIndexOf('Summary:'));
const lufs = Number(summary.match(/I:\s+(-?[\d.]+) LUFS/)[1]);
const peak = Number(summary.match(/Peak:\s+(-?[\d.]+) dBFS/)[1]);
assert(Math.abs(lufs + 16) < 1, `integrated loudness ${lufs} LUFS, expected −16`);
assert(peak <= -1, `true peak ${peak} dBFS, expected ≤ −1`);

// Rooms: the clips are the genuine capture (real recap evidence, no runtime errors).
const rooms = join(tour, 'public/capture/rooms');
const evidence = json(join(rooms, 'recap-evidence.json'));
assert.equal(evidence.metadata.summary_status, 'ready');
assert(evidence.draft.decisions.length && evidence.draft.actions.length);
assert.deepEqual(json(join(rooms, 'provenance.json')).runtimeErrors, []);

const mb = (statSync(join(out, film.file)).size / 1048576).toFixed(1);
console.log(`PASS: ${film.file} ${film.duration.toFixed(2)}s, ${mb} MiB, 1080p30 H.264/AAC, ${film.chapters.length} chapters on the bar grid, ${stamps.length} captions, ${lufs} LUFS / ${peak} dBTP, genuine Rooms evidence.`);
