// HTTP surface of the conversation view (docs/design/conversation-view.md §4.3).
// Every path the daemon derives itself; the ONLY client-supplied path is the
// History route's `?path=` (server-confined to the two provider roots).
import { api, authedBlobUrl } from '../../../lib/api/client';
import type { FsBrowse, InboxUploadResp, SlashCommand } from '../../../lib/api/types';

// Reading lives in the store (it owns paging + the live tail); re-exported so
// the module has one import surface.
export { fetchTranscript, sourceKey, type TranscriptSource, type TranscriptPage } from '../../../lib/stores/transcript.svelte';

/** Authed fetch → revocable blob URL for a transcript image (`GET
 *  …/transcript/images/{id}`). Only live/known sessions serve images — an
 *  on-disk history transcript has no session id, so callers show a placeholder. */
export function fetchImageUrl(sessionId: string, imageId: string): Promise<string> {
  return authedBlobUrl(
    `/sessions/${encodeURIComponent(sessionId)}/transcript/images/${encodeURIComponent(imageId)}`,
  );
}

/** Submit ONE prompt into the agent's PTY — the CLI's own "type + Enter", so
 *  the text lands as a submitted turn (Track A's `submit_text`, never a raw
 *  paste burst). Isolated here so the endpoint swap is a one-liner once the
 *  dedicated prompt route lands (see §8 of the design doc). */
/** Slash commands the composer can complete for this session (provider built-ins + user/project commands and skills). */
export function fetchSlashCommands(sessionId: string): Promise<SlashCommand[]> {
  return api.get<SlashCommand[]>(`/sessions/${encodeURIComponent(sessionId)}/slash-commands`);
}

export function submitPrompt(sessionId: string, text: string): Promise<void> {
  return api.post<void>(`/sessions/${encodeURIComponent(sessionId)}/input`, { text, submit: true });
}

/** Store a pasted/dropped image beside the session (`<data>/sessions/<id>/inbox/`)
 *  and get back the absolute path the composer inserts as `[Image: <path>]`. */
export async function uploadInboxImage(sessionId: string, file: File | Blob, filename: string): Promise<string> {
  const data_b64 = await blobToBase64(file);
  const resp = await api.post<InboxUploadResp>(`/sessions/${encodeURIComponent(sessionId)}/inbox`, {
    filename,
    mime: file.type || 'image/png',
    data_b64,
  });
  return resp.path;
}

function blobToBase64(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const r = new FileReader();
    r.onerror = () => reject(r.error ?? new Error('read failed'));
    r.onload = () => {
      const s = String(r.result ?? '');
      resolve(s.slice(s.indexOf(',') + 1));
    };
    r.readAsDataURL(blob);
  });
}

/** Interrupt the agent's current turn: one Escape keystroke into its PTY —
 *  exactly what pressing Esc in the terminal does in Claude Code and Codex
 *  ("esc to interrupt"). `submit: false` writes the byte verbatim. */
export function interruptAgent(sessionId: string): Promise<void> {
  return api.post<void>(`/sessions/${encodeURIComponent(sessionId)}/input`, { text: '\u001b', submit: false });
}

/** Write raw keystrokes (e.g. `"y\r"`, `"a\r"`, `"n\r"`) into the session's PTY
 *  verbatim (`submit: false`) to answer an approval or interactive prompt. */
export function sendRawInput(sessionId: string, text: string): Promise<void> {
  return api.post<void>(`/sessions/${encodeURIComponent(sessionId)}/input`, { text, submit: false });
}

/** Interrupt the active turn (Esc), let the CLI settle out of streaming mode,
 *  and immediately submit a new prompt to redirect ("steer") the agent mid-turn. */
export async function steerAgent(sessionId: string, text: string): Promise<void> {
  await interruptAgent(sessionId);
  await new Promise<void>((resolve) => setTimeout(resolve, 120));
  await submitPrompt(sessionId, text);
}

/** Browse one workspace folder (including files) for `@file` mention typeahead. */
export function browseWorkspaceFiles(path: string, signal?: AbortSignal): Promise<FsBrowse> {
  return api.get<FsBrowse>(`/fs/browse?path=${encodeURIComponent(path)}&files=true`, signal);
}
