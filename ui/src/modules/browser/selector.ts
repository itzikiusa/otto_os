// The mark-selector algorithm lives with the browser store
// (lib/stores/browser.svelte.ts § Mark selectors), where the agent UI-control
// handlers can share it without lib/ importing modules/. Re-exported for the
// module's own callers (ReaderView).
export { buildSelector, MAX_SELECTOR_LEN } from '../../lib/stores/browser.svelte';
