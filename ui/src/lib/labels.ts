// Wire enums → words a person reads. Wire values (`in_progress`, `needs_changes`)
// never go on screen; each map overrides the few that sentence-casing alone
// reads badly, and anything unknown falls back to `sentenceCase`, so a new
// backend state degrades to readable text instead of a raw identifier.

/** `needs_changes` / `needs-changes` / `NeedsChanges` → "Needs changes". */
export function sentenceCase(v: string | null | undefined): string {
  const s = (v ?? '')
    .replace(/([a-z0-9])([A-Z])/g, '$1 $2')
    .replace(/[_-]+/g, ' ')
    .trim()
    .toLowerCase();
  return s ? s[0].toUpperCase() + s.slice(1) : '';
}

/** The one "needs a workspace" message: toast titles, disabled-control
 *  titles and empty-state headings all say it the same way. */
export const NO_WORKSPACE = 'Select a workspace first';

function labeler(map: Record<string, string>): (v: string | null | undefined) => string {
  return (v) => (v != null && map[v]) || sentenceCase(v);
}

/** Run / agent / reviewer / summarizer / revision state. */
export const runStateLabel = labeler({
  pending: 'Waiting',
  running: 'Running',
  done: 'Done',
  ok: 'OK',
  error: 'Failed',
  failed: 'Failed',
  cancelled: 'Canceled',
  canceled: 'Canceled',
  timed_out: 'Timed out',
});

/** Review-finding severity. */
export const severityLabel = labeler({
  blocker: 'Blocker',
  critical: 'Critical',
  major: 'Major',
  minor: 'Minor',
  nit: 'Nit',
  info: 'Info',
});

/** Generic `kind` chip (run kind, version kind, message kind, attachment kind). */
export const kindLabel = labeler({
  ai: 'AI',
  api: 'API',
  pr: 'PR',
  qa: 'QA',
  url: 'URL',
});

/** Session-room recap / capture state. */
export const recapStateLabel = labeler({
  finalizing: 'Finalizing',
  ready: 'Ready',
  partial: 'Partial',
});

/** Review verdict. */
export const verdictLabel = labeler({
  approve: 'Approve',
  approved: 'Approved',
  request_changes: 'Changes requested',
  needs_changes: 'Changes requested',
  comment: 'Comment only',
});

/** Handover delivery state. */
export const handoverStateLabel = labeler({
  pending: 'Waiting',
  delivered: 'Delivered',
  failed: 'Failed',
  error: 'Failed',
});
