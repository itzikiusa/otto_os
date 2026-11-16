// Headless Threads runtime session port registered at shell boot for hosts
// (`modules/ide`) that drive structured runtime threads through
// `ui/src/lib/runtimeThreadPort.ts` without cross-module or Svelte UI imports.

import type { RuntimeSessionPort } from '../../../lib/runtimeThreadPort';
import { threads } from '../threadsStore.svelte';
import { readAttachments } from './runtimeComposer';
import { runtimeThreads } from './runtimeThreads.svelte';
import { runtimeViews } from './runtimeView.svelte';

export const ideRuntimeSessionPort: RuntimeSessionPort = {
  runtimeThreads,
  runtimeViews,
  threads,
  readAttachments,
  loadComposerUi: async () => (await import('./ideRuntimeComposerUi.svelte')).ideRuntimeComposerUi,
};

export const ideRuntimeBridge: RuntimeSessionPort = ideRuntimeSessionPort;
