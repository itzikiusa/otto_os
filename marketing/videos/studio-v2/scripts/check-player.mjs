// Isolated browser check of the actual Svelte walkthrough component.
// No daemon, accounts, production state or network APIs are involved.
import { createRequire } from "node:module";
import {
  mkdtemp,
  writeFile,
  readFile,
  rm,
  mkdir,
  symlink,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import assert from "node:assert/strict";
const root = fileURLToPath(new URL("../../../../", import.meta.url));
const require = createRequire(path.join(root, "ui/package.json"));
const { chromium } = require("@playwright/test");
const { createServer } = await import(pathToFileURL(require.resolve("vite")));
const { svelte } = await import(
  pathToFileURL(require.resolve("@sveltejs/vite-plugin-svelte"))
);
const dir = await mkdtemp(path.join(tmpdir(), "otto-film-player-"));
await symlink(
  path.join(root, "ui/node_modules"),
  path.join(dir, "node_modules"),
  "dir",
);
const out = path.join(root, "marketing/videos/out-v2/qa");
await mkdir(out, { recursive: true });
const catalog = JSON.parse(
  await readFile(
    path.join(root, "ui/src/lib/walkthroughs/catalog.json"),
    "utf8",
  ),
);
const css = `:root{--bg:#101b30;--text:#edf2f6;--text-dim:#a3b5cd;--surface-2:#1b2b42;--border:#354860;--accent:#83b1f1}html,body,#app{height:100%;margin:0;font-family:system-ui}*{box-sizing:border-box}`;
await writeFile(
  path.join(dir, "index.html"),
  `<html><head><meta name="viewport" content="width=device-width, initial-scale=1"><style>${css}</style></head><body><div id="app"></div><script type="module" src="/main.js"></script></body></html>`,
);
await writeFile(
  path.join(dir, "main.js"),
  `import {mount} from 'svelte';import Page from '/@fs/${root}/ui/src/modules/help/Walkthroughs.svelte';import {registry} from '/@fs/${root}/ui/src/lib/commands.svelte.ts';window.testRegistry=registry;mount(Page,{target:document.getElementById('app')});`,
);
const server = await createServer({
  configFile: false,
  root: dir,
  plugins: [svelte({ configFile: false })],
  resolve: { dedupe: ["svelte"] },
  define: {
    "import.meta.env.VITE_WALKTHROUGHS_V2_BASE": JSON.stringify(
      "http://127.0.0.1:8780",
    ),
  },
  server: {
    host: "127.0.0.1",
    port: 8781,
    strictPort: true,
    fs: { allow: [root, dir] },
  },
});
await server.listen();
const browser = await chromium.launch({ headless: true });
try {
  const page = await browser.newPage({
    viewport: { width: 1440, height: 1000 },
  });
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto("http://127.0.0.1:8781");
  await page.waitForSelector(".rail-item");
  assert.equal(await page.locator(".rail-item").count(), 8);
  await page.waitForFunction(
    () => document.querySelector("video")?.readyState >= 1,
    {},
    { timeout: 30000 },
  );
  assert(
    await page.locator("video").evaluate((v) => v.paused),
    "Narration must not autoplay on mount",
  );
  await page.locator(".chapter-button").nth(1).click();
  await page.waitForFunction(
    (start) =>
      Math.abs(document.querySelector("video").currentTime - start) < 2,
    catalog[0].chapters[1].start,
  );
  await page.locator("video").evaluate((v) => v.pause());
  // Same-film palette jumps must seek immediately; changing films waits for metadata.
  const first = catalog[0].chapters[0];
  await page.evaluate(
    (id) =>
      window.testRegistry.all
        .find((c) => c.id === "walkthrough.chapter." + id)
        .run(),
    first.id,
  );
  await page.waitForFunction(
    (start) =>
      Math.abs(document.querySelector("video").currentTime - start) < 2,
    first.start,
  );
  await page.locator("video").evaluate((v) => {v.pause();v.textTracks[0].mode='hidden';});
  await page.waitForFunction(() => document.querySelector('video').textTracks[0].cues?.length === 9);
  const proofChapter = catalog[1].chapters.find(ch => ch.id === 'proof');
  await page.evaluate(() => window.testRegistry.all.find(c => c.id === 'walkthrough.chapter.proof').run());
  await page.waitForFunction(start => document.querySelector('video').src.endsWith('/Delivery.mp4') && Math.abs(document.querySelector('video').currentTime-start)<2, proofChapter.start);
  await page.locator('.rail-item').first().click();
  await page.waitForFunction(() => document.querySelector('video').readyState >= 2);
  await page.getByRole("searchbox").fill("Kubernetes");
  assert.equal(await page.locator(".rail-item").count(), 1);
  await page.getByRole("searchbox").fill("Insights");
  assert.equal(await page.locator(".rail-item").count(), 1);
  await page.getByRole("searchbox").fill("");
  await page.screenshot({ path: path.join(out, "player-desktop.png") });
  await page.setViewportSize({ width: 390, height: 844 });
  const mobileVideo = await page.locator("video").boundingBox();
  assert(mobileVideo.width >= 320, "Mobile video must remain watchable, not a narrow strip");
  assert(mobileVideo.height >= 160 && mobileVideo.height <= 260, "Mobile video preserves a landscape frame");
  await page.screenshot({ path: path.join(out, "player-mobile.png") });
  assert(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth + 2,
    ),
    "Mobile horizontal overflow",
  );
  // Test a failed stream and explicit retry against a blocked film URL.
  await page.route("**/Cloud.mp4", (r) => r.abort());
  await page.locator(".rail-item").nth(5).click();
  await page.waitForSelector(".video-fallback");
  await page.unroute("**/Cloud.mp4");
  await page.locator(".fallback-retry").click();
  assert.equal(await page.locator("video").count(), 1);
  await page.waitForFunction(() => document.querySelector("video")?.readyState >= 1);
  // The screening package is also usable without the app shell.
  await page.goto("http://127.0.0.1:8780");
  await page.waitForSelector(".film");
  assert.equal(await page.locator(".film").count(), 8);
  await page.screenshot({
    path: path.join(out, "screening-mobile.png"),
    fullPage: true,
  });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.screenshot({
    path: path.join(out, "screening-desktop.png"),
    fullPage: true,
  });
  await page.locator(".film").first().click();
  await page.waitForFunction(
    () => document.querySelector("video")?.readyState >= 1,
  );
  await page.locator("#chapters button").nth(1).click();
  await page.waitForFunction(
    (start) =>
      Math.abs(document.querySelector("video").currentTime - start) < 2,
    catalog[0].chapters[1].start,
  );
  await page.keyboard.press("Escape");
  await page.waitForFunction(() => document.querySelector("video").paused && !document.querySelector("dialog").open);
  // A chapter requested before metadata must not resume audio after closing.
  let released = false;
  const releaseWaiters = [];
  await page.route('**/Delivery.mp4', async route => {
    if (!released) await new Promise(resolve => releaseWaiters.push(resolve));
    await route.continue();
  });
  await page.locator('.film').nth(1).click();
  await page.waitForFunction(() => document.querySelector('video').readyState === 0);
  await page.locator('#chapters button').nth(2).click();
  await page.locator('#close').click();
  released = true;
  for (const release of releaseWaiters) release();
  await page.waitForFunction(() => document.querySelector('video').readyState >= 1);
  assert(await page.locator('video').evaluate(v => v.paused), 'Deferred seek restarted closed-dialog audio');
  await page.unroute('**/Delivery.mp4');
  await page.locator('#play-all').click();
  await page.waitForFunction(() => document.querySelector('video').readyState >= 1);
  assert(Math.abs(await page.locator('video').evaluate(v => v.duration)-750)<0.15);
  await page.locator('video').evaluate(v => {v.pause();v.textTracks[0].mode='hidden';});
  await page.waitForFunction(() => document.querySelector('video').textTracks[0].cues?.length === 99);
  const lastStart = Number(await page.locator('#chapters button').last().getAttribute('data-start'));
  await page.locator('#chapters button').last().click();
  await page.waitForFunction(start => Math.abs(document.querySelector('video').currentTime-start)<2 && !document.querySelector('video').seeking, lastStart);
  await page.locator('#close').click();
  await page.waitForFunction(() => document.querySelector('video').paused);
  assert.deepEqual(errors, []);
  console.log(
    "PASS: catalog, seeking, command jump, no autoplay, search, offline/retry, desktop/mobile, screening room, modal close.",
  );
  await writeFile(
    path.join(out, "player-check.json"),
    JSON.stringify(
      { checkedAt: new Date().toISOString(), passed: true, errors },
      null,
      2,
    ),
  );
} finally {
  await browser.close();
  await server.close();
  await rm(dir, { recursive: true, force: true });
}
