#!/usr/bin/env node
// Independent W3C perfect-negotiation control; no RoomMediaClient import.
import {createRequire} from 'node:module';
import {createServer} from 'node:http';
import {mkdtemp, writeFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {startLoopbackStun} from './loopback-stun.mjs';
const stun = process.argv.includes('--stun') ? await startLoopbackStun() : null;
const require = createRequire(new URL('../../ui/package.json', import.meta.url));
const {chromium} = require('@playwright/test');
const output = await mkdtemp(join(tmpdir(), 'otto-rtc-control-'));
const server = createServer((_, response) => response.end('<!doctype html><title>Synthetic RTC control</title>'));
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const fakeCapture = process.argv.includes('--fake-capture');
const taskSignaling = process.argv.includes('--task-signaling');
const permission = process.argv.includes('--permission');
const browser = await chromium.launch({headless: true, args: fakeCapture ? ['--use-fake-device-for-media-stream', '--use-fake-ui-for-media-stream'] : []});
const reports = [];
try {
  for (const collision of [false, true, true, true, true, true]) {
    const page = await browser.newPage(permission ? {permissions: ['microphone']} : {});
    await page.goto(`http://127.0.0.1:${server.address().port}`);
    const report = await page.evaluate(async ({collision, iceServers, fakeCapture, taskSignaling}) => {
      const trace = [], owned = [], contexts = [], timers = [], peers = [];
      const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
      const log = detail => trace.push({at: Math.round(performance.now()), ...detail});
      const ends = [0, 1].map(index => {
        const pc = new RTCPeerConnection({iceServers}); peers.push(pc);
        return {index, pc, polite: index === 0, makingOffer: false, ignoreOffer: false, settingAnswer: false, candidates: 0};
      });
      const receive = async (end, message) => {
        const pc = end.pc;
        try {
          if (message.description) {
            const description = message.description;
            const ready = !end.makingOffer && (pc.signalingState === 'stable' || end.settingAnswer);
            const collided = description.type === 'offer' && !ready;
            end.ignoreOffer = !end.polite && collided;
            log({peer: end.index, event: 'description', type: description.type, collided, ignored: end.ignoreOffer});
            if (end.ignoreOffer) return;
            end.settingAnswer = description.type === 'answer';
            await pc.setRemoteDescription(description); end.settingAnswer = false;
            if (description.type === 'offer') { await pc.setLocalDescription(); send(end, {description: pc.localDescription.toJSON()}); }
          } else if ('candidate' in message) {
            try { await pc.addIceCandidate(message.candidate); }
            catch (error) { if (!end.ignoreOffer) throw error; }
          }
        } catch (error) { log({peer: end.index, event: 'error', error: String(error)}); }
      };
      const send = (end, message) => { const deliver = () => { void receive(ends[1 - end.index], message); }; if (taskSignaling) setTimeout(deliver, 0); else queueMicrotask(deliver); };
      for (const end of ends) {
        const pc = end.pc;
        pc.onicecandidate = ({candidate}) => { if (candidate) end.candidates++; log({peer: end.index, event: 'candidate', ufrag: candidate?.usernameFragment ?? null}); send(end, {candidate: candidate?.toJSON() ?? null}); };
        pc.onnegotiationneeded = async () => {
          try { end.makingOffer = true; await pc.setLocalDescription(); send(end, {description: pc.localDescription.toJSON()}); }
          catch (error) { log({peer: end.index, event: 'offer-error', error: String(error)}); }
          finally { end.makingOffer = false; }
        };
        pc.onicegatheringstatechange = () => log({peer: end.index, event: 'gathering', state: pc.iceGatheringState, ufrags: pc.localDescription?.sdp.match(/^a=ice-ufrag:.+$/gm)});
        pc.onconnectionstatechange = () => log({peer: end.index, event: 'connection', state: pc.connectionState});
      }
      const addMedia = end => {
        const context = new AudioContext(); contexts.push(context);
        const oscillator = context.createOscillator(), destination = context.createMediaStreamDestination(); oscillator.connect(destination); oscillator.start();
        const canvas = document.createElement('canvas'); canvas.width = 64; canvas.height = 64;
        const paint = canvas.getContext('2d'); let tick = 0;
        timers.push(setInterval(() => { paint.fillStyle = `hsl(${tick++ % 360} 50% 50%)`; paint.fillRect(0, 0, 64, 64); }, 100));
        const stream = canvas.captureStream(3);
        for (const track of [...destination.stream.getTracks(), ...stream.getTracks()]) { owned.push(track); end.pc.addTrack(track, new MediaStream([track])); }
      };
      try {
        if (fakeCapture) {
          const stream = await navigator.mediaDevices.getUserMedia({audio: true});
          owned.push(...stream.getTracks());
        }
        addMedia(ends[0]);
        if (!collision) {
          const until = performance.now() + 12000;
          while (performance.now() < until && ends.some(end => end.pc.connectionState !== 'connected')) await sleep(100);
        }
        addMedia(ends[1]);
        const until = performance.now() + 20000;
        while (performance.now() < until && ends.some(end => end.pc.connectionState !== 'connected')) await sleep(100);
        await sleep(1000);
        return {collision, iceServers, fakeCapture, taskSignaling, connected: ends.every(end => end.pc.connectionState === 'connected'), endpoints: ends.map(end => ({peer: end.index, state: end.pc.connectionState, gathering: end.pc.iceGatheringState, candidates: end.candidates})), trace};
      } finally {
        timers.forEach(clearInterval); owned.forEach(track => track.stop()); peers.forEach(peer => peer.close()); await Promise.all(contexts.map(context => context.close()));
      }
    }, {collision, iceServers: stun ? [{urls: stun.url}] : [], fakeCapture, taskSignaling});
    report.microphonePermissionGranted = permission;
    report.browserVersion = browser.version();
    reports.push(report);
    console.log(JSON.stringify({...report, trace: undefined}));
    await page.close();
  }
} finally {
  await browser.close(); await stun?.close(); await new Promise(resolve => server.close(resolve));
  await writeFile(join(output, 'report.json'), JSON.stringify(reports, null, 2));
  console.log(`Artifacts: ${output}`);
}
