#!/usr/bin/env node
// Four production RoomMediaClient objects, generated pixels/audio, no daemon.
import {createRequire} from 'node:module';
import {mkdtemp, writeFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join, dirname, resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {execFileSync} from 'node:child_process';
import {randomBytes} from 'node:crypto';
import {createServer as createHttpServer} from 'node:http';
import {startLoopbackStun} from './loopback-stun.mjs';

const repo = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const require = createRequire(join(repo, 'ui/package.json'));
const {createServer} = await import(require.resolve('vite'));
const {chromium} = require('@playwright/test');
const args = process.argv.slice(2);
const number = (key, fallback) => args.includes(key) ? Number(args[args.indexOf(key) + 1]) : fallback;
const seconds = number('--seconds', 600), cycles = number('--cycles', 20);
const relay = args.includes('--relay');
const fakeCapture = args.includes('--fake-capture');
if (!Number.isInteger(seconds) || seconds < 5 || seconds > 900 || !Number.isInteger(cycles) || cycles < 1 || cycles > 30) throw new Error('Use --seconds 5..900 and --cycles 1..30');
const output = await mkdtemp(join(tmpdir(), 'otto-room-stability-'));
console.log(`Artifacts: ${output}`);
const samples = [], errors = [];
const container = `otto-room-turn-${process.pid}-${randomBytes(3).toString('hex')}`;
// Official coturn 4.18.0-r0 manifest, pinned to the verified download.
const image = 'coturn/coturn@sha256:bbefd3e1fdfdc0d58770fe01b581fd8b00d9f3a5580d00acb77cf719a6bc78e3';
let server, httpServer, browser, page, stun, containerAttempted = false, imageId;
let interrupted = false;
const interrupt = () => { interrupted = true; void browser?.close().catch(() => {}); };
process.on('SIGINT', interrupt);
process.on('SIGTERM', interrupt);
const deadline = setTimeout(() => { errors.push('Overall fixture deadline exceeded'); void browser?.close(); }, (seconds + 90) * 1000);
const docker = (...params) => execFileSync('docker', params, {encoding: 'utf8', timeout: 30000, stdio: ['ignore', 'pipe', 'pipe']});
try {
  const options = {taskSignaling: args.includes('--task-signaling')};
  if (args.includes('--stun')) { stun = await startLoopbackStun(); options.iceServers = [{urls: [stun.url]}]; }
  if (relay) {
    // Image must already exist: dependency download is an explicit separate step.
    imageId = docker('image', 'inspect', image, '--format', '{{.Id}}').trim();
    const port = 54000 + process.pid % 1000;
    const min = port + 1000, max = min + 31;
    const password = randomBytes(18).toString('hex');
    containerAttempted = true;
    docker('run', '--detach', '--name', container, '--memory', '256m', '--cpus', '1',
      '-p', `127.0.0.1:${port}:3478/tcp`, '-p', `127.0.0.1:${port}:3478/udp`,
      '-p', `127.0.0.1:${min}-${max}:${min}-${max}/udp`, imageId,
      '-n', '--log-file=stdout', '--verbose', '--no-tls', '--fingerprint', '--lt-cred-mech',
      '--realm=otto-room-fixture', `--user=probe:${password}`, '--listening-ip=0.0.0.0', '--relay-ip=127.0.0.1',
      '--external-ip=127.0.0.1', `--min-port=${min}`, `--max-port=${max}`, '--allow-loopback-peers', '--no-multicast-peers');
    options.iceServers = [{urls: [`turn:127.0.0.1:${port}?transport=tcp`], username: 'probe', credential: password}];
    options.relayOnly = true;
    await new Promise(resolve => setTimeout(resolve, 1000));
  }
  // Middleware mode leaves process signals and HTTP listener cleanup with this
  // runner; Vite's standalone server installs a SIGTERM process.exit handler.
  server = await createServer({root: join(repo, 'ui'), server: {middlewareMode: true, hmr: false}, logLevel: 'error'});
  httpServer = createHttpServer(server.middlewares);
  await new Promise((resolve, reject) => { httpServer.once('error', reject); httpServer.listen(0, '127.0.0.1', resolve); });
  const origin = `http://127.0.0.1:${httpServer.address().port}`;
  browser = await chromium.launch({headless: true, handleSIGINT: false, handleSIGTERM: false, handleSIGHUP: false, args: fakeCapture ? ['--use-fake-device-for-media-stream', '--use-fake-ui-for-media-stream'] : []});
  page = await browser.newPage();
  page.on('pageerror', error => errors.push(error.message));
  await page.route(`${origin}/`, route => route.fulfill({contentType: 'text/html', body: '<!doctype html><title>Synthetic room stability fixture</title>'}));
  await page.goto(origin);
  if (fakeCapture) await page.evaluate(async () => {
    // Browser process is explicitly fake-device-only. Keep its generated capture
    // active to reproduce the ICE gathering conditions of a voice participant.
    window.fixtureCapture = await navigator.mediaDevices.getUserMedia({audio: true});
  });
  await page.evaluate(async options => {
    const {startMediaSmoke} = await import('/e2e/fixtures/room-media-smoke.ts');
    window.mediaSmoke = await startMediaSmoke(options);
  }, options);
  const count = async expected => page.waitForFunction(expected => {
    const received = Object.values(window.mediaSmoke.summary().received);
    return received.length === 4 && received.every(sources => sources.length === expected);
  }, expected, {timeout: 20000});
  await count(4);
  const connected = async () => {
    // Playwright's installed waitForFunction treats a returned Promise as
    // truthy even when it resolves false. Await getStats explicitly in Node.
    const until = Date.now() + 20000; let readySince = 0;
    while (Date.now() < until) {
      const {peers} = await page.evaluate(() => window.mediaSmoke.statistics());
      const active = peers.filter(peer => peer.state !== 'closed');
      const ready = active.length === 6 && active.every(peer => peer.state === 'connected' && peer.signaling === 'stable' && (!relay || (peer.selectedType === 'relay' && peer.policy === 'relay')));
      if (!ready) readySince = 0;
      else { readySince ||= Date.now(); if (Date.now() - readySince >= 2000) return; }
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    throw new Error('Six production peers did not reach stable connected signaling');
  };
  await connected();
  let start = Date.now();
  const sample = async cycle => {
    const result = await page.evaluate(async () => ({...window.mediaSmoke.summary(), statistics: await window.mediaSmoke.statistics()}));
    result.statistics.retiredPeerCount = result.statistics.peers.filter(peer => peer.state === 'closed').length;
    result.statistics.peers = result.statistics.peers.filter(peer => peer.state !== 'closed');
    if (Object.values(result.received).some(sources => sources.length !== 4)) throw new Error('A client lost a presentation');
    if (Object.values(result.states).some(state => !state.audioJoined || state.muted)) throw new Error('A synthetic voice participant is absent or muted');
    if (Object.values(result.states).some(state => state.error)) throw new Error(`Production media error: ${JSON.stringify(result.states)}`);
    if (result.statistics.peers.length !== 6 || result.statistics.peers.some(peer => peer.state !== 'connected')) throw new Error('Production peer disconnected');
    if (relay && result.statistics.peers.some(peer => peer.selectedType !== 'relay' || peer.policy !== 'relay')) throw new Error('Forced-relay fixture selected a non-relay path');
    const previous = samples.at(-1)?.statistics.peers;
    if (result.statistics.peers.some(peer => peer.audioBytesReceived <= 0 || peer.audioPacketsReceived <= 0)) throw new Error('A production peer received no audio RTP');
    if (previous && result.statistics.peers.some((peer, index) => peer.bytesReceived <= previous[index].bytesReceived || peer.framesDecoded <= previous[index].framesDecoded || peer.audioBytesReceived <= previous[index].audioBytesReceived || peer.audioPacketsReceived <= previous[index].audioPacketsReceived)) throw new Error('Media stopped advancing between samples');
    samples.push({elapsedMs: Date.now() - start, cycle, ...result});
    console.log(JSON.stringify({event: 'sample', elapsedSeconds: Math.round((Date.now() - start) / 1000), cycle, sources: result.sources, peers: result.statistics.peers.length, relay}));
  };
  await new Promise(resolve => setTimeout(resolve, 1500));
  await connected();
  start = Date.now();
  await sample(0);
  for (let cycle = 1; cycle <= cycles; cycle++) {
    await page.evaluate(() => window.mediaSmoke.stopPresenting('guest3'));
    await count(3);
    await page.evaluate(() => window.mediaSmoke.startPresenting('guest3'));
    await count(4);
    const next = start + seconds * 1000 * cycle / cycles;
    while (Date.now() < next) {
      if (interrupted) throw new Error('Fixture interrupted');
      await new Promise(resolve => setTimeout(resolve, Math.min(1000, next - Date.now())));
    }
    await sample(cycle);
  }
  const first = samples[0].statistics.peers, last = samples.at(-1).statistics.peers;
  if (last.some((peer, index) => peer.bytesReceived <= first[index].bytesReceived || peer.framesDecoded <= first[index].framesDecoded)) throw new Error('Decoded media did not advance on every peer');
  await page.evaluate(() => { window.fixtureCapture?.getTracks().forEach(track => track.stop()); return window.mediaSmoke.stop(); });
  const cleanup = await page.evaluate(async () => ({...await window.mediaSmoke.statistics(), fixtureTracks: window.fixtureCapture?.getTracks().map(track => track.readyState) ?? []}));
  if (cleanup.peers.some(peer => peer.state !== 'closed') || cleanup.tracks.some(state => state !== 'ended') || cleanup.contexts.some(state => state !== 'closed') || cleanup.fixtureTracks.some(state => state !== 'ended')) throw new Error('Owned synthetic media did not clean up');
  await writeFile(join(output, 'cleanup.json'), JSON.stringify(cleanup, null, 2));
} catch (error) {
  errors.push(String(error.stack ?? error));
  if (page && !page.isClosed()) {
    const failure = await page.evaluate(async () => window.mediaSmoke ? ({...window.mediaSmoke.summary(), statistics: await window.mediaSmoke.statistics()}) : null).catch(() => null);
    await writeFile(join(output, 'failure.json'), JSON.stringify(failure, null, 2));
  }
} finally {
  clearTimeout(deadline);
  if (page && !page.isClosed()) await page.evaluate(() => { window.fixtureCapture?.getTracks().forEach(track => track.stop()); return window.mediaSmoke?.stop(); }).catch(() => {});
  if (httpServer) await new Promise(resolve => httpServer.close(error => { if (error) errors.push(`HTTP cleanup: ${error.message}`); resolve(); }));
  for (const [label, resource] of [['browser', browser], ['Vite', server], ['STUN', stun]]) {
    try { await resource?.close(); }
    catch (error) { errors.push(`${label} cleanup: ${error.message}`); }
  }
  if (containerAttempted) {
    try { await writeFile(join(output, 'turn.log'), docker('logs', container)); }
    catch (error) { errors.push(`TURN logs: ${error.message}`); }
    finally {
      try { docker('rm', '--force', container); }
      catch (error) { errors.push(`TURN cleanup: ${error.message}`); }
    }
  }
  await writeFile(join(output, 'report.json'), JSON.stringify({seconds, cycles, relay, fakeCapture, taskSignaling: args.includes('--task-signaling'), loopbackStun: !!stun, image: relay ? image : null, imageId, samples, errors,
    note: 'One Mac, Chromium, synthetic capture and local signaling. Not physical capture, production server signaling, cross-network NAT, or per-Mac performance acceptance.'}, null, 2));
}
process.removeListener('SIGINT', interrupt);
process.removeListener('SIGTERM', interrupt);
console.log(JSON.stringify({event: 'done', passed: errors.length === 0, samples: samples.length, errors, output}));
if (errors.length) process.exitCode = 1;
