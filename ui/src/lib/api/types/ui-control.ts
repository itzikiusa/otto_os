// Contract mirror: ui-control. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md + ws.md — change them together.

import type {
  Id,
} from './core';

// ---------------------------------------------------------------------------
// Agent UI control — `otto.ui_*` tools driving this window (docs/contracts/
// ui-commands.json is the catalog; ws.md §2 the per-connection frames; api.md
// the result / progress / grant routes). Runtime: ui/src/lib/uiCommands.ts.
// ---------------------------------------------------------------------------

/** How much a UI command can change: `read` and `navigate` run once the session
 *  is granted; `local_write` confirms every time (with an "allow for this
 *  session" memory); `outward` always confirms and is never remembered. */
export type UiCommandRisk = 'read' | 'navigate' | 'local_write' | 'outward';

/** One `docs/contracts/ui-commands.json` entry (also `GET /ui/commands/catalog`). */
export interface UiCommandSpec {
  /** snake_case, unique; the tool is `otto.ui_<name>` (stdio `otto_ui_<name>`). */
  name: string;
  /** The paneKey (lib/sidePane.ts) of the document that runs it; `shell` = any document. */
  module: string;
  /** Hash route opened when no document shows `module` (`''` for shell commands). */
  route: string;
  risk: UiCommandRisk;
  /** Default deadline the daemon gives the UI (1000..=120000). */
  timeout_ms: number;
  /** Daemon-side fallback when no Otto window can run it (`read` only). */
  headless?: 'presence' | 'db_list_connections' | 'db_mcp_query';
  description: string;
  input_schema: Record<string, unknown>;
}

export interface UiCommandsCatalog {
  version: number;
  commands: UiCommandSpec[];
}

/** Query metadata shared by db_list_connections and Connections module state.
 * Tabs are scoped to their owning connection in this document; rows are only
 * returned by the separately capped/redacted db_get_result command. */
export interface UiDbTabSummary {
  tab_id: string;
  statement: string;
  running: boolean;
  has_result: boolean;
  by_agent: string | null;
}

/** Additive connection_tabs entry in the Connections module's ui_state result. */
export interface UiDbConnectionTabs {
  connection_id: Id;
  tabs: UiDbTabSummary[];
}

/** The agent a UI command comes from (attribution in the driven document). */
export interface UiAgentRef {
  session_id: Id;
  title: string;
  provider: string;
}

/** Logical pane identity: native child webview on desktop, iframe in browsers.
 * Detachment keeps the same identity and socket. */
export type UiPane = 'main' | 'side';

/** Client → server on `/ws/events`, once per socket open (and again when the
 *  registered command set changes): who this document is and what it can run. */
export interface UiHelloFrame {
  type: 'hello';
  /** Per-device id (`clientId()`, the value stamped as `session.meta.client_id`). */
  client_id: string;
  /** Logical window id (`lib/win.ts`); a detached side pane still reports its host. */
  window_id: string;
  pane: UiPane;
  /** Side pane only: original host, unchanged by native detachment. */
  host_window_id?: string;
  /** Current route without `#/`. */
  route: string;
  /** paneKey(route). */
  module: string;
  focused: boolean;
  visible: boolean;
  /** Registered UI command names (the catalog `name`s this document implements). */
  capabilities: string[];
}

/** Client → server, debounced 250 ms, on a route / focus / visibility change. */
export interface UiPresenceFrame {
  type: 'presence';
  route: string;
  module: string;
  focused: boolean;
  visible: boolean;
}

/** Server → this connection: the ephemeral id sent back as `X-Otto-Ui-Conn`. */
export interface UiHelloAckFrame {
  type: 'hello_ack';
  conn_id: string;
  /** This daemon process's boot id (changes on every daemon restart). */
  boot_id?: string;
  /** What this daemon's boot restore did to sessions (A4): re-adopted from
   *  their PTY holders (`kept_running`) vs live sessions that lost their
   *  process with the previous daemon (`suspended`). Null until restored. */
  boot_restore?: { kept_running: number; suspended: number } | null;
}

/** Server → this connection: run one UI command. */
export interface UiCommandFrame {
  type: 'ui_command';
  id: string;
  session_id: Id;
  agent: UiAgentRef;
  /** Catalog name (`db_run_query`). */
  command: string;
  args: Record<string, unknown>;
  /** How long the daemon waits for the result (ms). */
  deadline_ms: number;
}

/** Server → this connection: stop a command (deadline, Stop, grant revoked). */
export interface UiCommandCancelFrame {
  type: 'ui_command_cancel';
  id: string;
  reason: string;
}

export type UiServerFrame = UiHelloAckFrame | UiCommandFrame | UiCommandCancelFrame;
export type UiClientFrame = UiHelloFrame | UiPresenceFrame;

/** Error codes a UI handler reports back (`POST /ui/commands/{id}/result`). */
export type UiCommandErrorCode = 'cancelled_by_user' | 'invalid_args' | 'not_found' | 'forbidden' | 'failed';

/** Body for `POST /ui/commands/{id}/result` (header `X-Otto-Ui-Conn: <conn_id>`). */
export type UiCommandResultReq =
  | { ok: true; result: unknown }
  | { ok: false; error: { code: UiCommandErrorCode; message: string } };

/** Body for `POST /ui/commands/{id}/progress`. `awaiting_human` extends the
 *  daemon's deadline (≤ 120 s) while a confirm is open. */
export interface UiCommandProgressReq {
  note: string;
  awaiting_human: boolean;
}

/** Body for `POST /sessions/{id}/ui-control` (owner or admin, human credential). */
export interface UiControlReq {
  enabled: boolean;
}

/** `session.meta.ui_control` — server-owned (PATCH /sessions strips it). */
export interface UiControlGrant {
  enabled: boolean;
  granted_at?: string;
  granted_by?: Id;
}
