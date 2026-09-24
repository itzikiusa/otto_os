// Narration: macOS `say` (offline, no cloud) → ffmpeg, one file per chapter.
// Each SENTENCE is synthesized on its own and joined with a short breath so we
// know exactly when every line starts — that drives both the edit
// (src/generated/timing.json) and the captions (out/otto-tour.vtt).
//
//   node scripts/voice.mjs            # all chapters
//   VOICE="Daniel" RATE=180 node scripts/voice.mjs
import { execFileSync } from 'node:child_process';
import { mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { tmpdir } from 'node:os';
import { mkdtempSync } from 'node:fs';

const tour = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const FFMPEG = process.env.FFMPEG ?? '/opt/homebrew/bin/ffmpeg';
const FFPROBE = process.env.FFPROBE ?? '/opt/homebrew/bin/ffprobe';
const VOICE = process.env.VOICE ?? 'Samantha';
const RATE = process.env.RATE ?? '182';
const FPS = 30;
const GAP = 0.28; // breath between sentences (s)

const script = JSON.parse(readFileSync(join(tour, 'script/chapters.json'), 'utf8'));
const audioDir = join(tour, 'public/audio');
const genDir = join(tour, 'src/generated');
const outDir = join(tour, 'out');
for (const d of [audioDir, genDir, outDir]) mkdirSync(d, { recursive: true });
const tmp = mkdtempSync(join(tmpdir(), 'otto-tour-vo-'));

const probe = (f) =>
  Number(execFileSync(FFPROBE, ['-v', 'error', '-show_entries', 'format=duration', '-of', 'csv=p=0', f], { encoding: 'utf8' }).trim());

/** Split into sentences; keep the chapter's own punctuation. */
const sentences = (text) => text.match(/[^.!?]+[.!?]+["”’)]*\s*/g)?.map((s) => s.trim()) ?? [text];
/** What the synthesizer should read (spoken forms for symbols). */
const spoken = (s) =>
  s
    .replace(/⌘K/g, 'Command K')
    .replace(/⌥Space/g, 'Option Space')
    .replace(/⌃1/g, 'Control 1')
    .replace(/\bSFTP\b/g, 'S F T P')
    .replace(/\bMCP\b/g, 'M C P')
    .replace(/\bAWS\b/g, 'A W S')
    .replace(/\bAPIs\b/g, 'A P Is')
    .replace(/\bAPI\b/g, 'A P I')
    .replace(/\bSSH\b/g, 'S S H')
    .replace(/\bPRs\b/g, 'P Rs')
    .replace(/\bGROUP BY\b/gi, 'group by')
    .replace(/\bORDER BY\b/gi, 'order by')
    .replace(/\bHAVING\b/g, 'having')
    .replace(/\bCI\b/g, 'C I')
    .replace(/\bKubernetes\b/g, 'Kubernetes')
    .replace(/\bOtto\b/g, 'Otto');

const chapters = [];
let cursor = 0;
const cues = [];
for (const ch of script.chapters) {
  const parts = [];
  const lines = [];
  let t = 0;
  const sents = ch.text ? sentences(ch.text) : [];
  for (const [i, s] of sents.entries()) {
    const aiff = join(tmp, `${ch.id}-${i}.aiff`);
    const wav = join(tmp, `${ch.id}-${i}.wav`);
    execFileSync('say', ['-v', VOICE, '-r', RATE, '-o', aiff, spoken(s)]);
    // Trim synth leading/trailing silence so gaps are ours, not the engine's.
    execFileSync(FFMPEG, [
      '-y', '-v', 'error', '-i', aiff,
      '-af', 'silenceremove=start_periods=1:start_threshold=-50dB:start_silence=0.02,areverse,silenceremove=start_periods=1:start_threshold=-50dB:start_silence=0.05,areverse',
      '-ar', '48000', '-ac', '1', wav,
    ]);
    const d = probe(wav);
    lines.push({ text: s, start: t, end: t + d });
    parts.push(wav);
    t += d + GAP;
  }
  const voDuration = sents.length ? t - GAP : 0;
  // Chapter length: narration + lead-in/out, never shorter than its visuals need.
  const lead = ch.leadIn ?? 0.5;
  const tail = ch.tail ?? 0.7;
  const duration = Math.max(ch.minSeconds ?? 0, lead + voDuration + tail);
  const frames = Math.round(duration * FPS);
  if (parts.length) {
    // Concatenate sentences with GAP silence, then polish the voice chain.
    const list = join(tmp, `${ch.id}.txt`);
    const silence = join(tmp, 'gap.wav');
    execFileSync(FFMPEG, ['-y', '-v', 'error', '-f', 'lavfi', '-i', `anullsrc=r=48000:cl=mono`, '-t', String(GAP), silence]);
    writeFileSync(list, parts.flatMap((p, i) => (i ? [`file '${silence}'`, `file '${p}'`] : [`file '${p}'`])).join('\n'));
    execFileSync(FFMPEG, [
      '-y', '-v', 'error', '-f', 'concat', '-safe', '0', '-i', list,
      '-af', [
        'highpass=f=75',
        'equalizer=f=220:t=q:w=1.2:g=-1.5',
        'equalizer=f=3200:t=q:w=1.1:g=2.5',
        'acompressor=threshold=-21dB:ratio=2.6:attack=6:release=90:makeup=2',
        'loudnorm=I=-16:TP=-2:LRA=7',
      ].join(','),
      '-ar', '48000', '-ac', '1', join(audioDir, `vo-${ch.id}.wav`),
    ]);
  }
  const start = cursor;
  chapters.push({
    id: ch.id,
    section: ch.section,
    title: ch.title,
    startFrame: Math.round(start * FPS),
    frames,
    voOffset: lead,
    voDuration,
    lines,
    hasVo: parts.length > 0,
  });
  for (const l of lines) {
    // Long sentences become two cues, split at the punctuation nearest the
    // middle, timed in proportion to their length.
    const a = start + lead + l.start;
    const b = start + lead + l.end;
    if (l.text.length > 84) {
      const mid = l.text.length / 2;
      let cut = -1;
      for (const m of l.text.matchAll(/[,:;—]\s/g)) if (cut < 0 || Math.abs(m.index - mid) < Math.abs(cut - mid)) cut = m.index;
      if (cut > 20 && cut < l.text.length - 20) {
        const first = l.text.slice(0, cut + 1).trim();
        const second = l.text.slice(cut + 1).trim();
        const t = a + ((b - a) * first.length) / (first.length + second.length);
        cues.push({ start: a, end: t - 0.15, text: first }, { start: t, end: b, text: second });
        continue;
      }
    }
    cues.push({ start: a, end: b, text: l.text });
  }
  console.log(`[voice] ${ch.id.padEnd(16)} vo ${voDuration.toFixed(2)}s  chapter ${duration.toFixed(2)}s  (${sents.length} lines)`);
  cursor += frames / FPS;
}
const totalFrames = chapters.reduce((s, c) => s + c.frames, 0);
writeFileSync(
  join(genDir, 'timing.json'),
  JSON.stringify({ fps: FPS, voice: VOICE, rate: Number(RATE), totalFrames, totalSeconds: totalFrames / FPS, chapters }, null, 2) + '\n',
);

// WebVTT captions straight from the sentence timings.
const ts = (s) => {
  const ms = Math.round(s * 1000);
  const h = Math.floor(ms / 3600000);
  const m = Math.floor((ms % 3600000) / 60000);
  const sec = Math.floor((ms % 60000) / 1000);
  return `${String(h).padStart(2, '0')}:${String(m).padStart(2, '0')}:${String(sec).padStart(2, '0')}.${String(ms % 1000).padStart(3, '0')}`;
};
const vtt = ['WEBVTT', '', ...cues.flatMap((c, i) => [String(i + 1), `${ts(c.start)} --> ${ts(c.end + 0.15)}`, c.text, ''])].join('\n');
writeFileSync(join(outDir, 'otto-tour.vtt'), vtt);
rmSync(tmp, { recursive: true, force: true });
const words = script.chapters.map((c) => c.text ?? '').join(' ').split(/\s+/).filter(Boolean).length;
console.log(`[voice] ${VOICE} @${RATE}wpm · ${words} words · film ${(totalFrames / FPS).toFixed(2)}s · ${cues.length} captions`);
