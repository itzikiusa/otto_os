// Keyboard triage handler for the MCP Approvals queue:
//   J / K  — select next / previous pending approval
//   A      — approve the selected pending approval
//   D      — deny the selected pending approval
// Suppressed whenever focus is inside an input, textarea, select, contentEditable,
// or modal dialog so typing notes or interacting with dialogs is never hijacked.

export type ApprovalTriageAction =
  | { kind: 'select'; id: string }
  | { kind: 'approve'; id: string }
  | { kind: 'deny'; id: string }
  | null;

interface TriageKeyEvent {
  key: string;
  metaKey?: boolean;
  ctrlKey?: boolean;
  altKey?: boolean;
  target?: unknown;
}

interface TriageOptions {
  modalOpen?: boolean;
  canDecideId?: (id: string) => boolean;
}

function isEditableOrDialogTarget(target: unknown): boolean {
  if (!target || typeof target !== 'object') return false;
  const el = target as {
    tagName?: string;
    isContentEditable?: boolean;
    closest?: (selector: string) => unknown;
  };
  if (el.isContentEditable) return true;
  const tag = (el.tagName ?? '').toUpperCase();
  if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return true;
  if (typeof el.closest === 'function') {
    if (el.closest('input, textarea, select, [contenteditable="true"], [role="dialog"], [aria-modal="true"]')) {
      return true;
    }
  }
  return false;
}

export function approvalTriageAction(
  ev: TriageKeyEvent,
  pendingIds: readonly string[],
  focusedId: string | null,
  opts: TriageOptions = {},
): ApprovalTriageAction {
  if (opts.modalOpen || ev.metaKey || ev.ctrlKey || ev.altKey) return null;
  if (isEditableOrDialogTarget(ev.target)) return null;
  if (pendingIds.length === 0) return null;

  const key = ev.key.toLowerCase();
  const currentIdx = focusedId ? pendingIds.indexOf(focusedId) : -1;

  if (key === 'j') {
    const nextIdx = currentIdx < 0 ? 0 : Math.min(pendingIds.length - 1, currentIdx + 1);
    return { kind: 'select', id: pendingIds[nextIdx] };
  }
  if (key === 'k') {
    const nextIdx = currentIdx < 0 ? 0 : Math.max(0, currentIdx - 1);
    return { kind: 'select', id: pendingIds[nextIdx] };
  }
  if (key === 'a' || key === 'd') {
    const id = currentIdx >= 0 ? pendingIds[currentIdx] : pendingIds[0];
    if (opts.canDecideId && !opts.canDecideId(id)) return null;
    return { kind: key === 'a' ? 'approve' : 'deny', id };
  }
  return null;
}
