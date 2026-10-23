import type { RoomCredential } from '../../lib/api/room-types';
import { captureRoomInvite, dropRoomInvite, roomInvite } from '../../lib/rooms/invites.ts';
/** Window-memory only. Never persist capabilities or send them to owner APIs.
 *  The captured invitations live in `lib/rooms/invites.ts` (the router writes them). */
export { captureRoomInvite, roomInvite };
const credentials = new Map<string, { origin: string; credential: RoomCredential }>();
export function rememberRoom(origin: string, credential: RoomCredential): void {
  credentials.set(credential.room_id, { origin, credential });
  dropRoomInvite(credential.room_id);
}
export function recallRoom(roomId: string) { return credentials.get(roomId); }
export function forgetRoom(roomId: string): void { credentials.delete(roomId); dropRoomInvite(roomId); }
