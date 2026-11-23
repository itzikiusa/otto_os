import { test } from '@playwright/test';
import { samplesPresent, seedCreate, type CreateSeed } from './create-fixture';
import { agentPickFlow, characterFlow, smartFlow, smartRealServer, soundAndKit, voiceFlow } from './create-flows';
import { progressSamplesPresent } from './create-progress-fixture';
import { inProgressListed, progressFlow } from './create-progress-flows';
import { settingFlow } from './create-setting-flow';

// Create on every phone and tablet, portrait and landscape: the Smart flow
// with the owner's scene, the one-button character, a voice, sounds and the
// missing kit — the same plain-words checks as desktop (no jargon, one main
// button, 44 px targets, no sideways scrolling).

test.use({ serviceWorkers: 'block' });
test.skip(!samplesPresent(), 'needs the Otto Forge sample models');
// One after another, in one worker: the daemon has one engine (each seed wires its own fake
// one) and makes one creation at a time, so the real-server runs here must not overlap.
test.describe.configure({ mode: 'default' });

let seed: CreateSeed;
test.beforeAll(async () => {
  seed = await seedCreate({ step: 0.6 });
});

test('Smart mode with the owner’s scene', async ({ page }, info) => {
  test.setTimeout(180_000);
  await smartFlow(page, seed, info);
});

test('One character, made and changed', async ({ page }, info) => {
  test.setTimeout(300_000);
  await characterFlow(page, seed, info);
});

// The real server on a phone, a phone on its side and a tablet (each run makes
// every piece one at a time, so not on all nine).
test('Smart mode on the real productions server', async ({ page }, info) => {
  test.skip(!['iphone-portrait', 'iphone-landscape', 'ipad-portrait'].includes(info.project.name), 'real-server run on three touch sizes');
  test.setTimeout(600_000);
  await smartRealServer(page, seed, info);
});

test('Voice, sound and the missing kit', async ({ page }, info) => {
  test.setTimeout(120_000);
  await voiceFlow(page, seed, info);
  await soundAndKit(page, seed, info);
});

test('Making at real-life pace on a phone: clock, pictures, the scene so far, a piece opened alive, two at once', async ({ page }, info) => {
  test.skip(!progressSamplesPresent(), 'needs the Smart acceptance run outputs');
  test.setTimeout(240_000);
  await progressFlow(page, seed, info);
});

test('My creations lists a creation started moments ago, without a reload, and keeps it up to date', async ({ page }, info) => {
  test.setTimeout(240_000);
  await inProgressListed(page, seed, info);
});

test('Smart mode on a phone: a pier at night in fog over water, then made daytime with nothing made again', async ({ page }, info) => {
  test.setTimeout(420_000);
  await settingFlow(page, seed, info);
});

test('Who plans it: the chip picks the agent and model the production is planned with', async ({ page }, info) => {
  test.setTimeout(90_000);
  await agentPickFlow(page, seed, info);
});
