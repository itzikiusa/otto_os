// Make the app play the new film — run ONLY after the candidate's mp4, poster and
// vtt are uploaded to the `walkthroughs` release and downloadable.
//   node scripts/activate.mjs
// Copies out/film.candidate.json → ui/src/lib/walkthroughs/film.json and the
// candidate's captions next to it (bundled, because release assets are served
// without CORS headers, so TourFilm resolves captions by basename locally).
import { copyFileSync, existsSync, readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const tour = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repo = resolve(tour, '../../..');
const out = join(tour, 'out');
const walk = join(repo, 'ui/src/lib/walkthroughs');
const film = JSON.parse(readFileSync(join(out, 'film.candidate.json'), 'utf8'));
for (const f of [film.file, film.poster, film.captions]) if (!existsSync(join(out, f))) throw new Error(`out/${f} is missing: render first`);
copyFileSync(join(out, film.captions), join(walk, film.captions));
copyFileSync(join(out, 'film.candidate.json'), join(walk, 'film.json'));
console.log(`[activate] ui/src/lib/walkthroughs/film.json → ${film.file}; captions ${film.captions} bundled. Commit both with the UI.`);
