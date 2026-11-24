import { test } from '@playwright/test';
import { expectDraftFailureSaysWhy, expectDraftStreams } from './git-commit-card-fixture';

// "Draft from the diff" on a phone: the same streamed draft, Stop and Retry
// as the desktop spec (desktop-git-commit-draft.spec.ts).
test.use({ serviceWorkers: 'block' });

const PHONES = ['iphone-portrait', 'android-phone'];

test('phone: the draft streams into the card; Stop keeps what arrived', async ({ page }, info) => {
  test.skip(!PHONES.includes(info.project.name), 'phone projects only');
  await expectDraftStreams(page, `e2e-draft-stream-${info.project.name}`);
});

test('phone: a failed draft says why, with Retry', async ({ page }, info) => {
  test.skip(!PHONES.includes(info.project.name), 'phone projects only');
  await expectDraftFailureSaysWhy(page, `e2e-draft-fail-${info.project.name}`);
});
