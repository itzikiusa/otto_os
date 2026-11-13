import type { Session } from '../../lib/api/types';
import type { MenuItem } from '../../lib/contextmenu.svelte';
import { canSwitchSessionSurface, openSessionSurface } from '../../lib/sessionSurface';
import { phoneHandoff } from '../../lib/stores/phoneHandoff.svelte';
import { sessionRoute } from '../../lib/stores/sessionBuckets';
import { ws } from '../../lib/stores/workspace.svelte';

export function sessionHandoffItems(session: Session | null): MenuItem[] {
  if (!session) return [];
  return [
    ...(canSwitchSessionSurface(session) ? [{ label: 'Open in Threads', icon: 'comment', title: 'Same agent and conversation in a chat view', action: () => void openSessionSurface(session, 'threads') }] : []),
    // Where the session lives (a delegated Threads worker opens as a thread);
    // a terminal link on a light-version phone gets the gate's one-visit way in.
    ...phoneHandoff.menuItem(sessionRoute(session, session.id, ws.sessionById), 'this session'),
  ];
}
