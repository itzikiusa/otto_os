// Pure Markdown formatting for coding-agent transcripts (zero runtime imports
// so unit tests in plain Node and Vite bundles both import directly).

import type { Block, Turn } from '../../../lib/api/types';

/** Strip ANSI CSI / OSC / 2-byte escape sequences and normalize CR/CRLF. */
export function stripAnsi(raw: string): string {
  return raw
    .replace(/\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)/g, '')
    .replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, '')
    .replace(/\x1b[@-Z\\-_]/g, '')
    .replace(/\r\n?/g, '\n');
}

export interface ExportTranscriptOptions {
  title?: string | null;
  provider?: string | null;
  model?: string | null;
  cwd?: string | null;
  turns: readonly Turn[];
}

const PROVIDER_LABELS: Record<string, string> = {
  claude: 'Claude',
  codex: 'Codex',
  agy: 'Antigravity',
  gemini: 'Gemini',
};

function speakerLabel(provider?: string | null): string {
  const key = (provider ?? '').trim().toLowerCase();
  if (!key) return 'Assistant';
  return PROVIDER_LABELS[key] ?? key.charAt(0).toUpperCase() + key.slice(1);
}

function toolTarget(input: unknown): string {
  if (!input || typeof input !== 'object') return '';
  const rec = input as Record<string, unknown>;
  for (const key of ['command', 'cmd', 'file_path', 'path', 'pattern', 'query', 'url', 'prompt']) {
    const val = rec[key];
    if (typeof val === 'string' && val.trim()) return val.trim();
  }
  return '';
}

function formatToolSummary(block: Extract<Block, { kind: 'tool_call' }>): string {
  const label = stripAnsi((block.title || block.name || block.tool).trim());
  const target = stripAnsi(toolTarget(block.input)).trim();
  const failed = block.result?.ok === false ? ' _(failed)_' : '';
  if (target) {
    const safeTarget = target.replace(/`/g, "'").replace(/\s+/g, ' ');
    return `- **${label}:** \`${safeTarget}\`${failed}`;
  }
  return `- **${label}**${failed}`;
}

/** Format transcript turns into clean, paste-ready Markdown (no raw ANSI or
 *  unbounded tool stdout dumps). Returns `''` when there is no readable content. */
export function formatTranscriptMarkdown(opts: ExportTranscriptOptions): string {
  const title = stripAnsi(opts.title?.trim() || 'Agent conversation');
  const assistantHeading = speakerLabel(opts.provider);

  const metaParts: string[] = [];
  if (opts.provider) metaParts.push(`Provider: ${assistantHeading}`);
  if (opts.model) metaParts.push(`Model: \`${stripAnsi(opts.model).trim()}\``);
  if (opts.cwd) metaParts.push(`Folder: \`${stripAnsi(opts.cwd).trim()}\``);

  const sections: string[] = [];
  for (const turn of opts.turns) {
    const parts: string[] = [];
    const tools: string[] = [];

    const flushTools = () => {
      if (tools.length > 0) {
        parts.push(tools.join('\n'));
        tools.length = 0;
      }
    };

    for (const block of turn.blocks) {
      if (block.kind === 'text') {
        const clean = stripAnsi(block.md).trim();
        if (clean) {
          flushTools();
          parts.push(clean);
        }
      } else if (block.kind === 'tool_call') {
        tools.push(formatToolSummary(block));
      } else if (block.kind === 'subagent') {
        const desc = stripAnsi(block.description || block.agent_id).trim();
        if (desc) tools.push(`- **Subagent:** ${desc}`);
      } else if (block.kind === 'image') {
        flushTools();
        const alt = stripAnsi(block.alt?.trim() || block.id);
        parts.push(`_Image: ${alt}_`);
      }
    }
    flushTools();

    if (parts.length === 0) continue;
    const speaker = turn.role === 'user' ? 'You' : assistantHeading;
    sections.push(`## ${speaker}\n\n${parts.join('\n\n')}`);
  }

  if (sections.length === 0) return '';

  const header = [`# ${title}`];
  if (metaParts.length > 0) header.push(`_${metaParts.join(' · ')}_`);
  return `${header.join('\n\n')}\n\n${sections.join('\n\n---\n\n')}\n`;
}
