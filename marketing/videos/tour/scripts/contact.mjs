// Dev helper: tile captures (or extracted frames) 4-up into labelled contact
// sheets for quick review.   node scripts/contact.mjs [dir] [filter-regex]
import { execFileSync } from 'node:child_process';
import { mkdirSync, readdirSync, rmSync } from 'node:fs';
import { basename, join } from 'node:path';

const FF = process.env.FFMPEG ?? '/opt/homebrew/bin/ffmpeg';
const dir = process.argv[2] ?? 'public/capture';
const re = new RegExp(process.argv[3] ?? '.');
const out = '.cache/cs';
rmSync(out, { recursive: true, force: true });
mkdirSync(out, { recursive: true });
const files = readdirSync(dir).filter((f) => /\.(jpg|png)$/.test(f) && re.test(f)).sort().map((f) => join(dir, f));
for (let i = 0; i < files.length; i += 4) {
  const group = files.slice(i, i + 4);
  const args = [];
  let filt = '';
  group.forEach((f, k) => {
    args.push('-i', f);
    filt += `[${k}:v]scale=1600:900:force_original_aspect_ratio=decrease,pad=1600:900:(ow-iw)/2:(oh-ih)/2[v${k}];`;
  });
  for (let k = group.length; k < 4; k++) {
    args.push('-f', 'lavfi', '-i', 'color=c=black:s=1600x900:d=1');
    filt += `[${k}:v]null[v${k}];`;
  }
  const name = join(out, `${String(i / 4 + 1).padStart(2, '0')}.jpg`);
  execFileSync(FF, ['-y', '-v', 'error', ...args, '-filter_complex', `${filt}[v0][v1][v2][v3]xstack=inputs=4:layout=0_0|w0_0|0_h0|w0_h0`, '-frames:v', '1', '-q:v', '3', name]);
  console.log(name, group.map((f) => basename(f)).join(' '));
}
