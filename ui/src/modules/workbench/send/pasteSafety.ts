// What may be typed into a live session from a Workbench file, and how.
//
// Workbench files are not trusted: an agent can write one (`workbench_write`),
// and its text reaches the PTY verbatim (`POST /sessions/{id}/input` with
// `submit: false`). Two things would turn "paste, never run" into "run":
//
//  1. Control bytes. `ESC[201~` inside the text ends a bracketed paste early,
//     so everything after it arrives as live keystrokes; a `\r` is Enter; C1
//     codes (U+009B is a one-byte CSI) and other C0 codes act as keys too.
//     `cleanPasteText` removes them all, keeping only `\t` and `\n`.
//  2. Clients without bracketed-paste mode. ssh to an old host, redis-cli,
//     mongosh, mysql, clickhouse-client — or a k8s exec — run each line as it
//     arrives. A connection session (`kind: 'connection'`) therefore gets ONE
//     line, typed without paste markers and without Enter; multi-line text is
//     refused there (`planPaste`).
//
// Kept free of runtime imports so the unit tests load it directly.

import type { Session } from '../../../lib/api/types';

/** Sessions the text can be pasted into: this workspace's, not archived, still
 *  alive. Connection sessions stay listed — they take a single line. */
export function pasteTargets(sessions: Session[], ws: string): Session[] {
  return sessions.filter((s) => s.workspace_id === ws && !s.archived && s.status !== 'exited');
}

/** Agent CLIs and plain shells (`kind: 'agent'`, incl. provider `shell`)
 *  take a bracketed paste; a connection session (ssh, DB clients, k8s exec,
 *  custom commands) — or any kind this build does not know — is typed one line
 *  at a time. */
export function takesMultiLine(session: Pick<Session, 'kind'>): boolean {
  return session.kind === 'agent';
}

// Every C0 control except TAB (09) and LF (0a) — ESC (1b) and CR (0d) included —
// plus DEL and the C1 block (U+0080–U+009F, e.g. U+009B CSI). CR is handled
// before this runs (line endings are normalised, not dropped).
// eslint-disable-next-line no-control-regex
const UNSAFE = /[\u0000-\u0008\u000b-\u001f\u007f-\u009f]/g;

export interface CleanedText {
  text: string;
  /** Control characters dropped (line-ending normalisation not counted). */
  removed: number;
}

/** Normalise line endings (CRLF / lone CR → LF), then drop every control
 *  character a terminal would act on. Removal is a single pass over the whole
 *  string and ESC itself is dropped, so no escape sequence can be re-formed
 *  from pieces (`ESC` + `[201~` split across the text stays inert text). */
export function cleanPasteText(raw: string): CleanedText {
  const normalised = raw.replace(/\r\n?/g, '\n');
  let removed = 0;
  const text = normalised.replace(UNSAFE, () => {
    removed += 1;
    return '';
  });
  return { text, removed };
}

/** Bracketed paste (`ESC[200~ … ESC[201~`) without a trailing newline. The
 *  body is cleaned first, so it cannot close the paste itself. */
export function bracketedPaste(text: string): string {
  return `\u001b[200~${cleanPasteText(text).text.replace(/\n+$/, '')}\u001b[201~`;
}

export type PastePlan =
  | { ok: true; payload: string; lines: number; removed: number; multiLine: boolean }
  | { ok: false; reason: 'empty' | 'multi-line'; lines: number; removed: number };

/** Decide what (if anything) is written to `session` for `raw`. Trailing blank
 *  lines are dropped; nothing ever carries a final newline. */
export function planPaste(raw: string, session: Pick<Session, 'kind'>): PastePlan {
  const { text, removed } = cleanPasteText(raw);
  const body = text.replace(/\n+$/, '');
  const lines = body === '' ? 0 : body.split('\n').length;
  if (!body.trim()) return { ok: false, reason: 'empty', lines, removed };
  if (takesMultiLine(session)) {
    return { ok: true, payload: `\u001b[200~${body}\u001b[201~`, lines, removed, multiLine: true };
  }
  if (lines > 1) return { ok: false, reason: 'multi-line', lines, removed };
  return { ok: true, payload: body, lines, removed, multiLine: false };
}

function count(n: number, word: string): string {
  return `${n} ${n === 1 ? word : `${word}s`}`;
}

/** The sentence added to a confirm when cleaning changed the text. */
export function removedNote(removed: number): string {
  return removed > 0
    ? ` ${count(removed, 'control character')} (escape codes, carriage returns or similar) ${removed === 1 ? 'was' : 'were'} removed, so the text cannot act as key presses.`
    : '';
}

/** Confirm body for an ok plan — true for each kind of target. */
export function pasteConfirmText(
  plan: Extract<PastePlan, { ok: true }>,
  docName: string,
  session: Pick<Session, 'title' | 'id' | 'provider'>,
): string {
  const target = `“${session.title || session.id}”`;
  if (!plan.multiLine) {
    return (
      `Type 1 line from “${docName}” into ${target}? No Enter is sent — it runs only when you press Enter in that session.` +
      removedNote(plan.removed)
    );
  }
  const shellNote =
    session.provider === 'shell'
      ? ' If that shell is running another program (ssh, a database client), it may not support bracketed paste and could run each line as it arrives.'
      : '';
  return (
    `Paste ${count(plan.lines, 'line')} from “${docName}” into ${target}? ` +
    'It goes in as a bracketed paste with no Enter, so it waits as typed text until you press Enter in that session.' +
    shellNote +
    removedNote(plan.removed)
  );
}

/** Why a multi-line file is refused for a connection session. */
export function multiLineRefusal(lines: number): string {
  return (
    `This is a connection session, and the file has ${count(lines, 'line')}. ` +
    'Clients like ssh, redis-cli, mongosh or mysql can run each line the moment it arrives, so Otto pastes only a single line there. ' +
    'Copy the text and paste it yourself, or send it to an agent or shell session.'
  );
}

/** Whether `raw` is more than one line once cleaned (trailing newlines aside). */
export function isMultiLine(raw: string): boolean {
  return cleanPasteText(raw).text.replace(/\n+$/, '').includes('\n');
}

/** Picker meta for a row: connection sessions say they take one line. */
export function targetNote(session: Pick<Session, 'kind'>, multiLineText: boolean): string {
  if (takesMultiLine(session)) return '';
  return multiLineText ? 'one line only — not this file' : 'one line only';
}
