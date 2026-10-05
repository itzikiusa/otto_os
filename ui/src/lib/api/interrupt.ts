// Interrupt an agent session's current turn: one Escape keystroke into its PTY
// — what pressing Esc does in Claude Code / Codex ("esc to interrupt"). Shared
// by the in-module AI assists (Canvas Ask Otto, Browser Summarize, the Design
// Hall Otto panel) whose Stop button halts the backing session's turn without
// killing the session. Mirrors `interruptAgent` in agents/conversation/api.ts.
import { api } from './client';

export function interruptSession(sessionId: string): Promise<void> {
  return api.post<void>(`/sessions/${encodeURIComponent(sessionId)}/input`, { text: '\u001b', submit: false });
}
