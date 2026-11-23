import { test, expect } from '@playwright/test';
import { samplesPresent, seedCreate, type CreateSeed } from './create-fixture';
import { agentPickFlow, characterFlow, objectFlow, smartFlow, smartRealServer, soundAndKit, voiceFlow } from './create-flows';
import { progressSamplesPresent } from './create-progress-fixture';
import { inProgressListed, progressFlow } from './create-progress-flows';
import { settingFlow } from './create-setting-flow';
import { projectFlow } from './create-project-flow';

// Create — the plain-words front door — on the desktop shell: Smart mode with
// the owner's own scene (productions API mocked in the page), the one-button
// character through a fake local engine, an object, a voice, sounds and the
// missing-kit state, each screen checked for jargon, one main button, big
// targets and no sideways scrolling.

test.use({ serviceWorkers: 'block' });
test.skip(!samplesPresent(), 'needs the Otto Forge sample models');
// One after another, in one worker: the daemon has one engine (each seed wires its own fake
// one) and makes one creation at a time, so the real-server runs here must not overlap.
test.describe.configure({ mode: 'default' });

let seed: CreateSeed;
test.beforeAll(async () => {
  seed = await seedCreate();
});

test('Smart mode: the owner’s scene from one description, steered in plain words', async ({ page }, info) => {
  test.setTimeout(150_000);
  await smartFlow(page, seed, info);
});

test('Smart mode on the real productions server: plan, make, scene, steer, keep new', async ({ page }, info) => {
  test.setTimeout(600_000);
  await smartRealServer(page, seed, info);
});

test('One character: Make it builds the whole moving character, and Change something remakes it', async ({ page }, info) => {
  test.setTimeout(240_000);
  await characterFlow(page, seed, info);
});

test('One object: no skeleton steps and no voice', async ({ page }) => {
  test.setTimeout(120_000);
  await objectFlow(page, seed);
});

test('Voice: pick who, how they sound, type the lines, make them talk', async ({ page }, info) => {
  test.setTimeout(90_000);
  await voiceFlow(page, seed, info);
});

test('Sound waits calmly and a missing kit is one friendly state', async ({ page }, info) => {
  await soundAndKit(page, seed, info);
});

test('Create is Product’s first tab and the sidebar lands on it', async ({ page }) => {
  await page.addInitScript((ws) => localStorage.setItem('otto_workspace', ws), seed.wsId);
  await page.goto('/#/product');
  const tabs = page.getByRole('tablist', { name: 'View' });
  await expect(tabs.getByRole('tab').first()).toHaveText('Create');
  await tabs.getByRole('tab', { name: 'Create' }).click();
  await expect(page).toHaveURL(/#\/design\/create$/);
  await expect(page.getByRole('heading', { name: 'What do you want to make?' })).toBeVisible();
});

test('Making at real-life pace: a ticking clock, pictures on rows, the scene so far, a piece opened alive, two at once', async ({ page }, info) => {
  test.skip(!progressSamplesPresent(), 'needs the Smart acceptance run outputs');
  test.setTimeout(240_000);
  await progressFlow(page, seed, info);
});

test('My creations lists a creation started moments ago, without a reload, and keeps it up to date', async ({ page }, info) => {
  test.setTimeout(240_000);
  await inProgressListed(page, seed, info);
});

test('Smart mode: a pier at night in fog over water, then made daytime with nothing made again', async ({ page }, info) => {
  test.setTimeout(420_000);
  await settingFlow(page, seed, info);
});

test('Who plans it: the chip picks the agent and model the production is planned with', async ({ page }, info) => {
  test.setTimeout(90_000);
  await agentPickFlow(page, seed, info);
});

test('A project: a new scene reuses the people, things and place already made, and is done at once', async ({ page }, info) => {
  test.setTimeout(420_000);
  await projectFlow(page, seed, info);
});
