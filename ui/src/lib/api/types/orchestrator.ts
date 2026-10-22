// Contract mirror: orchestrator. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md + ws.md — change them together.

import type {
  Id,
} from './core';

// ---------------------------------------------------------------------------
// Orchestrator
// ---------------------------------------------------------------------------

export type Action =
  | { action: 'spawn_sessions'; provider: string; count: number }
  | { action: 'broadcast'; text: string }
  | { action: 'open_connection'; connection_id: Id }
  | { action: 'run_command'; session_id: Id; text: string };

export type ActionPlan = Action[];

/** POST /workspaces/{id}/broadcast — dedicated, AI-free relay to sessions. */
export interface BroadcastReq {
  text: string;
  /** Target sessions; omit/empty to hit every live agent session. */
  session_ids?: Id[];
}

export interface BroadcastResp {
  /** The sessions that actually received the message. */
  session_ids: Id[];
}

/** POST /workspaces/{id}/relay — deliver a NAME-ADDRESSED message
 *  ("ronaldo: do X", "ronaldo, messi: ship it", "all: stand down"). */
export interface RelayReq {
  text: string;
}

export interface RelayResp {
  /** Sessions the message was delivered to (empty when unaddressed). */
  session_ids: Id[];
  /** True when the address was a broadcast keyword (all/everyone). */
  broadcast: boolean;
  /** True when no session was named — the caller should fall back. */
  unaddressed: boolean;
  /** The message actually sent (address prefix stripped). */
  text: string;
}

// --- Session name themes (auto-naming new sessions: "Ronaldo", "Messi", …) ----

/** One selectable name theme (built-in or a user's custom list). */
export interface NameThemeInfo {
  /** Built-in id ("footballers") or a custom theme's id. */
  id: string;
  label: string;
  /** "builtin" | "custom". */
  kind: string;
  /** How many distinct names the theme can yield. */
  capacity: number;
  /** A few example names for the picker preview. */
  sample: string[];
}

/** GET /name-themes */
export interface NameThemesResp {
  themes: NameThemeInfo[];
  /** Active theme id: a built-in id, a custom id, or "none" (legacy numbering). */
  active: string;
}

/** PUT /name-themes/active */
export interface SetActiveThemeReq {
  theme_id: string;
}

/** POST /name-themes */
export interface CreateNameThemeReq {
  label: string;
  names: string[];
}

/** PUT /name-themes/{id} */
export interface UpdateNameThemeReq {
  label: string;
  names: string[];
}

/** A custom name theme as returned to its owner. */
export interface CustomThemeResp {
  id: Id;
  label: string;
  names: string[];
}

export interface OrchestrateReq {
  text: string;
  optimize: boolean;
  ai_fallback: boolean;
  focused_session_id?: Id | null;
}

export interface OrchestrateResp {
  plan: ActionPlan;
  optimized_text: string | null;
}

export interface ExecutePlanReq {
  plan: ActionPlan;
}

export interface ExecuteResult {
  action_index: number;
  ok: boolean;
  detail: string;
  session_ids: Id[];
}
