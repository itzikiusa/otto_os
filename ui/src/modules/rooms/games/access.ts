/** Game invitations live only in window memory, never owner credentials/storage. */
const invites = new Map<string,string>();
export function captureGameInvite(id:string,invite:string):void { invites.set(id,invite); }
export function consumeGameInvite(id:string):string|undefined { const value=invites.get(id); invites.delete(id); return value; }
