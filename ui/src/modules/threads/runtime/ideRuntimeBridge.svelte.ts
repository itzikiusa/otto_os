// Lazy Threads runtime & composer widget bridge registered at shell boot for
// hosts (`modules/ide`) that drive structured runtime threads through
// `ui/src/lib/runtimeThreadPort.ts` without cross-module imports.

import type { IdeRuntimeBridge } from '../../../lib/runtimeThreadPort';
import StageSlashPopup from '../StageSlashPopup.svelte';
import { threads } from '../threadsStore.svelte';
import { askTargets } from './askTargets.svelte';
import CouncilStagePicker from './CouncilStagePicker.svelte';
import { council, councilAgentLabel } from './councilStore.svelte';
import RuntimeAccessChip from './RuntimeAccessChip.svelte';
import { readAttachments } from './runtimeComposer';
import RuntimeComposerImages from './RuntimeComposerImages.svelte';
import RuntimeMentionMenu from './RuntimeMentionMenu.svelte';
import { runtimeThreads } from './runtimeThreads.svelte';
import { runtimeViews } from './runtimeView.svelte';

export const ideRuntimeBridge: IdeRuntimeBridge = {
  runtimeThreads,
  runtimeViews,
  council,
  councilAgentLabel,
  askTargets,
  threads,
  readAttachments,
  CouncilStagePicker,
  RuntimeAccessChip,
  RuntimeComposerImages,
  RuntimeMentionMenu,
  StageSlashPopup,
};
