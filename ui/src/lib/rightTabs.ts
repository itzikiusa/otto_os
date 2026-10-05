// The Session panel's tabs (the agent-mode right panel, ⌘J) — one list for the
// panel's tab strip / icon rail (shell/RightPanel.svelte) and the ⌘K
// "Open <tab> panel" commands (shell/App.svelte), so the two never drift.
import type { IconName } from './components/Icon.svelte';
import type { RightTab } from './stores/ui.svelte';

/** The panel's one name: toggle button, drawer, tablist and ⌘K all use it. */
export const SESSION_PANEL = 'Session panel';

export const RIGHT_TABS: readonly { id: RightTab; icon: IconName; label: string }[] = [
  { id: 'git', icon: 'branch', label: 'Git' },
  { id: 'files', icon: 'file', label: 'Files' },
  { id: 'notes', icon: 'note', label: 'Notes' },
  { id: 'activity', icon: 'zap', label: 'Activity' },
  // Outputs — artifacts the focused agent produced, with sandboxed previews
  // (docs/design/conversation-view.md §5.6). Gated like the rest on an
  // active agent session by the shell.
  { id: 'outputs', icon: 'layers', label: 'Outputs' },
  { id: 'canvas', icon: 'shapes', label: 'Canvas' },
  { id: 'info', icon: 'info', label: 'Info' },
  { id: 'browser', icon: 'globe', label: 'Browser' },
  { id: 'api', icon: 'send', label: 'API' },
];
