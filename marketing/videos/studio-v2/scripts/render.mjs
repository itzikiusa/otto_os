import { bundle } from "@remotion/bundler";
import {
  getCompositions,
  openBrowser,
  renderMedia,
  renderStill,
} from "@remotion/renderer";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";
const root = fileURLToPath(new URL("../../", import.meta.url));
const out = path.join(root, "out-v2");
const catalog = JSON.parse(
  await readFile(
    path.resolve(root, "../../ui/src/lib/walkthroughs/catalog.json"),
    "utf8",
  ),
);
const args = process.argv.slice(2);
const stills = args.includes("--stills");
const ids = args.filter((a) => !a.startsWith("--"));
if (ids.some((id) => !catalog.some((e) => e.id === id)))
  throw new Error("Unknown composition: " + ids.join(", "));
await mkdir(path.join(out, "qa"), { recursive: true });
const serveUrl = await bundle({
  entryPoint: path.join(root, "studio-v2/index.tsx"),
  publicDir: path.join(root, "public"),
  onProgress: () => {},
});
const browser = await openBrowser("chrome");
try {
  const compositions = await getCompositions(serveUrl, {
    puppeteerInstance: browser,
  });
  for (const ep of catalog.filter((e) => !ids.length || ids.includes(e.id))) {
    const composition = compositions.find((c) => c.id === ep.id);
    if (!composition) throw new Error("Missing composition " + ep.id);
    if (stills) {
      await renderStill({
        serveUrl,
        composition,
        puppeteerInstance: browser,
        frame: 75,
        output: path.join(out, `${ep.id}.jpg`),
        imageFormat: "jpeg",
        jpegQuality: 95,
      });
      for (const ch of ep.chapters) {
        const frame = Math.round((ch.start + ch.stepStarts[1] + 1.1) * 30);
        await renderStill({
          serveUrl,
          composition,
          puppeteerInstance: browser,
          frame,
          output: path.join(out, "qa", `${ep.id}-${ch.id}.png`),
          imageFormat: "png",
        });
      }
      console.log(ep.id, "stills ready");
    } else {
      let last = -1;
      await renderMedia({
        serveUrl,
        composition,
        puppeteerInstance: browser,
        codec: "h264",
        audioCodec: "aac",
        audioBitrate: "192k",
        crf: 18,
        pixelFormat: "yuv420p",
        x264Preset: "fast",
        concurrency: 8,
        imageFormat: "jpeg",
        jpegQuality: 95,
        outputLocation: path.join(out, ep.file),
        onProgress: ({ progress }) => {
          const pct = Math.floor(progress * 5) * 20;
          if (pct !== last) {
            last = pct;
            console.log(ep.id, pct + "%");
          }
        },
      });
      console.log(ep.id, "rendered");
    }
  }
  await writeFile(
    path.join(out, "catalog.json"),
    JSON.stringify(catalog, null, 2) + "\n",
  );
} finally {
  await browser.close({ silent: true });
}
