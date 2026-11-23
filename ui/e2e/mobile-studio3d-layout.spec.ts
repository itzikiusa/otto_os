import { test, expect, type Page } from '@playwright/test';
import { openPage, expectNoHorizontalOverflow, expectFullyInViewport } from './helpers';
import { apiCtx, seedWorkspace } from './seed';
import { riggedGlbBase64 } from './rigged-glb';

// 3D Studio on every phone / tablet project: the toolbar is one row that fits,
// touch targets are finger-sized, the canvas owns the screen, the first view
// frames the whole model, the view cube shows the face the camera looks at, and
// a 3D preview that can't start says so instead of leaving a blank canvas.

test.use({ serviceWorkers: 'block' });

// A 2.6 m lantern (base, body, cap, handle ring) from the authored hero camera:
// the camera the mobile review saw cut the cap off in portrait viewports.
const LANTERN = {
  type: 'otto-scene3d',
  version: 2,
  background: '#161b26',
  grid: true,
  environment: { preset: 'studio-soft', intensity: 1, background: true },
  camera: { position: [3.4, 2.1, 5.2], target: [0, 0.8, 0], fov: 30 },
  lights: [{ id: 'key', name: 'Key light', type: 'directional', position: [4, 6, 4], target: [0, 0.8, 0], intensity: 1.6, shadow: true }],
  objects: [
    { id: 'base', name: 'Lantern base', type: 'cylinder', position: [0, 0.1, 0], rotation: [0, 0, 0], scale: [1.1, 0.2, 1.1], material: { preset: 'brushed-metal', color: '#3b3f4a' } },
    { id: 'body', name: 'Lantern body', type: 'box', radius: 0.05, position: [0, 0.95, 0], rotation: [0, 20, 0], scale: [1.2, 1.4, 0.9], material: { preset: 'glossy-plastic', color: '#5b3df5' } },
    { id: 'cap', name: 'Lantern cap', type: 'cone', position: [0, 2.0, 0], rotation: [0, 0, 0], scale: [1.0, 0.6, 1.0], material: { preset: 'brushed-metal', color: '#3b3f4a' } },
    { id: 'ring', name: 'Handle ring', type: 'torus', position: [0, 2.55, 0], rotation: [90, 0, 0], scale: [0.35, 0.35, 0.35], material: { preset: 'brushed-metal', color: '#c9a227' } },
  ],
  groups: [],
  states: [{ id: 'idle', name: 'Idle' }, { id: 'lit', name: 'Lit', duration_ms: 400, overrides: { body: { rotation: [0, 180, 0] } } }],
  default_state: 'idle',
};

let seeded: { wsId: string; sceneId: string } | null = null;
async function seed(): Promise<{ wsId: string; sceneId: string }> {
  if (seeded) return seeded;
  const { ctx, base } = await apiCtx();
  try {
    const wsId = await seedWorkspace(ctx, base, 'Studio3D layout');
    const proj = await ctx.post(`${base}/api/v1/design/projects`, { data: { workspace_id: wsId, name: `Studio3D layout ${Date.now()}` } });
    expect(proj.ok()).toBeTruthy();
    const art = await ctx.post(`${base}/api/v1/design/artifacts`, {
      data: { workspace_id: wsId, project_id: ((await proj.json()) as { id: string }).id, format: 'scene3d', studio: '3d', title: 'Layout Lantern', content: JSON.stringify(LANTERN) },
    });
    expect(art.ok()).toBeTruthy();
    seeded = { wsId, sceneId: ((await art.json()) as { artifact: { id: string } }).artifact.id };
    return seeded;
  } finally {
    await ctx.dispose();
  }
}

async function openStudio(page: Page, scheme: 'dark' | 'light' = 'dark'): Promise<void> {
  const s = await seed();
  await page.addInitScript(([ws, sc]) => {
    localStorage.setItem('otto_workspace', ws);
    localStorage.setItem('otto_scheme', sc);
  }, [s.wsId, scheme]);
  await openPage(page, `design/a/${encodeURIComponent(s.sceneId)}`);
  await expect(page.getByTestId('s3d-stats')).toBeVisible({ timeout: 30_000 });
}

/** Model pixels (dark metal, violet body, gold ring) read back from the WebGL canvas; `topBand` counts
 *  those under the viewport tools (their row band, across the width the tools take with an object
 *  selected: Move / Rotate / Scale + Frame). */
async function modelPixels(page: Page): Promise<{ total: number; topBand: number; pixels: number }> {
  return page.evaluate(async () => {
    const canvas = document.querySelector<HTMLCanvasElement>('.s3d-viewport canvas')!;
    const hud = document.querySelector('.s3d-hud')!.getBoundingClientRect();
    const cr = canvas.getBoundingClientRect();
    const w = 200;
    const h = Math.round((200 * cr.height) / cr.width);
    const c2 = document.createElement('canvas');
    c2.width = w;
    c2.height = h;
    const g = c2.getContext('2d')!;
    g.drawImage(canvas, 0, 0, w, h);
    const d = g.getImageData(0, 0, w, h).data;
    const band = Math.ceil(((hud.bottom - cr.top + 4) / cr.height) * h);
    const bandRight = Math.ceil(((Math.max(hud.right, hud.left + 280) - cr.left) / cr.width) * w);
    let total = 0;
    let topBand = 0;
    for (let y = 0; y < h; y++) {
      for (let x = 0; x < w; x++) {
        const i = (y * w + x) * 4;
        const [r, gr, b] = [d[i], d[i + 1], d[i + 2]];
        const lum = (0.2126 * r + 0.7152 * gr + 0.0722 * b) / 255;
        const model = lum < 0.3 || (b > r + 30 && b > gr + 60) || (r > 150 && gr > 110 && b < 90);
        if (!model) continue;
        total++;
        if (y < band && x < bandRight) topBand++;
      }
    }
    return { total, topBand, pixels: w * h };
  });
}

const coarsePointer = (page: Page) => page.evaluate(() => matchMedia('(pointer: coarse)').matches);
const shots = process.env.OTTO_E2E_SHOTS;
async function shot(page: Page, name: string): Promise<void> {
  if (!shots) return;
  // Let a menu or sheet finish its entry animation.
  await page.evaluate(() =>
    Promise.all(document.getAnimations().filter((a) => a.effect?.getComputedTiming().endTime !== Infinity).map((a) => a.finished.catch(() => undefined))));
  await page.screenshot({ path: `${shots}/${test.info().project.name}-${name}.png` });
}

/** Visible box heights (px) of the controls matching `sel` inside `root`. */
async function heights(page: Page, root: string, sel: string): Promise<{ label: string; h: number }[]> {
  return page.locator(root).first().evaluate((el, s) =>
    [...el.querySelectorAll<HTMLElement>(s)].filter((e) => e.getClientRects().length && getComputedStyle(e).visibility !== 'hidden')
      .map((e) => ({ label: (e.getAttribute('aria-label') || e.textContent || e.tagName).trim().slice(0, 30), h: Math.round(e.getBoundingClientRect().height) })), sel);
}
async function expectTouchSized(page: Page, root: string, sel: string, what: string): Promise<void> {
  const hs = await heights(page, root, sel);
  expect.soft(hs.length, `${what}: controls present`).toBeGreaterThan(0);
  for (const c of hs) expect.soft(c.h, `${what} "${c.label}" is a visible 40 px touch target`).toBeGreaterThanOrEqual(40);
}

/** Elements that scroll vertically inside `root` (form fields aside). */
async function scrollAreas(page: Page, root: string): Promise<number> {
  return page.locator(root).first().evaluate((el) =>
    [el as HTMLElement, ...el.querySelectorAll<HTMLElement>('*')].filter((e) => !/^(TEXTAREA|SELECT|INPUT)$/.test(e.tagName) && /auto|scroll/.test(getComputedStyle(e).overflowY) && e.getClientRects().length).length);
}

/** The open dialog: inside the viewport, its footer actions and close button finger-sized. */
async function expectDialogFits(page: Page, name: string): Promise<void> {
  const dlg = page.getByRole('dialog').last();
  await expect(dlg).toBeVisible();
  await expectFullyInViewport(page, dlg, name);
  await expectFullyInViewport(page, dlg.locator('footer'), `${name} footer`);
  expect.soft(await scrollAreas(page, '.sheet[role="dialog"] >> nth=-1'), `${name}: one scroll area (the body), not two side by side`).toBe(1);
  if (await coarsePointer(page)) {
    await expectTouchSized(page, '[role="dialog"] >> nth=-1', 'footer .btn', `${name} footer`);
    await expectTouchSized(page, '[role="dialog"] >> nth=-1', ':scope > header button', `${name} close`);
  }
}

test('3D Studio: one toolbar row, finger-sized controls, canvas first, whole model in view, upright view cube', async ({ page }) => {
  test.setTimeout(90_000);
  await openStudio(page);
  await expectNoHorizontalOverflow(page);
  const coarse = await page.evaluate(() => matchMedia('(pointer: coarse)').matches);

  // ── Toolbar: one row, nothing clipped or scrolled, one primary, the rest under ⋯.
  const bar = page.locator('.studio3d .toolbar');
  await expect(bar).toBeVisible();
  const fit = await bar.evaluate((el) => {
    const r = el.getBoundingClientRect();
    const kids = [...el.querySelectorAll<HTMLElement>('button')].filter((b) => b.offsetParent !== null);
    return {
      height: r.height,
      overflow: el.scrollWidth - el.clientWidth,
      outside: kids.filter((b) => { const k = b.getBoundingClientRect(); return k.left < r.left - 1 || k.right > r.right + 1 || k.top < r.top - 1 || k.bottom > r.bottom + 1; }).map((b) => b.textContent?.trim()),
      primaries: kids.filter((b) => b.classList.contains('primary')).length,
      labels: kids.map((b) => (b.textContent ?? '').trim()),
    };
  });
  expect.soft(fit.height, 'the 3D toolbar is a single row').toBeLessThanOrEqual(60);
  expect.soft(fit.overflow, 'the 3D toolbar never scrolls or clips sideways').toBeLessThanOrEqual(1);
  expect.soft(fit.outside, 'every toolbar control sits fully inside the bar').toEqual([]);
  expect.soft(fit.primaries, 'exactly one primary action').toBe(1);
  expect.soft(fit.labels, 'no near-duplicate "Wire" / "Scale" toolbar buttons').not.toContain('Wire');
  expect.soft(fit.labels).not.toContain('Scale');
  // Touch widths (every project here): the side panes collapse to Scene / Details.
  await expect.soft(page.locator('.studio3d .scene-toggle')).toBeVisible();
  await expect.soft(page.locator('.studio3d .details-toggle')).toBeVisible();

  // ── Canvas first: the viewport gets the studio's full width until a pane is opened.
  const share = await page.evaluate(() => {
    const vp = document.querySelector('.s3d-viewport')!.getBoundingClientRect();
    const st = document.querySelector('[data-testid="studio3d"]')!.getBoundingClientRect();
    return vp.width / st.width;
  });
  expect.soft(share, 'the canvas takes the studio width (side panes collapsed)').toBeGreaterThan(0.95);

  // ── The model renders, and none of it hides under the viewport tools at the top.
  await expect.poll(async () => (await modelPixels(page)).total, { message: 'the canvas draws the model' }).toBeGreaterThan(200);
  const px = await modelPixels(page);
  expect.soft(px.topBand, 'no part of the model is under the top-left viewport tools').toBeLessThanOrEqual(3);
  await shot(page, 'dark-default');

  // ── The stats chip and the States bar never touch (they stack on a narrow canvas).
  const chipGap = await page.evaluate(() => {
    const a = document.querySelector('[data-testid="s3d-stats"]')!.getBoundingClientRect();
    const b = document.querySelector('[data-testid="s3d-states"]')!.getBoundingClientRect();
    const sideBySide = a.left >= b.right || b.left >= a.right;
    return sideBySide ? Infinity : Math.max(b.top - a.bottom, a.top - b.bottom);
  });
  expect.soft(chipGap, 'the stats chip keeps clear of the States bar').toBeGreaterThanOrEqual(6);

  // ── Header: the artifact's own title stays readable; the breadcrumb gives way first.
  const titleShown = await page.locator('[data-testid="page-header"] h1').first().evaluate((h1) => {
    const text = [...h1.querySelectorAll('*'), h1].flatMap((n) => [...n.childNodes]).filter((n) => n.nodeType === 3 && n.textContent!.includes('Layout Lantern'))[0];
    const r = document.createRange();
    r.selectNodeContents(text!);
    const t = r.getBoundingClientRect();
    const box = (text!.parentElement!.closest('h1 > span') ?? h1).getBoundingClientRect();
    const hb = h1.getBoundingClientRect();
    const crumbs = [...h1.querySelectorAll('.ph-crumb')].map((c) => c.getBoundingClientRect().width);
    return { visible: Math.min(t.right, box.right, hb.right) - Math.max(t.left, box.left, hb.left), natural: t.width, crumbs };
  });
  expect.soft(titleShown.visible, 'the artifact title is visible in the header').toBeGreaterThanOrEqual(Math.min(titleShown.natural, 60) - 1);
  if (titleShown.visible < titleShown.natural - 1) {
    for (const w of titleShown.crumbs) expect.soft(w, 'the breadcrumb truncates before the title does').toBeLessThanOrEqual(32);
  }

  // ── View cube: the camera sits above the target, so the cube's TOP face turns toward us.
  const topFacing = await page.locator('.s3d-cube').evaluate((el) => {
    const m = new DOMMatrix(getComputedStyle(el).transform);
    return m.transformPoint(new DOMPoint(0, -1, 0, 0)).z; // CSS y points down: (0,-1,0) is "up"
  });
  expect.soft(topFacing, 'view cube top face faces the viewer when the camera is above').toBeGreaterThan(0);

  // ── Touch targets: header status pill, States pills + buttons, viewport tools.
  if (coarse) {
    const heights = await page.evaluate(() => {
      const h = (sel: string) => [...document.querySelectorAll<HTMLElement>(sel)].filter((e) => e.offsetParent !== null).map((e) => Math.round(e.getBoundingClientRect().height));
      return {
        status: h('[data-testid="design-status"]'),
        states: h('[data-testid="s3d-states"] button'),
        hud: h('.s3d-hud button'),
        more: h('[data-testid="s3d-more"]'),
      };
    });
    for (const [what, hs] of Object.entries(heights)) {
      expect.soft(hs.length, `${what} controls present`).toBeGreaterThan(0);
      for (const v of hs) expect.soft(v, `${what} touch target height`).toBeGreaterThanOrEqual(40);
    }
  }

  // ── ⋯ holds the rest, inside the viewport.
  await page.getByTestId('s3d-more').click();
  const menu = page.getByRole('menu').first();
  await expectFullyInViewport(page, menu, '3D toolbar ⋯ menu');
  await expect(menu.getByText(/Scale reference/)).toBeVisible();
  await expect(menu.getByText('Turntable')).toBeVisible();
  await page.keyboard.press('Escape');

  // ── Details: the inspector says when the environment backdrop covers the background colour.
  await page.locator('.studio3d .details-toggle').click();
  await expect(page.getByTestId('s3d-bg-covered')).toBeVisible();
  await expectNoHorizontalOverflow(page);
  // A tab past the pane's edge ("Links") says so with a visible "More tabs" control.
  const tabsHidden = await page.locator('#s3d-details-panel [role="tablist"]').evaluate((el) => el.scrollWidth > el.clientWidth + 1);
  if (tabsHidden) await expect.soft(page.locator('#s3d-details-panel').getByTestId('tabs-more'), 'hidden tabs have a visible affordance').toBeVisible();
  if (coarse) {
    await expectTouchSized(page, '#s3d-details-panel', '[role="tab"]', 'Details tab');
    await expectTouchSized(page, '#s3d-details-panel', '.nd, input:not([type="checkbox"], [type="range"], [type="color"], .nd-input), select', 'Inspector field');
  }
  await shot(page, 'dark-details-pane');
  // Scene: the sub-mesh list never scrolls sideways.
  await page.locator('.studio3d .scene-toggle').click();
  if (coarse) {
    await expectTouchSized(page, '.studio3d .left', '[role="treeitem"], [role="treeitem"] .s3d-icon-btn', 'Hierarchy row');
    // One scroll area: the pane, not the hierarchy and sub-mesh lists inside it as well.
    expect.soft(await scrollAreas(page, '.studio3d'.concat(' .left')), 'the Scene pane scrolls as a whole').toBe(1);
  }
  // Truncated sub-mesh names keep their distinguishing end ("… base" / "… body").
  const tails = await page.locator('[data-testid="s3d-mesh-outliner"] .mo-name').evaluateAll((els) =>
    els.map((b) => {
      const t = b.querySelector('.mo-t');
      const r = t?.getBoundingClientRect();
      const box = b.getBoundingClientRect();
      return { name: b.getAttribute('title') ?? '', tail: t?.textContent ?? '', shown: !!r && r.width > 0 && r.right <= box.right + 1 };
    }));
  for (const n of ['Lantern base', 'Lantern body']) {
    const row = tails.find((x) => x.name.startsWith(`Focus ${n}`));
    expect.soft(row?.shown && n.endsWith(row.tail.trim()), `"${n}" keeps its last word visible`).toBe(true);
  }
  await shot(page, 'dark-scene-pane');
  const subMesh = await page.locator('[data-testid="s3d-mesh-outliner"] .mo-list').evaluate((el) => {
    const r = el.getBoundingClientRect();
    return {
      // A horizontal scrollbar needs a scrollable overflow-x AND wider content
      // (touch hit areas may poke past the edge invisibly).
      scrolls: !/hidden|clip/.test(getComputedStyle(el).overflowX) && el.scrollWidth > el.clientWidth + 1,
      cut: [...el.querySelectorAll('.mo-row > *')].filter((k) => k.getBoundingClientRect().right > r.right + 1).length,
    };
  });
  expect(subMesh, 'sub-mesh rows fit the pane: no sideways scrollbar, no clipped buttons').toEqual({ scrolls: false, cut: 0 });
});

test('3D Studio: selecting an object shows 40 px gizmo modes; light theme toolbar still fits', async ({ page }) => {
  test.setTimeout(60_000);
  await openStudio(page, 'light');
  const canvas = page.locator('.s3d-viewport canvas');
  const box = (await canvas.boundingBox())!;
  // Tap the body (the target sits in the middle of the frame).
  await canvas.click({ position: { x: box.width / 2, y: box.height / 2 } });
  const modes = page.locator('[data-testid="s3d-hud"] [role="radio"]');
  await expect(modes).toHaveCount(3);
  const hudFit = await page.getByTestId('s3d-hud').evaluate((el) => {
    const r = el.getBoundingClientRect();
    return { right: r.right, vw: innerWidth, rows: Math.round(r.height) };
  });
  expect(hudFit.right, 'viewport tools stay on screen').toBeLessThanOrEqual(hudFit.vw);
  const cubeGap = await page.evaluate(() => {
    const cube = document.querySelector('.s3d-cube')!.getBoundingClientRect();
    const tools = [...document.querySelectorAll('[data-testid="s3d-hud"] button')].map((b) => b.getBoundingClientRect());
    return Math.min(...tools.filter((t) => t.bottom > cube.top && t.top < cube.bottom).map((t) => cube.left - t.right), Infinity);
  });
  expect.soft(cubeGap, 'the viewport tools keep clear of the view cube').toBeGreaterThanOrEqual(4);
  await shot(page, 'light-object-selected');
  if (await page.evaluate(() => matchMedia('(pointer: coarse)').matches)) {
    for (const hgt of await modes.evaluateAll((els) => els.map((e) => e.getBoundingClientRect().height))) expect(hgt).toBeGreaterThanOrEqual(40);
  }
  const overflow = await page.getByTestId('s3d-toolbar').evaluate((el) => el.scrollWidth - el.clientWidth);
  expect(overflow).toBeLessThanOrEqual(1);
  await expectNoHorizontalOverflow(page);

  // Generate menu: every item reads in full (the first one was cut to "…TREL…").
  await page.getByTestId('s3d-generate').click();
  const cut = await page.getByRole('menu').first().locator('[role="menuitem"]').evaluateAll((items) =>
    items.filter((i) => [i, ...i.querySelectorAll<HTMLElement>('*')].some((e) => e.scrollWidth > e.clientWidth + 1 && getComputedStyle(e).overflow !== 'visible')).map((i) => i.textContent?.trim()));
  expect.soft(cut, 'no Generate menu item is truncated').toEqual([]);
  await shot(page, 'light-generate-menu');
  // Generate modal: on screen, finger-sized actions, and its body never grows a sideways scrollbar.
  await page.getByRole('menuitem', { name: /Generate 3D asset/ }).click();
  const body = page.getByTestId('gen3d-modal');
  await expectDialogFits(page, '3D Generate modal');
  // A keyboard focus ring inside the dialog is drawn whole (the scrolling body clipped it).
  await page.keyboard.press('Tab');
  await page.keyboard.press('Shift+Tab');
  const ring = await page.evaluate(() => {
    const el = document.activeElement as HTMLElement;
    if (!el?.matches(':focus-visible') || !el.closest('.sheet-body')) return null;
    const s = getComputedStyle(el);
    const grow = Math.max(0, parseFloat(s.outlineOffset) + parseFloat(s.outlineWidth));
    const r = el.getBoundingClientRect();
    let clipped = false;
    for (let n = el.parentElement; n && !n.matches('.sheet'); n = n.parentElement) {
      const cs = getComputedStyle(n);
      if (cs.overflowX === 'visible' && cs.overflowY === 'visible') continue;
      const c = n.getBoundingClientRect();
      if (r.top - grow < c.top - 0.5 || r.left - grow < c.left - 0.5 || r.right + grow > c.right + 0.5 || r.bottom + grow > c.bottom + 0.5) clipped = true;
    }
    return { label: el.textContent?.trim().slice(0, 30), clipped };
  });
  if (ring) expect.soft(ring.clipped, `the focus ring on "${ring.label}" is not clipped by the dialog`).toBe(false);
  await shot(page, 'light-generate-modal');
  const scrollsSideways = await body.evaluate((el) => {
    for (let n: Element | null = el; n; n = n.parentElement) {
      if (n.getAttribute('role') === 'dialog') break;
      if (/auto|scroll/.test(getComputedStyle(n).overflowX) && n.scrollWidth > n.clientWidth + 1) return true;
    }
    return false;
  });
  expect(scrollsSideways, 'the Generate modal body has no horizontal scrollbar').toBe(false);
  // The provider manager it opens (a second sheet on top).
  await body.getByText('Advanced', { exact: true }).click();
  await body.getByTestId('gen3d-manage-providers').click();
  await expect(page.getByRole('dialog', { name: '3D generation providers' })).toBeVisible();
  await expectDialogFits(page, '3D providers modal');
  // Otto Forge is the built-in card (its binary auto-detected); no second, bare-`forge` copy is offered.
  const pm = page.getByRole('dialog', { name: '3D generation providers' });
  await expect(pm.locator('.pm-card', { hasText: 'Otto Forge (local)' })).toHaveCount(1);
  await expect(pm.getByTestId('gen3d-add-forge')).toHaveCount(0);
  await shot(page, 'light-provider-modal');
});

test('3D Studio: tapping a comment pin with Scene open shows Details below the canvas', async ({ page }, info) => {
  test.skip(!/^(iphone-portrait|iphone-se|android-phone)$/.test(info.project.name), 'phone portrait only');
  // Its own scene: the pin's comment would change the shared lantern's Comments count.
  const s = await seed();
  const { ctx, base } = await apiCtx();
  let sceneId = '';
  try {
    const art = await ctx.post(`${base}/api/v1/design/artifacts`, {
      data: { workspace_id: s.wsId, format: 'scene3d', studio: '3d', title: 'Pinned Lantern', content: JSON.stringify(LANTERN) },
    });
    expect(art.ok(), await art.text()).toBeTruthy();
    sceneId = ((await art.json()) as { artifact: { id: string } }).artifact.id;
    const c = await ctx.post(`${base}/api/v1/design/artifacts/${sceneId}/comments`, {
      data: { body: 'Brighter body', pin_3d: { selection_kind: 'point', object_id: 'body', position: [0, 0.95, 0.45] } },
    });
    expect(c.ok(), await c.text()).toBeTruthy();
  } finally {
    await ctx.dispose();
  }
  await page.addInitScript((ws) => localStorage.setItem('otto_workspace', ws), s.wsId);
  await openPage(page, `design/a/${encodeURIComponent(sceneId)}`);
  await expect(page.getByTestId('s3d-stats')).toBeVisible({ timeout: 30_000 });
  await page.locator('.studio3d .scene-toggle').click();
  await expect(page.getByTestId('s3d-left')).toBeVisible();
  await page.getByTestId('s3d-pin-marker').first().click();
  // Scene + Details are both open now; Details takes the lower half, the canvas keeps the top.
  const details = page.locator('#s3d-details-panel');
  await expect(details.getByTestId('design-tab-comments')).toHaveAttribute('aria-selected', 'true');
  const box = (await details.boundingBox())!;
  expect(box.width, 'Details spans the screen').toBeGreaterThan(page.viewportSize()!.width - 2);
  expect(box.height, 'Details has room').toBeGreaterThan(120);
  await expectFullyInViewport(page, details.locator('[role="tablist"]'), 'Details tabs');
  expect((await page.locator('.s3d-viewport').boundingBox())!.width, 'the canvas keeps the width').toBeGreaterThan(page.viewportSize()!.width - 2);
});

test('3D Studio: on a short landscape phone (browser bars shown) the stats pill and States bar stay on screen', async ({ page }, info) => {
  test.skip(!/phone-landscape|iphone-landscape/.test(info.project.name), 'phone landscape only');
  // A real Pixel 7 in Chrome landscape leaves ≈300 px of page under the URL bar.
  await page.setViewportSize({ width: 846, height: 300 });
  await openStudio(page);
  await expectFullyInViewport(page, page.getByTestId('s3d-stats'), 'stats pill');
  await expectFullyInViewport(page, page.getByTestId('s3d-states'), 'States bar');
  // The canvas gets the screen: the toolbar floats over it instead of taking a row.
  const share = await page.evaluate(() => document.querySelector('.s3d-viewport')!.getBoundingClientRect().height / innerHeight);
  expect.soft(share, 'the 3D canvas takes ≥ 70 % of a 300 px landscape page').toBeGreaterThanOrEqual(0.7);
  await expect.poll(async () => (await modelPixels(page)).total, { message: 'the model renders' }).toBeGreaterThan(100);
  expect.soft((await modelPixels(page)).topBand, 'no part of the model is under the floating toolbar / viewport tools').toBeLessThanOrEqual(3);
  await shot(page, 'dark-short-landscape-846x300');
  // ⋯ fits a real landscape page and scrolls when it is longer than the screen.
  await page.getByTestId('s3d-more').click();
  const menu = page.getByRole('menu').first();
  await expectFullyInViewport(page, menu, '⋯ menu on a 300 px page');
  const scroll = await menu.evaluate((m) => ({ overflows: m.scrollHeight > m.clientHeight + 1, scrolls: /auto|scroll/.test(getComputedStyle(m).overflowY) }));
  if (scroll.overflows) expect.soft(scroll.scrolls, 'a menu taller than the page scrolls').toBe(true);
  await shot(page, 'dark-short-landscape-more-menu');
  await page.keyboard.press('Escape');
  // A dialog on that page: inside the screen, footer included.
  await page.getByTestId('s3d-generate').click();
  await page.getByRole('menuitem', { name: /Generate 3D asset/ }).click();
  await expectDialogFits(page, '3D Generate modal on a 300 px page');
  await shot(page, 'dark-short-landscape-generate-modal');
});

test('3D Studio: a small rigged model is framed (not the floor), skeleton overlay + clip play with touch-sized controls', async ({ page }) => {
  test.setTimeout(90_000);
  const s = await seed();
  const { ctx, base } = await apiCtx();
  let glbId = '';
  try {
    const art = await ctx.post(`${base}/api/v1/design/artifacts`, {
      data: { workspace_id: s.wsId, format: 'glb', studio: '3d', title: 'Rigged sample', content_b64: riggedGlbBase64() },
    });
    expect(art.ok(), await art.text()).toBeTruthy();
    glbId = ((await art.json()) as { artifact: { id: string } }).artifact.id;
  } finally {
    await ctx.dispose();
  }
  await page.addInitScript((ws) => {
    localStorage.setItem('otto_workspace', ws);
    localStorage.setItem('otto_scheme', 'light');
  }, s.wsId);
  await openPage(page, `design/a/${encodeURIComponent(glbId)}`);
  await expect(page.getByTestId('s3d-animation-bar')).toBeVisible({ timeout: 30_000 });
  // The model's red pixels span a good part of the canvas height (it was a few px on a 20 m floor).
  const span = async () =>
    page.evaluate(() => {
      const canvas = document.querySelector<HTMLCanvasElement>('.s3d-viewport canvas')!;
      const cr = canvas.getBoundingClientRect();
      const [w, h] = [200, Math.round((200 * cr.height) / cr.width)];
      const g = Object.assign(document.createElement('canvas'), { width: w, height: h }).getContext('2d')!;
      g.drawImage(canvas, 0, 0, w, h);
      const d = g.getImageData(0, 0, w, h).data;
      let top = h;
      let bottom = -1;
      for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
        const i = (y * w + x) * 4;
        if (d[i] > 140 && d[i + 1] < 90 && d[i + 2] < 90) [top, bottom] = [Math.min(top, y), Math.max(bottom, y)];
      }
      return bottom < 0 ? 0 : (bottom - top + 1) / h;
    });
  await expect.poll(span, { message: 'the rigged model fills a readable share of the canvas height', timeout: 15_000 }).toBeGreaterThanOrEqual(0.3);
  if (await coarsePointer(page)) await expectTouchSized(page, '[data-testid="s3d-animation-bar"]', 'button, select', 'Rig / clip control');
  // The clip select shows its whole label (it was cut mid-character), and Play / Skeleton have a visible surface.
  const bar = await page.getByTestId('s3d-animation-bar').evaluate((el) => {
    const sel = el.querySelector<HTMLSelectElement>('select')!;
    const s = getComputedStyle(sel);
    const g = document.createElement('canvas').getContext('2d')!;
    g.font = `${s.fontWeight} ${s.fontSize} ${s.fontFamily}`;
    const room = sel.clientWidth - parseFloat(s.paddingInlineStart) - parseFloat(s.paddingInlineEnd);
    const surface = (b: Element) => { const c = getComputedStyle(b); return !/rgba\(.*,\s*0\)|transparent/.test(c.backgroundColor) || parseFloat(c.borderTopWidth) > 0 && !/rgba\(.*,\s*0\)|transparent/.test(c.borderTopColor); };
    return { fits: g.measureText(sel.selectedOptions[0].text).width <= room + 0.5, play: surface(el.querySelector('[data-testid="s3d-anim-play"]')!), skel: surface(el.querySelector('[data-testid="s3d-skeleton-toggle"]')!) };
  });
  expect.soft(bar.fits, 'the clip select shows its whole label').toBe(true);
  expect.soft(bar.play, 'Play has a visible button surface').toBe(true);
  expect.soft(bar.skel, 'the Skeleton toggle has a visible button surface').toBe(true);
  await page.getByTestId('s3d-skeleton-toggle').click();
  await page.getByTestId('s3d-anim-play').click();
  await expect(page.getByTestId('s3d-skeleton-toggle')).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByRole('button', { name: 'Pause animation' })).toBeVisible();
  await expectNoHorizontalOverflow(page);
  await shot(page, 'light-rigged-skeleton-clip-playing');
});

test('3D Studio: no WebGL → a clear "3D preview unavailable" state with the reason, never a blank canvas', async ({ page }) => {
  await page.addInitScript(() => {
    const orig = HTMLCanvasElement.prototype.getContext;
    // @ts-expect-error narrowing the overloads is not the point here
    HTMLCanvasElement.prototype.getContext = function (this: HTMLCanvasElement, kind: string, ...rest: unknown[]) {
      if (/webgl/i.test(kind)) return null;
      return orig.call(this, kind, ...rest);
    };
  });
  const s = await seed();
  await page.addInitScript((ws) => localStorage.setItem('otto_workspace', ws), s.wsId);
  await openPage(page, `design/a/${encodeURIComponent(s.sceneId)}`);
  const state = page.getByTestId('s3d-unavailable');
  await expect(state).toBeVisible({ timeout: 30_000 });
  await expect(state).toContainText('3D preview unavailable');
  await expect(state).toContainText(/WebGL/);
  await expect(state.getByRole('button', { name: 'Retry' })).toBeVisible();
  if (await coarsePointer(page)) await expectTouchSized(page, '[data-testid="s3d-unavailable"]', 'button', 'Retry');
  await shot(page, 'error-webgl-unavailable');
  await expect(page.getByTestId('s3d-stats'), 'no "saved" status over a dead canvas').toHaveCount(0);
  await expectNoHorizontalOverflow(page);
});
