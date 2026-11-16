// The composers' emoji helper (EmojiAssist.svelte) as its own chunk: it loads
// as a composer mounts, so it never weighs on a page's first paint.
import { lazyComponent } from '../lazy-component.svelte';

export const EmojiAssistView = lazyComponent(() => import('./EmojiAssist.svelte'));
