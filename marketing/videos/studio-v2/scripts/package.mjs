// Build a local screening room, captions and chapter metadata from the edit.
// --master additionally joins the completed films without recompressing video.
import { readFile, writeFile, mkdir, copyFile } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";
const root = fileURLToPath(new URL("../../", import.meta.url));
const out = path.join(root, "out-v2");
await mkdir(out, { recursive: true });
const catalog = JSON.parse(
  await readFile(
    path.resolve(root, "../../ui/src/lib/walkthroughs/catalog.json"),
    "utf8",
  ),
);
const stamp = (t) =>
  `${String(Math.floor(t / 3600)).padStart(2, "0")}:${String(Math.floor(t / 60) % 60).padStart(2, "0")}:${(t % 60).toFixed(3).padStart(6, "0")}`;
let complete = "WEBVTT\n\n";
let offset = 0;
let metadata = ";FFMETADATA1\ntitle=Otto - A complete workflow\n";
let transcript =
  "# Otto walkthroughs\n\nOriginal illustrated demonstrations with fictional Atlas data. Narration is synthetic (Andrew, en-US); music and effects are original procedural compositions.\n\n";
for (const ep of catalog) {
  let vtt = "WEBVTT\n\n";
  transcript += `## ${ep.title}\n\n${ep.desc}\n\n`;
  for (const ch of ep.chapters) {
    transcript += `### ${ch.title}\n\nSource: \`${ch.source}\` · [Feature guide](../../../docs/features/${ch.doc})\n\n`;
    for (let i = 0; i < ch.steps.length; i++) {
      const start = ch.start + ch.stepStarts[i];
      const end = start + ch.voice[i].duration;
      const text = ch.steps[i].replaceAll("&", "&amp;").replaceAll("<", "&lt;");
      vtt += `${stamp(start)} --> ${stamp(end)}\n${text}\n\n`;
      complete += `${stamp(offset + start)} --> ${stamp(offset + end)}\n${text}\n\n`;
      transcript += `${i + 1}. ${ch.steps[i]}\n`;
    }
    transcript += "\n";
    metadata += `[CHAPTER]\nTIMEBASE=1/1000\nSTART=${Math.round((offset + ch.start) * 1000)}\nEND=${Math.round((offset + ch.start + ch.duration) * 1000)}\ntitle=${ep.title}: ${ch.title}\n`;
  }
  await writeFile(path.join(out, ep.id + ".vtt"), vtt);
  offset += ep.duration;
}
await writeFile(path.join(out, "Complete.vtt"), complete);
await writeFile(path.join(out, "chapters.ffmeta"), metadata);
await writeFile(
  path.join(out, "catalog.json"),
  JSON.stringify(catalog, null, 2) + "\n",
);
await writeFile(path.join(root, "studio-v2/TRANSCRIPT.md"), transcript);
await copyFile(
  path.join(root, "studio-v2/screening.html"),
  path.join(out, "index.html"),
);
if (process.argv.includes("--master")) {
  const concat = catalog
    .map((e) => `file '${e.file}'\nduration ${e.duration}`)
    .join("\n");
  await writeFile(
    path.join(out, "films.ffconcat"),
    "ffconcat version 1.0\n" + concat + "\n",
  );
  execFileSync(
    "ffmpeg",
    [
      "-v",
      "error",
      "-y",
      "-f",
      "concat",
      "-safe",
      "1",
      "-i",
      path.join(out, "films.ffconcat"),
      "-i",
      path.join(out, "chapters.ffmeta"),
      "-map_metadata",
      "1",
      "-map_chapters",
      "1",
      "-c",
      "copy",
      "-movflags",
      "+faststart",
      path.join(out, "Complete.mp4"),
    ],
    { stdio: "inherit" },
  );
}
console.log("Screening room + captions ready:", out);
