import type { Session } from '../../lib/api/types';
import type { MenuItem } from '../../lib/contextmenu.svelte';
import { canSwitchSessionSurface, openSessionSurface } from '../../lib/sessionSurface';
import { phoneHandoff } from '../../lib/stores/phoneHandoff.svelte';

export function sessionHandoffItems(session: Session | null): MenuItem[] {
  if (!session) return [];
  return [
    ...(canSwitchSessionSurface(session) ? [{ label: 'Open in Threads', icon: 'comment', title: 'Same agent and conversation in a chat view', action: () => void openSessionSurface(session, 'threads') }] : []),
    ...phoneHandoff.menuItem(`agents/${session.id}`, 'this session'),
  ];
}
