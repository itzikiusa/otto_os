// Verify the review artifact before publication; reads generated output only.
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {readFileSync, statSync, existsSync} from 'node:fs';
import {dirname, join, resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
const tour = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const out = join(tour, 'out'), capture = join(tour, 'public/capture/rooms');
const json = path => JSON.parse(readFileSync(path, 'utf8'));
const film = json(join(out, 'film.candidate.json'));
const media = JSON.parse(execFileSync('ffprobe', ['-v', 'error', '-show_streams', '-show_format', '-of', 'json', join(out, film.file)], {encoding: 'utf8'}));
const video = media.streams.find(s => s.codec_type === 'video');
const audio = media.streams.find(s => s.codec_type === 'audio');
assert.equal(video.codec_name, 'h264'); assert.equal(video.width, 1920); assert.equal(video.height, 1080); assert.equal(video.r_frame_rate, '30/1');
assert.equal(audio.codec_name, 'aac'); assert.equal(audio.channels, 2); assert.equal(audio.sample_rate, '48000');
assert(Math.abs(Number(media.format.duration) - film.duration) < 1 / 30);
assert.equal(film.chapters.filter(c => c.id === 'rooms').length, 1);
assert.equal(film.chapters.at(-1).id, 'outro');
for (const chapter of film.chapters) assert(chapter.start + chapter.duration <= film.duration + .02, `Chapter extends beyond film: ${chapter.id}`);
assert(existsSync(join(out, film.poster))); assert(existsSync(join(out, film.captions)));
const stamps = [...readFileSync(join(out, film.captions), 'utf8').matchAll(/(\d\d):(\d\d):(\d\d)\.(\d\d\d) --> (\d\d):(\d\d):(\d\d)\.(\d\d\d)/g)];
assert(stamps.length >= 50, 'Missing original or Rooms captions');
let previous = 0;
for (const stamp of stamps) {
  const value = i => Number(stamp[i]) * 3600 + Number(stamp[i + 1]) * 60 + Number(stamp[i + 2]) + Number(stamp[i + 3]) / 1000;
  const start = value(1), end = value(5);
  assert(start >= previous - .001 && end > start && end <= film.duration + .02, `Invalid caption range: ${stamp[0]}`); previous = end;
}
const evidence = json(join(capture, 'recap-evidence.json'));
assert.equal(evidence.metadata.summary_status, 'ready');
assert(evidence.draft.decisions.length && evidence.draft.actions.length && evidence.draft.open_questions.length);
const types = new Set(evidence.events.map(e => e.payload.type));
for (const type of ['speech', 'screen', 'annotation', 'terminal', 'chat', 'presentation']) assert(types.has(type), `No real ${type} evidence`);
const ids = new Set(evidence.events.map(e => e.seq));
for (const id of evidence.draft.source_event_ids) assert(ids.has(id));
const provenance = json(join(capture, 'provenance.json')); assert.deepEqual(provenance.runtimeErrors, []);
console.log(`PASS: ${film.duration.toFixed(2)}s, ${(statSync(join(out, film.file)).size / 1024 / 1024).toFixed(1)} MiB, 1080p30 H.264/AAC, ${stamps.length} caption cues, real recap evidence and draft, no browser runtime errors.`);
