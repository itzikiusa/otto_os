// Outward-action approval helpers for RemoteLiveView (fetch detail + submit decision).
import { ApiError } from '../../../lib/api/client';
import { mcpCpApi } from '../../../lib/api/mcp';
import { confirmer } from '../../../lib/confirm.svelte';
import { loadErrorText } from '../../../lib/loadError';
import type { McpApproval } from '../../../lib/api/types';
import type { ApprovalChoice } from './approvalFacts';

export function classifyConnectError(e: unknown):
  | { kind: 'not_installed'; reason: string }
  | { kind: 'ended'; reason: string }
  | { kind: 'retry'; note: string; reason: string } {
  const status = e instanceof ApiError ? e.status : 0;
  const code = e instanceof ApiError ? e.code : '';
  if (status === 409 && code === 'engine_not_installed') {
    return { kind: 'not_installed', reason: 'The live browser engine is not installed.' };
  }
  if (status === 409) {
    return { kind: 'ended', reason: 'Someone else has this tab open live. Open the page in a new tab to browse it yourself.' };
  }
  if (status === 429) {
    return { kind: 'ended', reason: 'Too many live tabs are open on this Mac. Close one, then reconnect.' };
  }
  if (status === 400 || status === 403 || status === 404) {
    return { kind: 'ended', reason: e instanceof Error && e.message ? e.message : 'This page can’t be opened live.' };
  }
  return { kind: 'retry', note: loadErrorText(e), reason: 'Couldn’t start the live browser.' };
}

export async function fetchPendingApproval(id: string): Promise<McpApproval | null> {
  try {
    const rows = await mcpCpApi.cpApprovals('pending');
    return rows.find((r) => r.id === id) ?? null;
  } catch {
    return null;
  }
}

export async function submitApprovalDecision(
  id: string,
  choice: ApprovalChoice,
  reason?: string | null,
): Promise<{ status: 'cancelled' | 'done' | 'error'; error?: string }> {
  let note: string | undefined;
  if (choice === 'deny' && reason !== undefined) {
    note = reason?.trim() || undefined;
  } else if (choice === 'deny') {
    const why = await confirmer.promptText('Tell the agent why (optional). It won’t send this request.', {
      title: 'Deny the request',
      confirmLabel: 'Deny',
      placeholder: 'Wrong time slot',
    });
    if (why === null) return { status: 'cancelled' };
    note = why.trim() || undefined;
  } else if (choice === 'take_over') {
    note = 'Taken over by the user — they will finish this step themselves.';
  }
  try {
    await mcpCpApi.cpDecide(id, { approved: choice === 'approve', note });
    return { status: 'done' };
  } catch (e) {
    const msg =
      e instanceof ApiError && e.status === 403
        ? 'You can’t decide MCP approvals in this workspace. Ask an admin, or take over the page.'
        : `Couldn’t record your decision: ${loadErrorText(e)}`;
    return { status: 'error', error: msg };
  }
}
