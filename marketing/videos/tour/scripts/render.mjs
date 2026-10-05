// Render the tour film and everything the app needs to play it — as review
// candidates in out/. It never edits the app's active manifest; after the
// assets are uploaded and verified, scripts/activate.mjs does that.
//
//   node scripts/render.mjs                          # full film → out/otto-tour-<edition>.mp4 (+ poster, vtt, film.candidate.json)
//   node scripts/render.mjs --stills 240,600,1800    # preview frames → .cache/frames/f<N>.jpg (half scale)
//   node scripts/render.mjs --stills 240 --scale 1   # full-size preview
//   node scripts/render.mjs --crf 24                 # override the H.264 quality (default 28)
//
// Steps: bundle once → renderMedia (concurrency ≤ 4) → ffmpeg two-pass loudnorm
// to −16 LUFS / −1.5 dBTP (video stream copied) → poster still → captions + manifest.
import { bundle } from '@remotion/bundler';
import { renderMedia, renderStill, selectComposition } from '@remotion/renderer';
import { execFileSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const tour = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const FF = process.env.FFMPEG ?? 'ffmpeg';
const argv = process.argv.slice(2);
const opt = (f) => {
  const i = argv.indexOf(`--${f}`);
  return i >= 0 ? argv[i + 1] : undefined;
};
const CONCURRENCY = Math.min(4, Number(opt('concurrency') ?? 4));
const CRF = Number(opt('crf') ?? 28);
const out = join(tour, 'out');
mkdirSync(out, { recursive: true });
const timing = JSON.parse(readFileSync(join(tour, 'src/generated/timing.json'), 'utf8'));
const name = `otto-tour-${timing.edition}`;

if (!existsSync(join(tour, 'public/audio/music.wav'))) throw new Error('public/audio/music.wav is missing: run node scripts/soundtrack.mjs first');

console.log('[render] bundling…');
const serveUrl = await bundle({ entryPoint: join(tour, 'src/index.ts'), publicDir: join(tour, 'public') });
const composition = await selectComposition({ serveUrl, id: 'OttoTour' });

const stills = opt('stills');
if (stills) {
  const dir = join(tour, '.cache/frames');
  mkdirSync(dir, { recursive: true });
  const scale = Number(opt('scale') ?? 0.5);
  for (const f of stills.split(',').map(Number)) {
    await renderStill({ serveUrl, composition, frame: f, output: join(dir, `f${f}.jpg`), imageFormat: 'jpeg', jpegQuality: 85, scale });
    console.log(`[render] still ${f}`);
  }
  process.exit(0);
}

const raw = join(out, `${name}.raw.mp4`);
const final = join(out, `${name}.mp4`);
let last = -1;
await renderMedia({
  serveUrl,
  composition,
  codec: 'h264',
  outputLocation: raw,
  concurrency: CONCURRENCY,
  crf: CRF,
  x264Preset: 'slow',
  pixelFormat: 'yuv420p',
  audioCodec: 'aac',
  audioBitrate: '192k',
  onProgress: ({ progress }) => {
    const p = Math.floor(progress * 20);
    if (p !== last) {
      last = p;
      console.log(`[render] ${Math.round(progress * 100)}%`);
    }
  },
});

// Loudness: measure, then normalize to −16 LUFS / −1.5 dBTP (two-pass, linear).
console.log('[render] loudness pass…');
let m = null;
try {
  // loudnorm prints its JSON report on stderr.
  const all = execFileSync('sh', ['-c', `"${FF}" -hide_banner -i "${raw}" -vn -af loudnorm=I=-16:TP=-1.5:LRA=11:print_format=json -f null - 2>&1`], { encoding: 'utf8' });
  m = JSON.parse(all.slice(all.lastIndexOf('{'), all.lastIndexOf('}') + 1));
} catch {
  m = null;
}
const ln = m
  ? `loudnorm=I=-16:TP=-1.5:LRA=11:measured_I=${m.input_i}:measured_TP=${m.input_tp}:measured_LRA=${m.input_lra}:measured_thresh=${m.input_thresh}:offset=${m.target_offset}:linear=true`
  : 'loudnorm=I=-16:TP=-1.5:LRA=11';
execFileSync(FF, ['-y', '-v', 'error', '-i', raw, '-c:v', 'copy', '-af', ln, '-ar', '48000', '-c:a', 'aac', '-b:a', '192k', '-movflags', '+faststart', final]);
rmSync(raw, { force: true });

// Poster.
const posterComp = await selectComposition({ serveUrl, id: 'Poster' });
await renderStill({ serveUrl, composition: posterComp, output: join(out, `${name}-poster.jpg`), imageFormat: 'jpeg', jpegQuality: 90 });

// Captions + the candidate manifest for the Help page (same contract as film.json).
copyFileSync(join(tour, `src/generated/${name}.vtt`), join(out, `${name}.vtt`));
const duration = Number(execFileSync('ffprobe', ['-v', 'error', '-show_entries', 'format=duration', '-of', 'default=nw=1:nk=1', final], { encoding: 'utf8' }));
const film = {
  file: `${name}.mp4`,
  poster: `${name}-poster.jpg`,
  captions: `${name}.vtt`,
  duration: Math.round(duration * 100) / 100,
  chapters: timing.chapters.map((c) => ({
    id: c.id,
    section: c.section,
    title: c.title,
    start: Math.round((c.startFrame / timing.fps) * 100) / 100,
    duration: Math.round((c.frames / timing.fps) * 100) / 100,
  })),
};
writeFileSync(join(out, 'film.candidate.json'), JSON.stringify(film, null, 2) + '\n');

const mb = (statSync(final).size / 1048576).toFixed(1);
console.log(`[render] ${final} — ${duration.toFixed(2)}s, ${mb} MB; poster, captions and out/film.candidate.json written (active manifest unchanged)`);
