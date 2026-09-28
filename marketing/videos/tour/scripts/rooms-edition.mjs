// Splice genuine room footage into the released film and replace its full score.
// Writes review candidates only. Never publishes or edits the active app manifest.
import {execFileSync} from 'node:child_process';
import {mkdirSync, readFileSync, writeFileSync} from 'node:fs';
import {dirname, join, resolve} from 'node:path';
import {fileURLToPath, pathToFileURL} from 'node:url';
const tour = resolve(dirname(fileURLToPath(import.meta.url)), '..'), repo = resolve(tour, '../../..');
const {chromium} = await import(pathToFileURL(join(repo, 'ui/node_modules/playwright/index.mjs')));
const out = join(tour, 'out'), capture = join(tour, 'public/capture/rooms'), edit = join(tour, '.cache/rooms-edit');
mkdirSync(out, {recursive: true}); mkdirSync(edit, {recursive: true});
const source = resolve(process.argv[2] ?? join(out, 'otto-tour-instrumental-20260925.mp4'));
const original = JSON.parse(readFileSync(join(tour, 'script/baseline-20260925/film.json'), 'utf8'));
const outro = original.chapters.find(c => c.id === 'outro');
const shots = JSON.parse(readFileSync(join(capture, 'shots.json'), 'utf8'));
if (shots.length !== 11) throw new Error('All 11 genuine room workflow shots must complete before rendering.');
const evidence = JSON.parse(readFileSync(join(capture, 'recap-evidence.json'), 'utf8'));
if (!evidence.draft || evidence.metadata.summary_status !== 'ready') throw new Error('A real completed recap draft is required.');
const ff = args => execFileSync(process.env.FFMPEG ?? 'ffmpeg', ['-hide_banner', '-loglevel', 'error', '-y', ...args], {stdio: 'inherit'});
const dimensions = file => JSON.parse(execFileSync('ffprobe', ['-v', 'error', '-select_streams', 'v:0', '-show_entries', 'stream=width,height,r_frame_rate', '-of', 'json', file], {encoding: 'utf8'})).streams[0];
for (const file of [source, ...shots.map(shot => join(capture, shot.file))]) {
  const stream = dimensions(file); if (stream.width !== 1920 || stream.height !== 1080 || stream.r_frame_rate !== '30/1') throw new Error(`Expected 1920×1080 at 30 fps: ${file}`);
}
const seconds = file => Number(execFileSync('ffprobe', ['-v', 'error', '-show_entries', 'format=duration', '-of', 'default=nw=1:nk=1', file], {encoding: 'utf8'}));
const encode = ['-c:v', 'libx264', '-threads', '2', '-preset', 'fast', '-crf', '20', '-pix_fmt', 'yuv420p', '-r', '30', '-an'];
const browser = await chromium.launch({headless: true});
const page = await browser.newPage({viewport: {width: 1920, height: 1080}, deviceScaleFactor: 1});
const escaped = text => text.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;');
try {
  for (const [index, shot] of shots.entries()) {
    shot.duration = Math.ceil(Math.max(seconds(join(capture, shot.file)), shot.id.includes('summary') ? 10 : shot.id.includes('transcript') ? 8 : 6.5) * 30) / 30;
    await page.setContent(`<html><style>*{box-sizing:border-box}html,body{margin:0;background:transparent;font-family:-apple-system,BlinkMacSystemFont,sans-serif;color:white}.caption{position:absolute;bottom:0;width:100%;height:112px;background:rgba(8,14,25,.95);padding:19px 76px 18px 112px;border-top:1px solid #334257;display:flex;gap:34px;align-items:center}.number{color:#64b9ff;font-size:22px;font-weight:700;white-space:nowrap;letter-spacing:2px}.copy{font-size:27px;line-height:1.35;max-width:1490px}</style><div class="caption"><span class="number">ROOMS ${String(index + 1).padStart(2, '0')}</span><span class="copy">${escaped(shot.caption)}</span></div></html>`);
    await page.screenshot({path: join(edit, `${shot.id}-caption.png`), omitBackground: true});
  }
} finally { await browser.close(); }
const roomDuration = shots.reduce((sum, shot) => sum + shot.duration, 0);
const duration = Math.round((original.duration + roomDuration) * 30) / 30;
console.log(`[edit] ${shots.length} room shots, ${roomDuration.toFixed(2)}s. Film ${duration.toFixed(2)}s.`);
ff(['-i', source, '-t', String(outro.start), '-map', '0:v:0', ...encode, join(edit, 'before.mp4')]);
for (const shot of shots) {
  ff(['-i', join(capture, shot.file), '-loop', '1', '-i', join(edit, `${shot.id}-caption.png`), '-filter_complex_threads', '1', '-filter_complex', '[0:v]fps=30,tpad=stop_mode=clone:stop_duration=15[v];[v][1:v]overlay=0:0,format=yuv420p[out]', '-map', '[out]', '-t', String(shot.duration), ...encode, join(edit, `${shot.id}.mp4`)]);
  console.log(`[edit] ${shot.id}`);
}
ff(['-ss', String(outro.start), '-i', source, '-map', '0:v:0', ...encode, join(edit, 'after.mp4')]);
writeFileSync(join(edit, 'concat.txt'), ['before', ...shots.map(s => s.id), 'after'].map(id => `file '${join(edit, `${id}.mp4`).replaceAll("'", "'\\''")}'`).join('\n'));
ff(['-f', 'concat', '-safe', '0', '-i', join(edit, 'concat.txt'), '-c', 'copy', join(edit, 'picture.mp4')]);
execFileSync(process.execPath, [join(tour, 'scripts/soundtrack.mjs'), '--duration', String(duration)], {stdio: 'inherit'});
const file = 'otto-tour-rooms-20260928.mp4';
ff(['-i', join(edit, 'picture.mp4'), '-i', join(tour, 'public/audio/music.wav'), '-map', '0:v:0', '-map', '1:a:0', '-c:v', 'copy', '-af', 'loudnorm=I=-16:TP=-1.5:LRA=9', '-ar', '48000', '-c:a', 'aac', '-b:a', '192k', '-t', String(duration), '-movflags', '+faststart', join(out, file)]);
const actualDuration = seconds(join(out, file));
if (Math.abs(actualDuration - duration) > 1 / 30) throw new Error(`Rendered duration differs: ${actualDuration} vs ${duration}`);
const manifest = {...original, file, poster: 'otto-tour-rooms-poster.jpg', captions: 'otto-tour-rooms.vtt', duration: actualDuration, chapters: original.chapters.flatMap(chapter => chapter.id === 'outro' ? [{id: 'rooms', section: 'rooms', title: 'Rooms · collaborate and recap', start: outro.start, duration: roomDuration}, {...chapter, start: chapter.start + roomDuration}] : [chapter])};
writeFileSync(join(out, 'film.candidate.json'), JSON.stringify(manifest, null, 2) + '\n');
function clock(sec) { const ms = Math.round(sec * 1000); return `${String(Math.floor(ms / 3600000)).padStart(2, '0')}:${String(Math.floor(ms / 60000) % 60).padStart(2, '0')}:${String(Math.floor(ms / 1000) % 60).padStart(2, '0')}.${String(ms % 1000).padStart(3, '0')}`; }
function parse(value) { const [h,m,s] = value.split(':').map(Number); return h * 3600 + m * 60 + s; }
let captions = readFileSync(join(tour, 'script/baseline-20260925/otto-tour.vtt'), 'utf8');
const blocks = captions.trim().split(/\n\s*\n/); const before = [], after = [];
for (const block of blocks) {
  const match = block.match(/(\d\d:\d\d:\d\d\.\d\d\d) --> (\d\d:\d\d:\d\d\.\d\d\d)/);
  if (!match || parse(match[1]) < outro.start) before.push(block);
  else after.push(block.replace(match[0], `${clock(parse(match[1]) + roomDuration)} --> ${clock(parse(match[2]) + roomDuration)}`));
}
let cursor = outro.start;
const roomCues = shots.map(shot => { const cue = `${clock(cursor)} --> ${clock(cursor + shot.duration)}\n${shot.caption}`; cursor += shot.duration; return cue; });
writeFileSync(join(out, 'otto-tour-rooms.vtt'), [...before, ...roomCues, ...after].join('\n\n') + '\n');
ff(['-ss', String(outro.start + 31), '-i', join(out, file), '-frames:v', '1', '-update', '1', join(out, 'otto-tour-rooms-poster.jpg')]);
writeFileSync(join(out, 'rooms-timeline.json'), JSON.stringify({insertAt: outro.start, roomDuration, duration, shots}, null, 2));
console.log(`[edit] Candidate ready: ${join(out, file)}. Active app manifest unchanged.`);
