import { mount, unmount } from 'svelte';
import RoomScreens from '../../src/modules/rooms/RoomScreens.svelte';
import type { RoomSnapshot, RoomAction, RoomSubscriptionTier } from '../../src/lib/api/room-types';

/** Synthetic pixels only; no permissions, real screen content or network peers. */
export function mountScreens(initial: RoomSnapshot) {
  const view = $state({room: initial});
  const actions: RoomAction[] = [];
  const subscriptions: {sourceId: string; generation: number; tier: RoomSubscriptionTier}[][] = [];
  const streams = new Map<string, MediaStream>();
  for (const source of initial.presentations ?? []) {
    const canvas = document.createElement('canvas'); canvas.width = 640; canvas.height = 360;
    const ctx = canvas.getContext('2d')!;
    ctx.fillStyle = '#16354a'; ctx.fillRect(0, 0, 640, 360);
    ctx.fillStyle = '#ccecff'; ctx.font = '24px sans-serif'; ctx.fillText(source.title, 24, 60);
    ctx.fillStyle = '#30657b'; ctx.fillRect(24, 100, 380, 18); ctx.fillRect(24, 136, 480, 18); ctx.fillRect(24, 172, 300, 18);
    streams.set(source.id, canvas.captureStream(1));
  }
  const target = document.createElement('div'); target.style.cssText = 'height:100%;display:flex;flex-direction:column;background:var(--bg);color:var(--text);';
  document.body.replaceChildren(target);
  const component = mount(RoomScreens, {target, props: {
    get room() { return view.room; }, streams,
    send: (action: RoomAction) => { actions.push(action); return true; },
    onSubscriptions: (values: {sourceId: string; generation: number; tier: RoomSubscriptionTier}[]) => subscriptions.push(values),
  }});
  return {actions, subscriptions, update(room: RoomSnapshot) { view.room = room; }, destroy() { for (const stream of streams.values()) for (const track of stream.getTracks()) track.stop(); void unmount(component); }};
}
