// The workflows page's first paint is its lists. The canvas and its palette,
// the node inspector, triggers, the run monitor, the run input form and the
// run's files tree each load on first use.
import { lazyComponent } from '../../lib/lazy-component.svelte';

export const CanvasLazy = lazyComponent(() => import('./WorkflowCanvas.svelte'));
export const PaletteLazy = lazyComponent(() => import('./NodePalette.svelte'));
export const ParamsLazy = lazyComponent(() => import('./inspector/NodeParams.svelte'));
export const TriggersLazy = lazyComponent(() => import('./TriggersPanel.svelte'));
export const MonitorLazy = lazyComponent(() => import('./RunMonitor.svelte'));
export const RunInputLazy = lazyComponent(() => import('./RunInputPanel.svelte'));
export const ApprovalsLazy = lazyComponent(() => import('./RunApprovals.svelte'));
export const GuideLazy = lazyComponent(() => import('../../lib/components/AutomateGuide.svelte'));
export const ContextLazy = lazyComponent(() => import('./RunContextSidebar.svelte'));
