/** Game invitations live only in window memory, never owner credentials/storage
 *  (the store itself is `lib/rooms/invites.ts`, which the router captures into). */
export { captureGameInvite, consumeGameInvite } from '../../../lib/rooms/invites.ts';
