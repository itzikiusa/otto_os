// The WIP commit composer's text and in-flight agent draft, kept OUTSIDE the
// panel so they survive navigation: leaving the Git page (or picking a commit)
// unmounts WipPanel, which used to throw away a typed summary/description and
// — worse — the agent draft still running for it (its reply landed in a dead
// component). Per repo, per window: sessionStorage for the text (a reload
// keeps it, another window doesn't share it), module memory for the request.
//
// Kept apart from the git store on purpose: this is composer state, not
// repo/fetch state.

import type { DraftCommitMessageResp } from '../../lib/api/types';

export interface ComposerText {
  subject: string;
  body: string;
  /** When the agent's message landed (the byline), null when hand-written. */
  draftedAt: number | null;
}

interface Stored extends ComposerText {
  /** A draft that finished while no panel was showing this repo. */
  pending?: DraftCommitMessageResp | null;
}

const EMPTY: ComposerText = { subject: '', body: '', draftedAt: null };
const key = (repoId: string): string => `otto_commit_composer:${repoId}`;

function read(repoId: string): Stored {
  try {
    const raw = sessionStorage.getItem(key(repoId));
    if (!raw) return { ...EMPTY };
    const v = JSON.parse(raw) as Partial<Stored>;
    return {
      subject: typeof v.subject === 'string' ? v.subject : '',
      body: typeof v.body === 'string' ? v.body : '',
      draftedAt: typeof v.draftedAt === 'number' ? v.draftedAt : null,
      pending: v.pending && typeof v.pending.message === 'string' ? v.pending : null,
    };
  } catch {
    return { ...EMPTY };
  }
}

function write(repoId: string, s: Stored): void {
  try {
    if (!s.subject && !s.body && !s.pending) sessionStorage.removeItem(key(repoId));
    else sessionStorage.setItem(key(repoId), JSON.stringify(s));
  } catch {
    /* blocked / over quota — the composer still works, it just won't persist */
  }
}

/** The saved summary / description for `repoId` (empty when none). */
export function readComposer(repoId: string): ComposerText {
  const { subject, body, draftedAt } = read(repoId);
  return { subject, body, draftedAt };
}

/** Save what the composer shows now (empty text clears the entry). */
export function writeComposer(repoId: string, text: ComposerText): void {
  write(repoId, { ...text, pending: read(repoId).pending ?? null });
}

/** A draft that finished while the panel was away — handed out once. */
export function takePendingDraft(repoId: string): DraftCommitMessageResp | null {
  const s = read(repoId);
  if (!s.pending) return null;
  write(repoId, { ...s, pending: null });
  return s.pending;
}

const inflight = new Map<string, Promise<DraftCommitMessageResp>>();
const watchers = new Map<string, number>();

/** Start (or join) the agent draft for `repoId`. If no panel is watching the
 *  repo when it lands, the reply is parked for {@link takePendingDraft}. */
export function startDraft(
  repoId: string,
  run: () => Promise<DraftCommitMessageResp>,
): Promise<DraftCommitMessageResp> {
  const running = inflight.get(repoId);
  if (running) return running;
  const p = run();
  inflight.set(repoId, p);
  p.then(
    (d) => {
      if (!watchers.get(repoId)) write(repoId, { ...read(repoId), pending: d });
    },
    () => {},
  ).finally(() => {
    if (inflight.get(repoId) === p) inflight.delete(repoId);
  });
  return p;
}

/** The draft still running for `repoId`, if any (a remounted panel resumes
 *  its spinner on it). */
export function draftInFlight(repoId: string): Promise<DraftCommitMessageResp> | null {
  return inflight.get(repoId) ?? null;
}

/** A panel shows `repoId`: replies land in it, not the parking slot. */
export function watchComposer(repoId: string): () => void {
  watchers.set(repoId, (watchers.get(repoId) ?? 0) + 1);
  let done = false;
  return () => {
    if (done) return;
    done = true;
    const n = (watchers.get(repoId) ?? 1) - 1;
    if (n <= 0) watchers.delete(repoId);
    else watchers.set(repoId, n);
  };
}
