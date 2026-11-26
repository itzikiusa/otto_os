// Workflows' first paint is the Runs inbox (or the Workflows list): the run
// page, the editor, Optimize, the start form and New workflow each load on
// first use (ux.md §14.4).
import { lazyComponent } from '../../lib/lazy-component.svelte';

export const RunPageLazy = lazyComponent(() => import('./runs/RunPage.svelte'));
export const EditorLazy = lazyComponent(() => import('./editor/EditorPage.svelte'));
export const OptimizeLazy = lazyComponent(() => import('./optimize/OptimizePage.svelte'));
export const StartRunLazy = lazyComponent(() => import('./runs/StartRunModal.svelte'));
export const NewWorkflowLazy = lazyComponent(() => import('./list/NewWorkflowModal.svelte'));
