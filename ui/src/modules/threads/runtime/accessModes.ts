// A runtime thread's access modes (labels, icons, descriptions — ported from
// T3 Code, MIT, Copyright (c) 2026 T3 Tools Inc.:
// apps/web/src/components/chat/runtimeModeConfig.ts), the one-time Full
// access confirm, and the menu rows shared by the composer's access chip
// and its "+" menu.
import type { IconName } from '../../../lib/components/Icon.svelte';
import { confirmer } from '../../../lib/confirm.svelte';
import type { MenuItem } from '../../../lib/contextmenu.svelte';
import type { RuntimeAccessMode } from '../../../lib/api/types';

export const ACCESS: Record<RuntimeAccessMode, { label: string; description: string; icon: IconName }> = {
  'approval-required': { label: 'Supervised', description: 'Ask before commands and file changes.', icon: 'lock' },
  'auto-accept-edits': { label: 'Auto-accept edits', description: 'Auto-approve edits, ask before other actions.', icon: 'edit' },
  'full-access': { label: 'Full access', description: 'Allow commands and edits without prompts.', icon: 'shieldOff' },
};
const FULL_OK_KEY = 'otto_runtime_full_access_ok';

/** Full access asks once (who acts, where); true = go ahead. */
async function confirmAccess(m: RuntimeAccessMode, agent: string, cwd: string): Promise<boolean> {
  if (m !== 'full-access') return true;
  try {
    if (localStorage.getItem(FULL_OK_KEY) === '1') return true;
  } catch {
    /* no storage: ask */
  }
  const ok = await confirmer.ask(
    `${agent} will run commands and change files in ${cwd} without asking first, from the next message on. The macOS sandbox still applies.`,
    { title: 'Allow full access?', confirmLabel: 'Allow full access', danger: true },
  );
  if (ok) {
    try {
      localStorage.setItem(FULL_OK_KEY, '1');
    } catch {
      /* asks again next time */
    }
  }
  return ok;
}

/** One checkable row per mode; picking Full access confirms first. */
export function accessMenu(mode: RuntimeAccessMode, agent: string, cwd: string, onpick: (m: RuntimeAccessMode) => void): MenuItem[] {
  return (Object.keys(ACCESS) as RuntimeAccessMode[]).map((m) => ({
    label: ACCESS[m].label,
    icon: ACCESS[m].icon,
    checked: m === mode,
    title: ACCESS[m].description,
    action: () => void confirmAccess(m, agent, cwd).then((ok) => ok && onpick(m)),
  }));
}
