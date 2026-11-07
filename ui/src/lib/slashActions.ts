// Client-side interactive slash commands for Threads & ConversationView.
// Turns `/model`, `/effort`, `/plan`, `/mode`, `/status`, `/diff`, `/files`,
// `/terminal`, `/new`, `/clear`, `/fork`, and `/review` into real interactive
// in-thread controls (matching T3Code & PandaCode) instead of sending dead
// text to a hidden TUI.
import type { SlashCommand } from './api/types';
import {
  normalizeEffort,
  type EffortLevel,
  type InteractionMode,
} from './providerModels.ts';

export interface ComposerSlashCommand {
  name: string;
  description: string;
  source: SlashCommand['source'] | 'interactive';
}

export type ParsedSlashIntent =
  | { kind: 'open-model-picker' }
  | { kind: 'set-model'; model: string }
  | { kind: 'open-effort-picker' }
  | { kind: 'set-effort'; effort: EffortLevel }
  | { kind: 'open-mode-picker' }
  | { kind: 'set-mode'; mode: InteractionMode; prompt?: string }
  | { kind: 'toggle-status' }
  | { kind: 'open-diff' }
  | { kind: 'open-files' }
  | { kind: 'toggle-terminal' }
  | { kind: 'new-thread'; beside: boolean }
  | { kind: 'fresh-context' }
  | { kind: 'side-chat'; question: string }
  | { kind: 'send-prompt'; prompt: string };

export const INTERACTIVE_SLASH_COMMANDS: readonly ComposerSlashCommand[] = [
  {
    name: 'model',
    description: 'Switch model interactively (or /model <name>)',
    source: 'interactive',
  },
  {
    name: 'effort',
    description: 'Choose reasoning effort level (low, medium, high, xhigh, max)',
    source: 'interactive',
  },
  {
    name: 'plan',
    description: 'Switch to Plan mode (or /plan <task> to plan immediately)',
    source: 'interactive',
  },
  {
    name: 'mode',
    description: 'Choose execution mode (Full access, Accept edits, Plan)',
    source: 'interactive',
  },
  {
    name: 'code',
    description: 'Switch out of Plan mode back to Full access',
    source: 'interactive',
  },
  {
    name: 'permissions',
    description: 'Choose tool permission mode for this thread',
    source: 'interactive',
  },
  {
    name: 'status',
    description: 'Show thread model, effort, mode, branch, and usage card',
    source: 'interactive',
  },
  {
    name: 'cost',
    description: 'Show token usage and session cost summary',
    source: 'interactive',
  },
  {
    name: 'diff',
    description: 'Inspect git changes for this workspace',
    source: 'interactive',
  },
  {
    name: 'files',
    description: 'Browse workspace files in the inspector',
    source: 'interactive',
  },
  {
    name: 'terminal',
    description: 'Toggle the docked terminal drawer',
    source: 'interactive',
  },
  {
    name: 'new',
    description: 'Start a fresh thread with the same harness and folder',
    source: 'interactive',
  },
  {
    name: 'clear',
    description: 'Start the next message with fresh context in this thread',
    source: 'interactive',
  },
  {
    name: 'fork',
    description: 'Open a parallel thread beside this one in Split view',
    source: 'interactive',
  },
  {
    name: 'review',
    description: 'Review uncommitted git changes in this repository',
    source: 'interactive',
  },
  {
    name: 'btw',
    description: 'Ask a side question without interrupting (opens a side chat)',
    source: 'interactive',
  },
];

/** Interactive commands only a host that supports them should list. */
export const HOST_ONLY_SLASH_COMMANDS: ReadonlySet<string> = new Set(['btw']);

/** TUI-only built-ins that hang or make no sense inside a structured thread view. */
const HIDDEN_TUI_BUILTINS = new Set([
  'exit',
  'quit',
  'vim',
  'terminal-setup',
]);

const IMMEDIATE_PICKER_COMMANDS = new Set([
  'model',
  'effort',
  'plan',
  'mode',
  'code',
  'permissions',
  'status',
  'cost',
  'usage',
  'diff',
  'files',
  'terminal',
  'new',
  'clear',
  'fork',
  'review',
]);

/** Merge client-side interactive commands ahead of server-discovered skills/commands. */
export function mergeSlashCommands(
  serverCmds: readonly SlashCommand[] | null,
  opts: { omitInteractive?: ReadonlySet<string> } = {},
): ComposerSlashCommand[] {
  const byName = new Map<string, ComposerSlashCommand>();
  for (const c of INTERACTIVE_SLASH_COMMANDS) {
    if (opts.omitInteractive?.has(c.name)) continue;
    byName.set(c.name, c);
  }
  for (const s of serverCmds ?? []) {
    if (HIDDEN_TUI_BUILTINS.has(s.name)) continue;
    if (byName.has(s.name) && s.source === 'builtin') continue;
    if (!byName.has(s.name)) {
      byName.set(s.name, s);
    }
  }
  return Array.from(byName.values());
}

/** Whether accepting this command from the `/` popup with no args should run right away. */
export function isImmediateSlashCommand(name: string): boolean {
  return IMMEDIATE_PICKER_COMMANDS.has(name.trim().toLowerCase());
}

/** Parse a slash command string (`/model opus`, `/plan Design X`, `/status`, …). */
export function parseInteractiveSlashInput(raw: string): ParsedSlashIntent | null {
  const trimmed = raw.trim();
  if (!trimmed.startsWith('/')) return null;
  const match = /^\/([\w:-]+)(?:\s+([\s\S]*))?$/.exec(trimmed);
  if (!match) return null;
  const cmd = match[1].toLowerCase();
  const arg = (match[2] ?? '').trim();

  switch (cmd) {
    case 'model':
      return arg ? { kind: 'set-model', model: arg } : { kind: 'open-model-picker' };
    case 'effort':
    case 'thinking': {
      if (!arg) return { kind: 'open-effort-picker' };
      const eff = normalizeEffort(arg);
      return eff ? { kind: 'set-effort', effort: eff } : { kind: 'open-effort-picker' };
    }
    case 'plan':
      return { kind: 'set-mode', mode: 'plan', ...(arg ? { prompt: arg } : {}) };
    case 'code':
      return { kind: 'set-mode', mode: 'full', ...(arg ? { prompt: arg } : {}) };
    case 'mode':
    case 'permissions': {
      if (!arg) return { kind: 'open-mode-picker' };
      const low = arg.toLowerCase();
      if (low === 'plan') return { kind: 'set-mode', mode: 'plan' };
      if (low === 'accept-edits' || low === 'standard' || low === 'edits') {
        return { kind: 'set-mode', mode: 'accept-edits' };
      }
      if (low === 'full' || low === 'auto' || low === 'bypass') {
        return { kind: 'set-mode', mode: 'full' };
      }
      return { kind: 'open-mode-picker' };
    }
    case 'status':
    case 'cost':
    case 'usage':
    case 'stats':
      return { kind: 'toggle-status' };
    case 'diff':
    case 'changes':
      return { kind: 'open-diff' };
    case 'files':
      return { kind: 'open-files' };
    case 'terminal':
      return { kind: 'toggle-terminal' };
    case 'new':
      return { kind: 'new-thread', beside: false };
    case 'clear':
      return { kind: 'fresh-context' };
    case 'fork':
      return { kind: 'new-thread', beside: true };
    case 'btw':
      return { kind: 'side-chat', question: arg };
    case 'review':
      return {
        kind: 'send-prompt',
        prompt: arg
          ? `Review the following focus area across current git changes: ${arg}`
          : 'Inspect git status and uncommitted diffs in this repository, identify any bugs, regressions, or missing edge-case tests, and propose concrete fixes.',
      };
    default:
      return null;
  }
}
