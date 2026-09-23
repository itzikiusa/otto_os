// History — every past Claude/Codex conversation, read-only + resumable
// (docs/design/conversation-view.md §5.3). Mounted by App.svelte at `#/history`;
// the sidebar row lives in lib/sidebar.ts, and its ⌘K "Go to History" command
// is derived from that registry (shell/App.svelte), like every other module's.
export { default as HistoryPage } from './HistoryPage.svelte';
export { history, entryKey, entryTitle, shortCwd } from './history.svelte';
export type { HistoryGroup, DateWindow, ProviderFilter, StatusFilter } from './history.svelte';
