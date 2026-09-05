// History — every past Claude/Codex conversation, read-only + resumable
// (docs/design/conversation-view.md §5.3). Mounted by App.svelte at `#/history`;
// the sidebar row lives in lib/sidebar.ts, and importing this module (App's
// route branch, or AgentsPage at boot) registers the ⌘K command below.
import { registry } from '../../../lib/commands.svelte';
import { router } from '../../../lib/router.svelte';

export { default as HistoryPage } from './HistoryPage.svelte';
export { history, entryKey, entryTitle, shortCwd } from './history.svelte';
export type { HistoryGroup, DateWindow, ProviderFilter, StatusFilter } from './history.svelte';

registry.register('history', [
  {
    id: 'history.go',
    title: 'Go to History',
    group: 'Navigate',
    keywords: 'module history past conversations transcripts sessions claude codex resume chat log',
    run: () => router.go('history'),
  },
]);
