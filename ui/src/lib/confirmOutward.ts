// One confirm shape for OUTWARD-FACING actions — anything other people see or
// that leaves this Mac (approve/decline a PR, post a review comment, move or
// assign a Jira issue, post a Jira comment, publish to Jira/Confluence). The
// design guidelines (patterns.md §5) require every such confirm to say WHERE
// it goes, WHAT is sent and WHO sees it, and to label the button with the
// action itself — never "OK"/"Yes". Building the copy here keeps those three
// lines consistent across modules instead of each call site improvising.

import { confirmer } from './confirm.svelte';

export interface OutwardConfirm {
  /** The action, as the confirm button reads it: "Approve PR", "Post comment". */
  verb: string;
  /** Destination: "acme/api · PR #42", "Jira GS-123", "Confluence page “Runbook”". */
  where: string;
  /** What is sent — a short preview or a before → after summary. */
  what?: string;
  /** Who will see it: "The PR author and reviewers are notified." */
  who?: string;
  /** Dialog title; defaults to `${verb}?`. */
  title?: string;
  /** Red confirm button (e.g. declining a PR). Defaults to false. */
  danger?: boolean;
}

/** Max characters of a `what` preview before it is ellipsised. */
const PREVIEW_MAX = 400;

/** Build the dialog body (exported for tests / previews). */
export function outwardMessage(o: Pick<OutwardConfirm, 'where' | 'what' | 'who'>): string {
  const lines = [`Where: ${o.where}`];
  if (o.what) {
    const w = o.what.trim();
    lines.push(`What: ${w.length > PREVIEW_MAX ? `${w.slice(0, PREVIEW_MAX - 1).trimEnd()}…` : w}`);
  }
  if (o.who) lines.push(`Who sees it: ${o.who}`);
  return lines.join('\n\n');
}

/** Ask before an outward-facing action. Resolves true when the user confirms. */
export function confirmOutward(o: OutwardConfirm): Promise<boolean> {
  return confirmer.ask(outwardMessage(o), {
    title: o.title ?? `${o.verb}?`,
    confirmLabel: o.verb,
    danger: o.danger ?? false,
  });
}
