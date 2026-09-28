// Real two-person room capture. Only media inputs are synthetic: a fictional
// checkout screen and macOS speech. HTTP, WS, PTY, WebRTC, consent, recognition,
// OCR and subscription-authenticated summary all run through production code.
// Requires existing whisper.cpp + model and signed-in Codex; never mocks a result.
import {execFileSync} from 'node:child_process';
import {mkdirSync, writeFileSync, readFileSync, existsSync, chmodSync} from 'node:fs';
import {dirname, join, resolve} from 'node:path';
import {fileURLToPath, pathToFileURL} from 'node:url';
import {startStack, client, storageState, UI} from './lib/stack.mjs';
import {record, CURSOR_SCRIPT} from './lib/recorder.mjs';
const tour = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repo = resolve(tour, '../../..');
const {chromium} = await import(pathToFileURL(join(repo, 'ui/node_modules/playwright/index.mjs')));
const out = join(tour, 'public/capture/rooms'); mkdirSync(out, {recursive: true});
const cache = join(tour, '.cache'); mkdirSync(cache, {recursive: true});
const whisper = process.env.OTTO_TOUR_WHISPER ?? '/tmp/otto-room-whisper-probe/build/bin/whisper-cli';
const model = process.env.OTTO_TOUR_WHISPER_MODEL ?? '/tmp/otto-room-whisper-probe/models/ggml-small.bin';
const codex = process.env.OTTO_TOUR_CODEX ?? execFileSync('which', ['codex'], {encoding: 'utf8'}).trim();
if (!process.env.CODEX_HOME || !existsSync(join(process.env.CODEX_HOME, 'auth.json'))) throw new Error('Set CODEX_HOME to an existing subscription-signed-in Codex home.');
for (const p of [whisper, model, codex]) if (!existsSync(p)) throw new Error(`Missing prerequisite: ${p}`);
const speech = 'Let us review the checkout retry flow. The retry button should stay visible after a network timeout. Alex will add a regression test. We will ship only after the test passes. The open question is whether we should retry automatically.';
execFileSync('say', ['-v', 'Samantha', '-r', '165', '-o', join(cache, 'room-speech.aiff'), speech]);
execFileSync('ffmpeg', ['-y', '-v', 'error', '-i', join(cache, 'room-speech.aiff'), '-ar', '48000', '-ac', '1', join(cache, 'room-speech.wav')]);
const wav = readFileSync(join(cache, 'room-speech.wav')).toString('base64');
let stack, browser; const shots = [];
const sleep = ms => new Promise(r => setTimeout(r, ms));
async function beat(page, id, caption, action, tail = 2500) {
  console.log(`[rooms] ${id}`); const rec = await record(page, {width: 1920, height: 1080});
  await sleep(600); await action(); await sleep(tail);
  await page.screenshot({path: join(out, `${id}.png`)});
  await rec.stop(join(out, `${id}.mp4`));
  shots.push({id, caption, file: `${id}.mp4`});
  writeFileSync(join(out, 'shots.json'), JSON.stringify(shots, null, 2));
}
async function click(page, name) { await page.getByRole('button', {name, exact: true}).click(); }
async function media(page, who) {
  await page.addInitScript(({wav, who}) => {
    let audio, source; let serial = 0;
    navigator.mediaDevices.getUserMedia = async () => {
      audio = new AudioContext(); const dest = audio.createMediaStreamDestination();
      const bytes = Uint8Array.from(atob(wav), c => c.charCodeAt(0));
      const buffer = await audio.decodeAudioData(bytes.buffer);
      window.playDemoSpeech = async () => { await audio.resume(); source = audio.createBufferSource(); source.buffer = buffer; source.connect(dest); source.start(); };
      return dest.stream;
    };
    navigator.mediaDevices.getDisplayMedia = async () => {
      const canvas = document.createElement('canvas'); canvas.width = 1280; canvas.height = 720;
      const context = canvas.getContext('2d'); const mine = ++serial;
      let frame = 0; const paint = () => {
        context.fillStyle = '#101827'; context.fillRect(0, 0, 1280, 720);
        context.fillStyle = '#98a9c6'; context.font = '22px -apple-system, sans-serif'; context.fillText('ACME STOREFRONT  /  FICTIONAL DEMO', 50, 52);
        context.fillStyle = '#f1f5fa'; context.font = 'bold 44px -apple-system, sans-serif'; context.fillText(who === 'Maya' ? 'Checkout recovery' : 'Regression test', 50, 134);
        const lines = who === 'Maya' ? ['Network timeout — your cart is saved.', 'Keep the retry action visible.', 'Decision: ship after the regression passes.'] : ['test("retry remains visible after timeout")', '  await checkout.simulateTimeout()', '  expect(retryButton).toBeVisible()', 'Alex: cover timeout and successful retry.'];
        context.font = '28px monospace'; lines.forEach((s, i) => context.fillText(s, 50, 215 + i * 65));
        context.fillStyle = '#207cec'; context.fillRect(50, 490, 315, 68); context.fillStyle = 'white'; context.font = 'bold 27px sans-serif'; context.fillText(who === 'Maya' ? 'Retry checkout' : 'Run regression', 75, 535);
        context.fillStyle = '#50cfaf'; context.fillRect(50, 632, 800 * ((frame++ % 150) / 150), 8);
        context.font = '20px sans-serif'; context.fillText(`${who}’s shared demo · live frame ${frame}`, 50, 682);
      };
      paint(); const timer = setInterval(paint, 100); const stream = canvas.captureStream(10);
      Object.defineProperty(stream.getVideoTracks()[0], 'label', {value: who === 'Maya' ? 'Checkout recovery · demo tab' : 'Regression test · demo tab'});
      stream.getVideoTracks()[0].addEventListener('ended', () => clearInterval(timer));
      return stream;
    };
  }, {wav, who});
  await page.addInitScript(CURSOR_SCRIPT);
}
try {
  stack = await startStack({bin: process.env.OTTO_E2E_BIN ?? join(repo, 'target/debug/ottod'), dist: join(repo, 'ui/dist'), log: join(cache, 'rooms-daemon.log'), prepare: ({fakeBin}) => {
    // Only recap generation invokes Codex; the demonstration session is a real shell.
    // Absolute executable, no copied auth and no credentials in footage/artifacts.
    writeFileSync(join(fakeBin, 'codex'), `#!/bin/sh\nexec '${codex.replaceAll("'", "'\\''")}' "$@"\n`); chmodSync(join(fakeBin, 'codex'), 0o700);
  }});
  const api = client(stack.token);
  await api.put('/room-recap-settings', {whisper_executable: whisper, whisper_model: model, language: 'en', threads: 2});
  const capability = await api.get('/room-recap-capabilities');
  if (!capability.speech_ready) throw new Error(`Speech unavailable: ${JSON.stringify(capability)}`);
  const work = join(stack.home, 'Acme'); mkdirSync(work); writeFileSync(join(work, 'retry-test.mjs'), 'import assert from "node:assert/strict";\nconst timeout = {cartSaved: true, retryVisible: true};\nassert.equal(timeout.retryVisible, true);\nconsole.log("PASS: retry stays visible after a network timeout");\n');
  const ws = await api.post('/workspaces', {name: 'Acme · Team review', root_path: work});
  const session = await api.post(`/workspaces/${ws.id}/sessions`, {kind: 'agent', provider: 'shell', title: 'Checkout · pair review', cwd: work, meta: {origin: 'manual'}});
  await api.post(`/sessions/${session.id}/input`, {text: 'clear; printf "Checkout recovery review\\nRoom demo: fictional Acme project\\n\\n"', submit: true});
  browser = await chromium.launch({headless: true, args: ['--autoplay-policy=no-user-gesture-required', '--disable-background-timer-throttling', '--disable-renderer-backgrounding']});
  const hostCtx = await browser.newContext({viewport: {width: 1600, height: 900}, deviceScaleFactor: 1, colorScheme: 'dark', storageState: storageState(stack.token, [{name: 'otto_workspace', value: ws.id}, {name: 'otto_firstrun_dismissed', value: '1'}, {name: 'otto_theme', value: 'native'}, {name: 'otto_scheme', value: 'dark'}])});
  hostCtx.setDefaultTimeout(20000);
  const host = await hostCtx.newPage(); await media(host, 'Maya');
  const errors = []; host.on('pageerror', e => errors.push(e.message));
  await host.goto(`${UI}/#/agents/${session.id}`); await host.locator('button[title="More…"]').last().waitFor({timeout: 30000});
  if (!await host.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue('--surface').trim())) throw new Error('Theme has no surface token; refuse unreadable footage.');
  await beat(host, '01-start', 'Start a room from a live session. The host approves who joins and who can type.', async () => {
    await host.locator('button[title="More…"]').last().click(); await host.getByText('Start room…', {exact: true}).click();
    await host.getByLabel('Your display name').fill('Maya'); await click(host, 'Start room'); await click(host, 'Invite someone…');
    await host.getByLabel('Maximum access').selectOption('editor'); await click(host, 'Create invitation');
  });
  const invite = await host.getByLabel('Invitation link').inputValue(); const fragment = new URL(invite).hash;
  await click(host, 'Done');
  const guestCtx = await browser.newContext({viewport: {width: 1600, height: 900}, deviceScaleFactor: 1, colorScheme: 'dark'});
  guestCtx.setDefaultTimeout(20000);
  const guest = await guestCtx.newPage(); await media(guest, 'Alex'); guest.on('pageerror', e => errors.push(e.message));
  await guest.goto(`${UI}/${fragment}`); await guest.getByLabel('Your display name').fill('Alex'); await click(guest, 'Request entry');
  await beat(host, '02-admit', 'Admit your teammate. Chat is shared with the room, separate from the agent.', async () => {
    await click(host, 'Admit can control'); await guest.getByLabel('Message everyone').fill('Can you show the retry state? I can add the regression test.'); await click(guest, 'Send');
    await host.getByLabel('Message everyone').fill('Yes. Let us review it together, then ship after the test passes.'); await click(host, 'Send');
  });
  await beat(host, '03-consent', 'Prepare a local recap. Speech and shared activity are captured only after everyone consents.', async () => {
    await click(host, 'Prepare a full recap…'); await click(host, 'Ask everyone to consent');
    await click(host, 'I consent to capture'); await click(guest, 'I consent to capture'); await click(host, 'Start capture');
    await click(host, 'Join audio'); await click(host, 'Turn microphone on'); await click(guest, 'Join audio');
  });
  await beat(host, '04-screen', 'Share a live screen and talk through the issue. Only shared content enters the recap.', async () => {
    await click(host, 'Share screen…'); await host.evaluate(() => window.playDemoSpeech());
    await guest.getByRole('button', {name: 'Request annotation', exact: true}).click(); await click(host, 'Allow Alex to draw');
  }, 3500);
  await beat(guest, '05-draw', 'Ask permission, highlight the retry action, and draw directly on the shared screen.', async () => {
    await click(guest, 'Highlight'); const box = await guest.locator('.annotation-overlay').first().boundingBox();
    await guest.mouse.move(box.x + box.width * .03, box.y + box.height * .66); await guest.mouse.down(); await guest.mouse.move(box.x + box.width * .31, box.y + box.height * .80, {steps: 18}); await guest.mouse.up();
    await click(guest, 'Draw'); await guest.mouse.move(box.x + box.width * .55, box.y + box.height * .58); await guest.mouse.down(); await guest.mouse.move(box.x + box.width * .38, box.y + box.height * .72, {steps: 20}); await guest.mouse.up();
  });
  await beat(host, '06-presenters', 'Multiple people can present. Pin the screen you want to focus on; your layout stays private.', async () => {
    await click(guest, 'Request to present'); await click(host, 'Allow presentation'); await click(guest, 'Share screen…');
    await host.getByRole('button', {name: 'Pin Alex’s screen', exact: true}).click();
  }, 4000);
  await beat(guest, '07-request-control', 'Request terminal control. Access changes only after the host grants permission.', async () => {
    await click(guest, 'Request control'); await click(host, 'Give control…'); await host.getByRole('dialog').getByRole('button', {name: 'Give control', exact: true}).click();
    await guest.locator('.xterm-helper-textarea').fill('node retry-test.mjs'); await guest.locator('.xterm-helper-textarea').press('Enter');
  });
  await beat(host, '08-take-back', 'The teammate runs the real test in the shared terminal. The host can take back control at any time.', async () => {
    await click(host, 'Take back control'); await click(host, 'Show grid');
    await host.getByLabel('Message everyone').fill('Decision: keep Retry visible. Alex owns the regression. Open question: automatic retry?'); await click(host, 'Send');
  });
  await click(host, 'Finish recap');
  await host.getByText('Recap stopped', {exact: true}).waitFor({timeout: 200000});
  const archives = await api.get('/room-recaps'); const recapId = archives[0].id;
  const detail = await api.get(`/room-recaps/${recapId}?limit=100`);
  const types = new Set(detail.events.map(e => e.payload.type));
  for (const kind of ['speech', 'screen', 'annotation', 'terminal', 'chat']) if (!types.has(kind)) throw new Error(`Actual recap missing ${kind}`);
  writeFileSync(join(out, 'recap-evidence.json'), JSON.stringify(detail, null, 2));
  await click(host, 'Open recap');
  if (await host.getByRole('dialog', {name: 'Room recap', exact: true}).evaluate(node => getComputedStyle(node).backgroundColor === 'rgba(0, 0, 0, 0)')) throw new Error('Recap dialog is transparent; refuse unreadable footage.');
  await beat(host, '09-transcript', 'The full local transcript combines actual recognized speech, chat, and terminal output.', async () => {
    const speechArticle = host.locator('.events article').filter({hasText: 'Let us review'}).first(); await speechArticle.scrollIntoViewIfNeeded();
  }, 5000);
  await beat(host, '10-evidence', 'Review saved screen samples and annotations with the session timeline. Samples are not a continuous video recording.', async () => {
    await click(host, 'Activity & screens'); await host.getByRole('button', {name: 'Show screen sample', exact: true}).first().click(); await host.locator('.events article').filter({has: host.locator('img')}).first().scrollIntoViewIfNeeded();
  }, 4500);
  await click(host, 'Summary'); await click(host, 'Generate summary…'); await host.getByRole('dialog', {name: 'Generate recap summary?', exact: true}).getByRole('button', {name: 'Generate summary', exact: true}).click();
  await host.getByText('Codex draft', {exact: true}).waitFor({timeout: 240000});
  await beat(host, '11-summary', 'Generate a Codex draft using the signed-in subscription: overview, decisions, action items, and open questions.', async () => {
    await host.locator('.draft').scrollIntoViewIfNeeded();
  }, 7000);
  const final = await api.get(`/room-recaps/${recapId}?limit=100`); if (!final.draft?.decisions.length) throw new Error('Actual Codex draft has no decisions');
  writeFileSync(join(out, 'recap-evidence.json'), JSON.stringify(final, null, 2));
  writeFileSync(join(out, 'provenance.json'), JSON.stringify({capturedAt: new Date().toISOString(), ui: 'production ui/dist', daemon: 'real isolated ottod', speech: 'macOS Samantha synthetic input, production whisper.cpp recognition', screens: 'fictional Acme canvas captureStreams carried by production WebRTC', summary: 'actual production Codex summary via subscription sign-in', speechText: speech, eventTypes: [...types], runtimeErrors: errors, limits: ['Browser room capture; native host-window acceptance is separate.', 'Terminal takeover, not operating-system remote control.', 'No webcam video feature is implied.', 'One screen source per presenter.']}, null, 2));
  console.log('[rooms] completed all real workflow shots');
} catch (error) {
  if (browser) for (const context of browser.contexts()) for (const [i, page] of context.pages().entries()) {
    await page.screenshot({path: join(cache, `room-failure-${context === browser.contexts()[0] ? 'host' : 'guest'}-${i}.png`)}).catch(() => {});
    console.error((await page.locator('body').innerText().catch(() => '')).slice(-4500));
  }
  throw error;
} finally { await browser?.close(); await stack?.stop(); }
