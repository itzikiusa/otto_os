// Review comments on a design: the stage's pin layer, the details panel's
// Comments tab and the per-design store they share (see commentsStore.svelte.ts).
export { default as CommentsTab } from './CommentsTab.svelte';
export { default as PinHost } from './PinHost.svelte';
export { commentsFor, CommentsStore } from './commentsStore.svelte';
export { openCount } from './pins';
