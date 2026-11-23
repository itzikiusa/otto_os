// A scene's SETTING on the real productions server (the isolated E2E daemon's
// fixed planner plans a pier at night in fog with a glowing lantern; the fake
// engine makes stand-in pieces): the picture shows water, a dark night and the
// lantern's light, and "Make it daytime" relights it at once, with nothing made
// again. Pixels are read from the picture as a person sees it (a screenshot).
import { expect, type Page, type TestInfo } from '@playwright/test';
import { evidence, openCreate, type CreateSeed } from './create-fixture';

export const PIER_DESCRIPTION = 'A robot keeper holds a glowing lantern on a foggy wooden pier at night.';
const GROUND = [0xa3, 0x9a, 0x8a]; // the plain ground an older scene stands on

interface Picture {
  /** Mean brightness, 0..255. */
  mean: number;
  median: number;
  /** The mean colour beside the cast, low on the left (the camera frames the cast, so the
   *  middle of the foreground is the base it stands on). */
  front: [number, number, number];
  /** The brightest 12 × 12 block: its brightness, its colour, and the ring round it. */
  glow: { mean: number; rgb: [number, number, number]; ring: number };
}

async function picture(page: Page): Promise<Picture> {
  const canvas = page.locator('[data-testid="create-model"] canvas').first();
  const shot = (await canvas.screenshot()).toString('base64');
  return page.evaluate(async (b64) => {
    const img = new Image();
    img.src = 'data:image/png;base64,' + b64;
    await img.decode();
    const c = document.createElement('canvas');
    [c.width, c.height] = [img.width, img.height];
    const g = c.getContext('2d')!;
    g.drawImage(img, 0, 0);
    const { width: w, height: h } = c;
    const d = g.getImageData(0, 0, w, h).data;
    const lum = (i: number) => 0.2126 * d[i] + 0.7152 * d[i + 1] + 0.0722 * d[i + 2];
    const all: number[] = [];
    for (let i = 0; i < d.length; i += 4) all.push(lum(i));
    const sorted = [...all].sort((a, b) => a - b);
    const box = (x0: number, y0: number, x1: number, y1: number) => {
      const s = [0, 0, 0, 0];
      for (let y = Math.max(0, y0); y < Math.min(h, y1); y++)
        for (let x = Math.max(0, x0); x < Math.min(w, x1); x++) {
          const i = (y * w + x) * 4;
          s[0] += d[i]; s[1] += d[i + 1]; s[2] += d[i + 2]; s[3]++;
        }
      return [s[0] / s[3], s[1] / s[3], s[2] / s[3]] as [number, number, number];
    };
    const l = (c: number[]) => 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    // The brightest block, away from the rounded corners.
    let best = { mean: -1, x: 0, y: 0, rgb: [0, 0, 0] as [number, number, number] };
    for (let y = 16; y < h - 28; y += 6)
      for (let x = 16; x < w - 28; x += 6) {
        const rgb = box(x, y, x + 12, y + 12);
        if (l(rgb) > best.mean) best = { mean: l(rgb), x, y, rgb };
      }
    // The ring: 48 × 48 round the block, the block itself left out.
    const big = box(best.x - 18, best.y - 18, best.x + 30, best.y + 30);
    const ring = (l(big) * 48 * 48 - best.mean * 144) / (48 * 48 - 144);
    return {
      mean: all.reduce((a, b) => a + b, 0) / all.length,
      median: sorted[Math.floor(sorted.length / 2)],
      front: box(Math.round(w * 0.03), Math.round(h * 0.6), Math.round(w * 0.2), Math.round(h * 0.95)),
      glow: { mean: best.mean, rgb: best.rgb, ring },
    };
  }, shot);
}

/** The picture once every piece has loaded and it shows what `ready` waits for. */
async function settled(page: Page, ready: (p: Picture) => boolean): Promise<Picture> {
  await page.waitForLoadState('networkidle').catch(() => {});
  let p: Picture | null = null;
  await expect(async () => {
    p = await picture(page);
    expect(ready(p)).toBe(true);
  }).toPass({ timeout: 30_000 });
  return p!;
}

export async function settingFlow(page: Page, seed: CreateSeed, info: TestInfo): Promise<void> {
  const shot = (n: string) => evidence(page, info.project.name, n);
  await openCreate(page, seed, '');
  await page.getByTestId('create-idea').fill(PIER_DESCRIPTION);
  await page.getByTestId('create-make').click();
  await expect(page.getByRole('heading', { name: 'Here’s my plan' })).toBeVisible({ timeout: 30_000 });
  await page.getByTestId('create-make').click();
  await expect(page.getByRole('heading', { name: 'The keeper’s light' }).or(page.getByRole('heading', { name: "The keeper's light" }))).toBeVisible({ timeout: 280_000 });
  const canvas = page.locator('[data-testid="create-model"] canvas').first();
  await expect(canvas).toBeVisible({ timeout: 30_000 });
  await canvas.scrollIntoViewIfNeeded();
  const night = await settled(page, (p) => p.glow.mean > 120);
  await shot('31-setting-night');
  // The graphics processor drops the view and gives it back: the night picture comes back by itself, no Retry.
  await canvas.evaluate((c: HTMLCanvasElement) => {
    const lose = (c.getContext('webgl2') ?? c.getContext('webgl'))?.getExtension('WEBGL_lose_context');
    lose?.loseContext();
    setTimeout(() => lose?.restoreContext(), 1500);
  });
  await expect(page.getByText('3D preview unavailable')).toBeVisible();
  await expect(page.getByText('3D preview unavailable')).toHaveCount(0, { timeout: 30_000 });
  await settled(page, (p) => p.glow.mean > 120);
  // Water beside the pier, not the plain ground: dark and blue rather than the beige floor.
  const [r, g, b] = night.front;
  expect(Math.abs(r - GROUND[0]) + Math.abs(g - GROUND[1]) + Math.abs(b - GROUND[2]), 'the foreground is not the old ground').toBeGreaterThan(90);
  expect(b, 'the water is bluish').toBeGreaterThanOrEqual(r);
  // Night: a dark picture.
  expect(night.mean, 'a night scene is dark').toBeLessThan(70);
  // The lantern: a warm spot much brighter than the picture, lighting what is round it.
  expect(night.glow.mean, 'the lantern shines').toBeGreaterThan(Math.max(120, 3 * night.median));
  // A warm light (its core may burn white when the camera is close to it).
  expect(night.glow.rgb[0], 'a warm light').toBeGreaterThanOrEqual(night.glow.rgb[2]);
  expect(night.glow.ring, 'the light falls round the lantern').toBeGreaterThan(night.median * 1.3);

  // "Make it daytime" is the setting's: no item changes, so nothing is made and the picture is relit at once.
  let making = false;
  const watch = page.getByRole('heading', { name: 'Making your scene…' });
  await page.getByTestId('create-steer').fill('Make it daytime.');
  await page.getByTestId('create-steer-go').click();
  const said = page.getByTestId('create-said');
  for (let i = 0; i < 120 && !(await said.isVisible().catch(() => false)); i++) {
    making ||= await watch.isVisible().catch(() => false);
    await page.waitForTimeout(250); // ui-guards: allow — watching that the making screen never shows
  }
  await expect(said).toContainText('It is daytime now.', { timeout: 30_000 });
  expect(making, 'no making screen for a setting change').toBe(false);
  await expect(canvas).toBeVisible();
  await canvas.scrollIntoViewIfNeeded();
  const day = await settled(page, (p) => p.mean > 90);
  await shot('32-setting-day');
  expect(day.mean, 'the day is much brighter than the night').toBeGreaterThan(Math.max(90, night.mean * 2));
}
