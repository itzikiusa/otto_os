import {mount, unmount} from 'svelte';
import RoomRecapsPage from '../../src/modules/rooms/RoomRecapsPage.svelte';
import ConfirmDialog from '../../src/lib/components/ConfirmDialog.svelte';
import {RoomRecapCapture} from '../../src/modules/rooms/recap-capture';
import type {RoomSnapshot} from '../../src/lib/api/room-types';
export function mountRecapPanel() {
  const target = document.createElement('div'); target.style.cssText = 'padding:16px;height:100%;overflow:auto;background:var(--bg);color:var(--text)'; document.body.replaceChildren(target);
  const panel = mount(RoomRecapsPage, {target}); const confirm = mount(ConfirmDialog, {target});
  return () => { void unmount(panel); void unmount(confirm); };
}
export async function startSyntheticCapture(room: RoomSnapshot) {
  const context = new AudioContext(); await context.resume();
  const oscillator = context.createOscillator(); oscillator.frequency.value = 440;
  const destination = context.createMediaStreamDestination(); oscillator.connect(destination); oscillator.start();
  const canvas = document.createElement('canvas'); canvas.width = 640; canvas.height = 360;
  const ctx = canvas.getContext('2d')!; ctx.fillStyle = 'navy'; ctx.fillRect(0, 0, 640, 360); ctx.fillStyle = 'white'; ctx.font = '24px sans-serif'; ctx.fillText('Synthetic recap sample', 24, 60);
  const stream = canvas.captureStream(2);
  let audible = true;
  const capture = new RoomRecapCapture(() => audible ? new Map([['host', {memberId: 'host', generation: 1, stream: destination.stream}]]) : new Map(), () => new Map([['screen', stream]]), () => {});
  capture.update(room, true);
  return {mute() { audible = false; }, unmute() { audible = true; }, update(next: RoomSnapshot) { capture.update(next, true); }, async finish() { await capture.finish(); }, stop() { capture.dispose(); oscillator.stop(); for (const track of stream.getTracks()) track.stop(); void context.close(); }};
}
export async function mountHostRoomPage() {
  const {rememberRoom} = await import('../../src/modules/rooms/room-access');
  const {default: RoomPage} = await import('../../src/modules/rooms/RoomPage.svelte');
  rememberRoom(location.origin, {room_id: 'finalize', member_id: 'host', token: 'host-room-token'});
  const target = document.createElement('div'); target.style.height = '100%'; document.body.replaceChildren(target);
  mount(RoomPage, {target, props: {roomId: 'finalize', guest: true}}); mount(ConfirmDialog, {target});
}
