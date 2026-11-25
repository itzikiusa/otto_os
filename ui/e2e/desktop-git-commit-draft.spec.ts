import { test } from '@playwright/test';
import { expectDraftFailureSaysWhy, expectDraftStreams } from './git-commit-card-fixture';

// "Draft from the diff" on the commit card, end to end with the daemon's
// headless draft (0.5.1 hotfix: it used to drive an interactive agent TUI
// and could hang a minute on a start-up screen before failing).
test.use({ serviceWorkers: 'block' });

test('the draft streams into the card; Stop keeps what arrived; it commits', async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  await page.setViewportSize({ width: 1440, height: 900 });
  await expectDraftStreams(page, 'e2e-draft-stream');
});

test('a failed draft says why in one line, with Retry', async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  await expectDraftFailureSaysWhy(page, 'e2e-draft-fail');
});
