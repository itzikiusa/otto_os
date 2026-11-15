// Lazy composer UI port for embedded IDE new-thread stage composers (`IdeAgentComposer.svelte`).
// Kept separate from the headless `RuntimeSessionPort` (`ideRuntimeBridge.svelte.ts`) so
// headless turn runners (`IdeAgentEngine`, `⌘I` inline edit) never evaluate composer Svelte UI.

import type { RuntimeComposerUiPort } from '../../../lib/runtimeThreadPort';
import StageSlashPopup from '../StageSlashPopup.svelte';
import { askTargets } from './askTargets.svelte';
import CouncilStagePicker from './CouncilStagePicker.svelte';
import { council, councilAgentLabel } from './councilStore.svelte';
import RuntimeAccessChip from './RuntimeAccessChip.svelte';
import RuntimeComposerImages from './RuntimeComposerImages.svelte';
import RuntimeMentionMenu from './RuntimeMentionMenu.svelte';

export const ideRuntimeComposerUi: RuntimeComposerUiPort = {
  council,
  councilAgentLabel,
  askTargets,
  CouncilStagePicker,
  RuntimeAccessChip,
  RuntimeComposerImages,
  RuntimeMentionMenu,
  StageSlashPopup,
};
