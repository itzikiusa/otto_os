// InlineDiff loads on first use: it is only shown for an edit, and keeping it
// out of the Agents page chunk holds that page under its byte budget.
import { lazyComponent } from '../../../lib/lazy-component.svelte';

export const inlineDiff = lazyComponent(() => import('./InlineDiff.svelte'));
