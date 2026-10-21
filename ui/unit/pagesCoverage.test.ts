// The e2e page sweeps (pages / theme / rtl specs) iterate `PAGES` from
// e2e/helpers.ts, so a sidebar module missing from that list silently never
// gets the axe, RTL-overflow or light/dark checks (Workbench shipped that way).
// helpers.ts imports Playwright + axe, so PAGES is read as source text.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { SIDEBAR_MODULES } from '../src/lib/sidebar.ts';

/** Sidebar modules deliberately NOT in PAGES, as `id: 'reason'` (none today —
 *  every module, including Connections, has its own sweep entry). */
const EXEMPT: Record<string, string> = {};

function pageIds(): string[] {
  const src = readFileSync(join(import.meta.dirname, '..', 'e2e', 'helpers.ts'), 'utf8');
  const start = src.indexOf('export const PAGES = [');
  assert.ok(start >= 0, 'e2e/helpers.ts no longer exports `PAGES = [` — update this guard');
  const body = src.slice(start, src.indexOf('] as const', start));
  return [...body.matchAll(/^\s*'([^']+)',/gm)].map((m) => m[1]);
}

test('every sidebar module is swept by the e2e page lists (or exempted with a reason)', () => {
  const pages = new Set(pageIds());
  assert.ok(pages.size > 10, `parsed only ${pages.size} PAGES entries — the parser is stale`);
  const missing = SIDEBAR_MODULES.map((m) => m.id).filter((id) => !pages.has(id) && !(id in EXEMPT));
  assert.deepEqual(missing, [], 'add these ids to PAGES in e2e/helpers.ts, or to EXEMPT here with a reason');
});

test('the exemption list names only real sidebar modules that PAGES lacks', () => {
  const ids = new Set(SIDEBAR_MODULES.map((m) => m.id));
  const pages = new Set(pageIds());
  for (const id of Object.keys(EXEMPT)) {
    assert.ok(ids.has(id), `EXEMPT names '${id}', which is not a sidebar module any more`);
    assert.ok(!pages.has(id), `'${id}' is in PAGES now — drop its exemption`);
  }
});
