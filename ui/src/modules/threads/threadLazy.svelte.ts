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
export const FolderPickerModal = lazyComponent(() => import('../../lib/components/FolderPicker.svelte'));
export const MentionMenuView = lazyComponent(() => import('./runtime/RuntimeMentionMenu.svelte'));
export const StartRoomView = lazyComponent(() => import('../rooms/StartRoomModal.svelte')); // ui-guards: allow — shared session room modal
export const TerminalDrawerView = lazyComponent(() => import('./ThreadTerminalDrawer.svelte'));
export const StageSlashView = lazyComponent(() => import('./StageSlashPopup.svelte'));
export const DetailsCardView = lazyComponent(() => import('./ThreadDetailsCard.svelte'));
export const RelatedListView = lazyComponent(() => import('./RelatedThreadsList.svelte'));
export const ThreadSheetsView = lazyComponent(() => import('./ThreadSheets.svelte'));
export const IdeProjectsSectionView = lazyComponent(() => import('./ThreadIdeProjectsSection.svelte'));
export const OriginGroupsView = lazyComponent(() => import('./ThreadOriginGroups.svelte'));
