// The shell's on-demand overlays and dialogs, each ONE module-level
// `lazyComponent` (lib/lazy-component.svelte.ts) rendered through
// `<LazyMount quiet>`. A bare `{#await import()}` showed nothing while the
// chunk loaded and, when it failed (offline, a stale chunk after an update),
// stayed a silent no-op with the open flag stuck true — so "New session" and
// ⌘N did nothing until a reload. LazyMount toasts the failure, and the hosts
// pass `onerror` to reset their open flag so the next open retries.
import { lazyComponent } from '../lib/lazy-component.svelte';

export const SharePageChunk = lazyComponent(() => import('../modules/share/SharePage.svelte'));
export const HandoverChunk = lazyComponent(() => import('../modules/agents/Handover.svelte'));
export const AttachIssueChunk = lazyComponent(() => import('../modules/agents/AttachIssue.svelte'));
export const AttachProductStoryChunk = lazyComponent(() => import('../modules/agents/AttachProductStory.svelte'));
export const BroadcastModalChunk = lazyComponent(() => import('../lib/components/BroadcastModal.svelte'));
export const NewSessionChunk = lazyComponent(() => import('../modules/agents/NewSession.svelte'));
export const NewWorkspaceChunk = lazyComponent(() => import('../modules/settings/NewWorkspace.svelte'));
/** Shared by the Navigator and the TabBar; it bundles the QR encoder. */
export const ShareModalChunk = lazyComponent(() => import('../modules/agents/ShareModal.svelte'));
