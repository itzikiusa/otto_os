// The vault's first paint is the tree and the reader. The graph, the docs
// agent, recovery and the raw-file viewer (yaml, OpenAPI) load when opened.
import { lazyComponent } from '../../lib/lazy-component.svelte';

export const GraphLazy = lazyComponent(() => import('./GraphView.svelte'));
export const DocsAgentsLazy = lazyComponent(() => import('./DocsAgentsView.svelte'));
export const RecoveryLazy = lazyComponent(() => import('./RecoveryView.svelte'));
export const FileViewerLazy = lazyComponent(() => import('./FileViewer.svelte'));
