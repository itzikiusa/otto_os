import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { budgetMs, dist, isDesktopProject } from './perf';

// ─────────────────────────────────────────────────────────────────────────────
// Smooth sidebar navigation (round 3, W2-D). Every module page is its own lazy
// chunk (shell/pages.svelte.ts) and the router holds the current route until
// the next page's chunk is in, so a sidebar switch must:
//  - never unmount or re-create the shell / sidebar, and never show a boot
//    screen (the root-boot remount fixed in e88f1b88 / c14b1d5e);
//  - never paint an empty content column (the old page stays until the new
//    one mounts in the same frame);
//  - not move or resize the sidebar;
//  - paint the new page's header within budget, timed from the click to the
//    end of the frame that shows it (after paint, not at a microtask).
// Runs in desktop-browser and desktop-webkit (the engine closest to the app).
// ─────────────────────────────────────────────────────────────────────────────

const ROUTE = ['git', 'connections', 'home', 'vault', 'agents'] as const;
type Stop = (typeof ROUTE)[number];

/** Header paint budget per switch (a warm page: its chunk is in). */
const HEADER_BUDGET_MS = 150;

let wsId = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  wsId = await seedWorkspace(ctx, base);
  await ctx.dispose();
});

test.beforeEach(async ({ page }, info) => {
  test.skip(!isDesktopProject(info.project.name), 'desktop projects only');
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_rail_expanded', '1');
  }, wsId);
});

/** Install the in-page probes: shell/sidebar removal and boot screens (a
 *  MutationObserver), empty-content frames (a rAF loop that only queries —
 *  no layout reads), and sidebar resizes (a ResizeObserver). */
async function installProbes(page: Page): Promise<void> {
  await page.evaluate(() => {
    const shell = document.querySelector('.shell');
    const sidebar = document.querySelector('.shell .sidebar');
    if (!shell || !sidebar) throw new Error('shell not mounted');
    const st = {
      shell,
      sidebar,
      removed: 0,
      boot: 0,
      emptyFrames: 0,
      frames: 0,
      resizes: -1, // the observer's initial callback is not a resize
      on: true,
      t0: 0,
      t: -1,
    };
    (window as unknown as { __nav: typeof st }).__nav = st;
    const hit = (n: Node, sel: string): boolean => n instanceof Element && (n.matches(sel) || !!n.querySelector(sel));
    new MutationObserver((recs) => {
      for (const r of recs) {
        for (const n of r.removedNodes) if (hit(n, '.shell, .sidebar')) st.removed++;
        for (const n of r.addedNodes) if (hit(n, '.boot')) st.boot++;
      }
    }).observe(document.body, { childList: true, subtree: true });
    new ResizeObserver(() => st.resizes++).observe(sidebar);
    const loop = (): void => {
      if (!st.on) return;
      st.frames++;
      const content = document.querySelector('.shell .content');
      if (!content || content.childElementCount === 0) st.emptyFrames++;
      requestAnimationFrame(loop);
    };
    requestAnimationFrame(loop);
  });
}

/** Is `stop`'s page on screen? Data, not code: the page checks a selector
 *  or a header title (each a query, never a layout read). */
type ReadySpec = { sel: string } | { title: string };
function readySpec(stop: Stop): ReadySpec {
  switch (stop) {
    case 'git':
      return { sel: '.shell .content .git-header' };
    case 'connections':
      return { title: 'Connections' };
    case 'home':
      return { title: 'Home' };
    case 'vault':
      return { sel: '.shell .content .vault-header' };
    case 'agents':
      // The session tab bar renders only while the Agents page is current.
      return { sel: '[data-testid="agents-history-btn"]' };
  }
}

async function navigate(page: Page, stop: Stop): Promise<number> {
  await page.evaluate((ready) => {
    const st = (window as unknown as { __nav: { t0: number; t: number } }).__nav;
    st.t = -1;
    const isReady = (): boolean =>
      'sel' in ready
        ? !!document.querySelector(ready.sel)
        : [...document.querySelectorAll('.shell .content h1.ph-title')].some(
            (h) => h.textContent?.trim() === ready.title,
          );
    window.addEventListener(
      'click',
      () => {
        st.t0 = performance.now();
        const poll = (): void => {
          if (!isReady()) return void requestAnimationFrame(poll);
          // This frame paints the header: end the sample after its paint.
          const ch = new MessageChannel();
          ch.port1.onmessage = () => {
            ch.port1.close();
            st.t = performance.now() - st.t0;
          };
          ch.port2.postMessage(null);
        };
        requestAnimationFrame(poll);
      },
      { capture: true, once: true },
    );
  }, readySpec(stop));
  await page.locator(`.shell .sidebar [data-nav-id="${stop}"]`).first().click();
  const handle = await page.waitForFunction(
    () => {
      const t = (window as unknown as { __nav: { t: number } }).__nav.t;
      return t >= 0 ? t : null;
    },
    null,
    { timeout: 20_000 },
  );
  return (await handle.jsonValue()) as number;
}

test('sidebar switches keep the shell mounted, never blank, and paint the next header within budget', async ({ page }) => {
  test.setTimeout(120_000);
  await page.goto('/#/agents');
  await expect(page.locator('.shell .sidebar [data-nav-id="git"]').first()).toBeVisible({ timeout: 30_000 });
  await expect(page.locator('[data-testid="agents-history-btn"]')).toBeVisible();
  // Give the main window's idle prefetch its window, as a person would.
  await page.waitForTimeout(2_000);
  await installProbes(page);

  const rect = () =>
    page.evaluate(() => {
      const r = document.querySelector('.shell .sidebar')!.getBoundingClientRect();
      return [r.x, r.y, r.width, r.height].map(Math.round).join(',');
    });
  const rect0 = await rect();

  // Pass 1 may still load a chunk on click; pass 2 is every page warm.
  const cold: Record<string, number> = {};
  const warm: Record<string, number> = {};
  for (const stop of ROUTE) {
    cold[stop] = await navigate(page, stop);
    expect(await rect(), `sidebar moved after switching to ${stop}`).toBe(rect0);
  }
  for (const stop of ROUTE) {
    warm[stop] = await navigate(page, stop);
    expect(await rect(), `sidebar moved after switching to ${stop}`).toBe(rect0);
  }

  const probe = await page.evaluate(() => {
    const st = (window as unknown as {
      __nav: { shell: Element; sidebar: Element; removed: number; boot: number; emptyFrames: number; frames: number; resizes: number; on: boolean };
    }).__nav;
    st.on = false;
    return {
      sameShell: st.shell.isConnected && st.shell === document.querySelector('.shell'),
      sameSidebar: st.sidebar.isConnected && st.sidebar === document.querySelector('.shell .sidebar'),
      removed: st.removed,
      boot: st.boot,
      emptyFrames: st.emptyFrames,
      frames: st.frames,
      resizes: Math.max(0, st.resizes),
    };
  });
  // eslint-disable-next-line no-console
  console.log(`[nav-smooth] cold ${JSON.stringify(cold)} warm ${JSON.stringify(warm)} warm-dist ${JSON.stringify(dist(Object.values(warm)))} probe ${JSON.stringify(probe)}`);

  expect(probe.sameShell, 'the shell root was re-created').toBe(true);
  expect(probe.sameSidebar, 'the sidebar was re-created').toBe(true);
  expect(probe.removed, 'shell/sidebar nodes removed during navigation').toBe(0);
  expect(probe.boot, 'a boot screen appeared during navigation').toBe(0);
  expect(probe.emptyFrames, 'frames with an empty content column').toBe(0);
  expect(probe.resizes, 'sidebar resized during navigation').toBe(0);
  for (const stop of ROUTE) {
    expect(warm[stop], `${stop} header painted ${warm[stop].toFixed(1)} ms after the click`).toBeLessThanOrEqual(budgetMs(HEADER_BUDGET_MS));
  }
});
