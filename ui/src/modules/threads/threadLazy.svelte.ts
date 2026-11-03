// Lazy chunks for everything Threads opens on demand (sheets, the PR view,
// right-panel surfaces): one loader each, shared by the host that renders it
// and the buttons that prefetch it on hover — so opening is instant.
import { lazyComponent } from '../../lib/lazy-component.svelte';

export const SettingsView = lazyComponent(() => import('../settings/Settings.svelte')); // ui-guards: allow — shared Settings page as a sheet
export const UsageView = lazyComponent(() => import('../usage/UsagePage.svelte')); // ui-guards: allow — shared Usage page as a sheet
export const PullRequestsView = lazyComponent(() => import('./ThreadPullRequests.svelte'));
export const SideChatView = lazyComponent(() => import('./ThreadSideChat.svelte'));
export const PrSurfaceView = lazyComponent(() => import('./ThreadPrSurface.svelte'));
export const TerminalDock = lazyComponent(() => import('./ThreadTerminalDock.svelte'));
export const FilterPopover = lazyComponent(() => import('./ThreadProjectFilter.svelte'));
