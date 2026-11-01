import type { TranscriptUnavailableReason } from '../../../lib/api/types';
import type { MenuItem } from '../../../lib/contextmenu.svelte';
import { transcript } from '../../../lib/stores/transcript.svelte';
import { ws } from '../../../lib/stores/workspace.svelte';
import { toastError } from '../../../lib/toastError';
import { submitPrompt } from './api';

export const UNAVAILABLE_TRANSCRIPT_COPY: Record<
  TranscriptUnavailableReason,
  { title: string; body: string }
> = {
  no_provider_session_id: {
    title: 'No transcript yet',
    body: 'The agent has not written a transcript for this session (it appears after the first prompt). The terminal has everything so far.',
  },
  transcript_missing: {
    title: 'Transcript not found on disk',
    body: 'The provider transcript file for this session is gone or was never created here. Use the terminal view.',
  },
  provider_unsupported: {
    title: 'Chat view not available for this provider',
    body: 'This agent does not keep a readable transcript. Use the terminal view.',
  },
  codex_rollout_unresolved: {
    title: 'Codex rollout not matched',
    body: 'Otto couldn’t match this session to a Codex rollout file yet. It usually resolves after the next turn; the terminal is complete meanwhile.',
  },
};

export async function runApprovedPlanImplementation(sessionId: string): Promise<void> {
  const followUp = 'Implement the approved plan above step by step.';
  try {
    await ws.updateSessionConfig(
      sessionId,
      { interaction_mode: 'full' },
      { toastTitle: 'Switched to Full access' },
    );
    transcript.recordOptimisticPrompt(sessionId, followUp);
    await submitPrompt(sessionId, followUp);
  } catch (e) {
    transcript.clearOptimisticPrompt(sessionId);
    toastError('Couldn’t start plan implementation', e);
  }
}

export function buildConversationMenuItems(opts: {
  includeSearch: boolean;
  showSystem: boolean;
  expandAll: boolean;
  hasChanges: boolean;
  onOpenSearch: () => void;
  onToggleSystem: () => void;
  onToggleExpandAll: () => void;
  onOpenAllChanges: () => void;
  onPrevPrompt: () => void;
  onNextPrompt: () => void;
  onJumpLatest: () => void;
  onRefresh: () => void;
}): MenuItem[] {
  return [
    ...(opts.includeSearch
      ? [{ label: 'Search…', icon: 'search' as const, hint: '⌘F', action: opts.onOpenSearch }]
      : []),
    {
      label: 'Show system notes',
      icon: opts.showSystem ? 'eye' : 'eyeOff',
      checked: opts.showSystem,
      title: 'Reveal system reminders, hooks, attachments and injected queue items',
      action: opts.onToggleSystem,
    },
    {
      label: 'Show all work',
      icon: 'layers',
      checked: opts.expandAll,
      title: 'Open every “Worked for …” fold — tool calls, narration and plan updates',
      action: opts.onToggleExpandAll,
    },
    ...(opts.hasChanges
      ? [{ label: 'All changes', icon: 'split' as const, action: opts.onOpenAllChanges }]
      : []),
    { separator: true },
    { label: 'Previous message of yours', icon: 'chevronUp', hint: '⌥↑', action: opts.onPrevPrompt },
    { label: 'Next message of yours', icon: 'chevronDown', hint: '⌥↓', action: opts.onNextPrompt },
    { label: 'Jump to latest', icon: 'arrowDown', hint: '⌘↓', action: opts.onJumpLatest },
    { separator: true },
    { label: 'Refresh transcript', icon: 'refresh', action: opts.onRefresh },
  ];
}
