// Turn a held browser action's MCP approval row into the where / what / who
// lines of the approval card. Pure (type-only imports) and unit-tested.
//
// The daemon files `kind:"browser_action"` approvals with "the origin,
// method, target host and the screenshot's path" in the (redacted) args
// (api.md "Outward actions"). The exact key names aren't pinned by the
// contract, so a few spellings are accepted and anything missing is simply
// left out rather than guessed.

import type { McpApproval } from '../../../lib/api/types';

export type ApprovalChoice = 'approve' | 'deny' | 'take_over';

export interface FactRow {
  label: string;
  value: string;
  /** Render left-to-right in mono (hosts, URLs). */
  ltr?: boolean;
}

export interface ApprovalFacts {
  rows: FactRow[];
  /** The agent's stated reason, if any. */
  why: string | null;
  /** A pre-action screenshot was captured. */
  screenshot: boolean;
  /** "An agent" or its identity. */
  requester: string;
}

function pick(o: Record<string, unknown>, keys: string[]): string | null {
  for (const k of keys) {
    const v = o[k];
    if (typeof v === 'string' && v.trim()) return v.trim();
  }
  return null;
}

function hostOf(u: string | null): string | null {
  if (!u) return null;
  try {
    return new URL(u).host || null;
  } catch {
    return null;
  }
}

export function approvalFacts(a: McpApproval | null, profile: string | null): ApprovalFacts {
  let args: Record<string, unknown> = {};
  if (a?.args_redacted_json) {
    try {
      const v = JSON.parse(a.args_redacted_json);
      if (v && typeof v === 'object' && !Array.isArray(v)) args = v as Record<string, unknown>;
    } catch {
      /* unreadable args: show what the row itself says */
    }
  }
  const origin = pick(args, ['origin', 'page_origin', 'from']);
  const method = pick(args, ['method', 'http_method']);
  const target = pick(args, ['target_host', 'host', 'target']) ?? hostOf(pick(args, ['url', 'target_url']));
  const screenshot = !!pick(args, ['screenshot', 'screenshot_path', 'screenshot_url']);

  const rows: FactRow[] = [];
  if (origin) rows.push({ label: 'Where', value: origin, ltr: true });
  if (method || target) {
    rows.push({
      label: 'What',
      value: `${method ? method.toUpperCase() : 'A'} request${target ? ` to ${target}` : ''}`,
    });
  }
  if (target) {
    rows.push({ label: 'Who sees it', value: `Whoever runs ${target}. The request leaves this Mac.` });
  }
  if (profile) {
    rows.push({
      label: 'As',
      value:
        profile === 'ephemeral'
          ? 'A private session (its cookies are wiped when it closes)'
          : `Your saved profile “${profile}”`,
    });
  }
  const requester =
    a?.requested_by_kind === 'agent' || !a?.requested_by_kind ? 'An agent asked' : `${a.requested_by_kind} asked`;
  return { rows, why: a?.detail?.trim() || null, screenshot, requester };
}
