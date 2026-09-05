// Checks editorial coverage and real media boundaries, including narration fit.
import { readFile, access, writeFile } from "node:fs/promises";
import { execFileSync, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";
import assert from "node:assert/strict";
const root = fileURLToPath(new URL("../../../../", import.meta.url));
const catalog = JSON.parse(
  await readFile(
    path.join(root, "ui/src/lib/walkthroughs/catalog.json"),
    "utf8",
  ),
);
const sidebar = await readFile(
  path.join(root, "ui/src/lib/sidebar.ts"),
  "utf8",
);
const routes = [...sidebar.matchAll(/\bid: '([^']+)'/g)].map((m) => m[1]);
const covered = new Set(
  catalog.flatMap((e) =>
    e.chapters.flatMap((c) => [c.route, ...(c.alsoRoutes || [])]),
  ),
);
const ids = new Set(),
  chapterIds = new Set();
const media = [];
for (const route of routes)
  assert(
    covered.has(route),
    `No walkthrough chapter for sidebar route ${route}`,
  );
for (const e of catalog) {
  assert(!ids.has(e.id), "Duplicate episode " + e.id);
  ids.add(e.id);
  let cursor = e.introSeconds;
  for (const c of e.chapters) {
    assert(!chapterIds.has(c.id), "Duplicate chapter " + c.id);
    chapterIds.add(c.id);
    await access(path.join(root, c.source));
    await access(path.join(root, "docs/features", c.doc));
    assert.equal(c.start, cursor, `Gap or overlap before ${c.id}`);
    assert.equal(c.steps.length, 3);
    assert.equal(c.actions.length, 3);
    assert.equal(c.voice.length, 3);
    for (let i = 0; i < 3; i++) {
      await access(path.join(root, "marketing/videos/public", c.voice[i].file));
      assert(
        c.stepStarts[i] + c.voice[i].duration < c.duration,
        `Voice exceeds scene ${c.id}`,
      );
      if (i < 2)
        assert(
          c.stepStarts[i] + c.voice[i].duration < c.stepStarts[i + 1],
          `Overlapping speech ${c.id}`,
        );
    }
    cursor += c.duration;
  }
  assert.equal(cursor + e.outroSeconds, e.duration);
  if (process.argv.includes("--media")) {
    const file = path.join(root, "marketing/videos/out-v2", e.file);
    const probe = JSON.parse(
      execFileSync(
        "ffprobe",
        ["-v", "error", "-show_streams", "-show_format", "-of", "json", file],
        { encoding: "utf8" },
      ),
    );
    const video = probe.streams.find((s) => s.codec_type === "video"),
      audio = probe.streams.find((s) => s.codec_type === "audio");
    assert(video && audio, `${e.id} must have picture and sound`);
    assert.equal(video.width, 1920);
    assert.equal(video.height, 1080);
    assert.equal(video.avg_frame_rate, "30/1");
    assert.equal(video.codec_name, "h264");
    assert.equal(audio.codec_name, "aac");
    assert.equal(audio.channels, 2);
    assert(
      Math.abs(Number(probe.format.duration) - e.duration) < 0.12,
      `Wrong duration ${e.id}`,
    );
    // Decode every stream and measure the final AAC mix, including codec peak overshoot.
    const measured = spawnSync(
      "ffmpeg",
      [
        "-hide_banner",
        "-xerror",
        "-i",
        file,
        "-af",
        "loudnorm=I=-16:TP=-1.5:LRA=9:print_format=json",
        "-f",
        "null",
        "-",
      ],
      { encoding: "utf8" },
    );
    assert.equal(measured.status, 0, measured.stderr);
    const level = JSON.parse(
      measured.stderr.match(/\{[\s\S]*?\}/)?.[0] || "{}",
    );
    assert(
      Number(level.input_i) > -18 && Number(level.input_i) < -14,
      `Unexpected loudness ${e.id}: ${level.input_i}`,
    );
    assert(
      Number(level.input_tp) < -0.5,
      `Audio peaks too high ${e.id}: ${level.input_tp}`,
    );
    media.push({
      id: e.id,
      duration: Number(probe.format.duration),
      width: video.width,
      height: video.height,
      fps: video.avg_frame_rate,
      audio: audio.codec_name,
      channels: audio.channels,
      decoded: true,
      loudnessLUFS: Number(level.input_i),
      truePeakDBTP: Number(level.input_tp),
    });
    console.log(e.id, "media OK");
  }
}
const report = {
  checkedAt: new Date().toISOString(),
  sourceRevision: execFileSync("git", ["rev-parse", "HEAD"], {
    cwd: root,
    encoding: "utf8",
  }).trim(),
  episodes: catalog.length,
  chapters: chapterIds.size,
  duration: catalog.reduce((n, e) => n + e.duration, 0),
  sidebarRoutes: routes,
  allRoutesCovered: true,
  media,
};
if (process.argv.includes("--media"))
  await writeFile(
    path.join(root, "marketing/videos/out-v2/verification.json"),
    JSON.stringify(report, null, 2) + "\n",
  );
console.log(
  `${catalog.length} films, ${chapterIds.size} chapters; every sidebar route covered; speech fits all scenes.`,
);
