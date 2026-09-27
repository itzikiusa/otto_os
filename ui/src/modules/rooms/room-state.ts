/** Pure room helpers: no owner auth, storage, or native commands. */
export interface RoomInvitation { origin: string; roomId: string; invite: string; url: string }
export function parseRoomInvite(value: string): RoomInvitation {
  const url = new URL(value.trim());
  const loopback = ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname);
  if ((url.protocol !== 'https:' && !(url.protocol === 'http:' && loopback)) || url.username || url.password || url.search || url.pathname !== '/' || url.hostname === 'tauri.localhost') throw new Error('Use an HTTPS room invitation (HTTP is supported only on this computer).');
  const match = /^#\/room\/([A-Za-z0-9_-]+)\/([A-Za-z0-9_-]+)$/.exec(url.hash);
  if (!match || match[1].length > 128 || match[2].length < 16 || match[2].length > 512) throw new Error('Paste the complete room invitation, including its invitation code.');
  return { origin: url.origin, roomId: match[1], invite: match[2], url: url.href };
}
export function roomSocketAddress(origin: string, roomId: string, terminal = false): string {
  const url = new URL(`/ws/rooms/${encodeURIComponent(roomId)}${terminal ? '/terminal' : ''}`, origin);
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
  return url.href;
}
export function reconcilePin(pin: string | null, sources: readonly string[]): string | null {
  return pin && sources.includes(pin) ? pin : null;
}
export function normalizedPoint(x: number, y: number, box: {left: number; top: number; width: number; height: number}, sourceWidth: number, sourceHeight: number): {x: number; y: number} | null {
  if (sourceWidth <= 0 || sourceHeight <= 0 || box.width <= 0 || box.height <= 0) return null;
  const scale = Math.min(box.width / sourceWidth, box.height / sourceHeight);
  const width = sourceWidth * scale, height = sourceHeight * scale;
  const px = (x - box.left - (box.width - width) / 2) / width;
  const py = (y - box.top - (box.height - height) / 2) / height;
  return px >= 0 && px <= 1 && py >= 0 && py <= 1 ? {x: px, y: py} : null;
}
/** Apply vectors only to their current source, clear operation and permission grant. */
export function applyRoomEvent(room: import('../../lib/api/room-types').RoomSnapshot, event: import('../../lib/api/room-types').RoomEvent): import('../../lib/api/room-types').RoomSnapshot {
  if (event.type === 'snapshot') return event.room.room_id === room.room_id ? event.room : room;
  if (event.type === 'annotation') {
    const mark = event.annotation;
    const source = room.presentations?.find(s => s.id === mark.source_id);
    const grant = room.annotation_grants?.find(g => g.source_id === mark.source_id && g.member_id === mark.member_id);
    if (!source || source.generation !== mark.source_generation || source.clear_epoch !== mark.clear_epoch || !grant?.allowed || grant.blocked || grant.epoch !== mark.grant_epoch || room.annotations_enabled === false) return room;
    return {...room, annotations: [...(room.annotations ?? []).filter(a => a.id !== mark.id && !(mark.tool === 'pointer' && a.tool === 'pointer' && a.source_id === mark.source_id && a.member_id === mark.member_id)), mark].slice(-400)};
  }
  if (event.type === 'clear_annotations') return {...room, presentations: room.presentations?.map(s => s.id === event.source_id && event.clear_epoch > s.clear_epoch ? {...s, clear_epoch: event.clear_epoch} : s), annotations: room.annotations?.filter(a => a.source_id !== event.source_id || a.clear_epoch >= event.clear_epoch)};
  if (event.type === 'annotation_removed') return {...room, annotations: room.annotations?.filter(a => !(a.source_id === event.source_id && a.id === event.annotation_id))};
  return room;
}
