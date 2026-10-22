// Contract mirror: browser. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md + ws.md — change them together.

import type {
  Id,
} from './core';

// ── Snips (screenshot → annotate → clipboard) ────────────────────────────────

/** A stored screenshot (file-backed under `data_dir/snips/`). Mirrors the Rust
 *  `Snip` DTO in `crates/otto-server/src/routes/snips.rs`. */
export interface Snip {
  id: Id;
  created_at: string;
  width: number;
  height: number;
  source: 'capture' | 'upload';
  /** Computed from the filesystem: a flattened annotated export exists. */
  has_annotated: boolean;
  /** Absolute path of the stored PNG **on the daemon's machine** — what an
   *  agent CLI needs to open it (the CLI runs next to the daemon, which is a
   *  different machine whenever Otto is driven from a remote browser).
   *  Recomputed server-side on every read. */
  path: string;
}

/** `POST /snips/capture` — long-polls while the native crosshair is on screen.
 *  Esc cancels cleanly (`cancelled:true`, no snip). */
export interface CaptureSnipResp {
  cancelled: boolean;
  snip?: Snip | null;
}

/** `POST /snips` legacy JSON body — base64 PNG upload. Current clients send a
 *  raw `image/png` body instead (no base64 inflation); both are accepted. */
export interface UploadSnipReq {
  data_b64: string;
  filename?: string;
}

/** Copy-style responses: whether the clipboard now holds the image (a `false`
 *  is a degraded success — the snip itself is saved). */
export interface SnipCopyResp {
  copied: boolean;
}

// ── Browser (reader/annotate tabs + on-demand page fetch) ───────────────────

/** A workspace-scoped browser tab. Mirrors `otto_state::browser::BrowserTab`. */
/** `POST /browser/proxy-ticket` — single-use ticket for the root-level
 *  `GET /browser/proxy?url=&ticket=` take-over frame (bound to `url`). */
export interface BrowserProxyTicket {
  ticket: string;
  expires_in_secs: number;
}

export interface BrowserTab {
  id: Id;
  workspace_id: Id;
  url: string;
  title: string;
  /** `"reader"` (fetched + rendered as markdown/HTML) or `"live"` (a real
   *  browser: the desktop WKWebView — engine `native` — or a daemon-owned
   *  Chromium streamed over WS — engine `remote`, see `BrowserLiveSession`).
   *  The engine is not stored on the tab row. */
  mode: 'reader' | 'live';
  created_at: string;
}

/** A DOM annotation, keyed on URL (survives the tab that made it being
 *  closed). Mirrors `otto_state::browser::BrowserAnnotation`. */
export interface BrowserAnnotation {
  id: Id;
  workspace_id: Id;
  /** Optional origin tab; creation returns 404 unless it belongs to this workspace. */
  tab_id: Id | null;
  url: string;
  selector: string;
  excerpt: string;
  text: string;
  comment: string;
  color: string;
  created_at: string;
}

/** `GET /workspaces/{wid}/browser/page?url=…` response — a fetched page,
 *  netguard-checked server-side before the fetch. */
export interface BrowserPage {
  url: string;
  title: string;
  markdown: string;
  /** Raw markup; `""` when fetched with `include_html=0` (the reader does). */
  html: string;
  /** `"lightpanda"` | `"fallback"` | `"mock"` — which engine produced this page. */
  engine: string;
  /** `true` when the plain-fetch fallback ran (no JS execution). */
  degraded: boolean;
}

/** One CSS-selector match returned by `GET /workspaces/{wid}/browser/query`.
 *  Mirrors `otto_browser::MatchedNode`. */
export interface BrowserMatchedNode {
  selector: string;
  outer_html: string;
  text: string;
}

/** `GET /workspaces/{wid}/browser/query?url=…&selector=…` response —
 *  netguard-checked and fetched the same way as `/page`, but returns only
 *  the nodes matching `selector` instead of the whole page. */
export interface BrowserQueryResp {
  matches: BrowserMatchedNode[];
}

/** `POST /workspaces/{wid}/browser/summarize` request — `{url}`. The daemon
 *  fetches it (netguard-checked) and runs one short-lived agent turn to
 *  summarize the (server-side, 30k-char-capped) markdown. */
export interface BrowserSummarizeReq {
  url: string;
}

/** `POST /workspaces/{wid}/browser/summarize` response. */
export interface BrowserSummarizeResp {
  summary: string;
  /** Which engine produced the underlying page fetch — see `BrowserPage.engine`. */
  engine: string;
  degraded: boolean;
}

/** `POST /workspaces/{wid}/browser/annotations/{id}/send` request. `session_id`
 *  must be a session in the SAME workspace as the route's `{wid}` — a 404
 *  otherwise. Writes a `[Browser mark] …` context block into that session's
 *  input (same effect as `POST /sessions/{id}/input` with `submit:true`). */
export interface BrowserSendAnnotationReq {
  session_id: Id;
}

/** `POST /workspaces/{wid}/browser/vault-save` request — `{url, vault_id}`.
 *  `summary` is optional: pass it (e.g. from a prior `/summarize` call) to
 *  skip a second page fetch; omitted, the daemon derives the note's summary
 *  section from a fresh fetch of `url`. */
export interface BrowserVaultSaveReq {
  url: string;
  vault_id: number;
  summary?: string;
}

/** `POST /workspaces/{wid}/browser/ask` body — the Browser page's embedded-
 *  agent ask bar. One submitted turn carrying the page (`url`), the marks
 *  the user wants the agent to see (`annotation_ids`, each fenced like a
 *  send-to-session block) and their question (`text`). The session and every
 *  annotation must belong to `wid`. Server caps: 8 000 chars of `text`, 20
 *  ids. */
export interface BrowserAskReq {
  session_id: Id;
  url: string;
  text: string;
  annotation_ids?: Id[];
}

/** `POST /workspaces/{wid}/browser/vault-save` response — the vault-relative
 *  path the note was written to (e.g. `browser/example-com-page.md`). */
export interface BrowserVaultSaveResp {
  note_path: string;
}

/** A stored site credential. Mirrors `otto_state::browser_credentials::
 *  BrowserCredential` — deliberately has NO password field; the secret lives
 *  only in the Keychain, keyed by the opaque `keychain_ref`. List/get
 *  responses always look like this; only `POST .../reveal` returns a
 *  password, as a separate `{password: string}` shape. */
export interface BrowserCredential {
  id: Id;
  workspace_id: Id;
  /** eTLD+1-ish, lowercased — see `match_domain` server-side doc comment. */
  domain: string;
  username: string;
  /** Opaque Keychain key. Never the password. */
  keychain_ref: string;
  /** Defaults `false` — an unattended agent session only autofills a
   *  credential the user has explicitly opted in. */
  allow_agent_use: boolean;
  notes: string;
  created_at: string;
  last_used_at: string | null;
}

/** `POST /workspaces/{wid}/browser/credentials` request. */
export interface BrowserCreateCredentialReq {
  domain: string;
  username: string;
  password: string;
  allow_agent_use?: boolean;
  notes?: string;
}

/** `PATCH /browser/credentials/{id}` request. An included `password` rotates
 *  the Keychain entry in place; the `keychain_ref` itself never changes. */
export interface BrowserPatchCredentialReq {
  username?: string;
  allow_agent_use?: boolean;
  notes?: string;
  password?: string;
}

/** `POST /browser/credentials/{id}/reveal` request — `confirm` MUST be
 *  `true` or the server rejects with 400. Gate this behind a UI confirm
 *  dialog too; the server-side check is belt-and-suspenders, not a
 *  substitute for it. */
export interface BrowserRevealCredentialReq {
  confirm: true;
}

/** `POST /browser/credentials/{id}/reveal` response — the only shape in this
 *  module that ever carries a plaintext password. */
export interface BrowserRevealCredentialResp {
  password: string;
}

/** `POST /workspaces/{wid}/browser/login` request — the governed agent
 *  sign-in. The server resolves the credential for `domain` server-side; the
 *  password never travels through this request or its response. */
export interface BrowserLoginReq {
  domain: string;
}

/** `POST /workspaces/{wid}/browser/login` response. `engine` is the backend
 *  that ran it (`"lightpanda"` normally — `"fallback"` never supports
 *  login(), since it doesn't run JS). */
export interface BrowserLoginResp {
  logged_in: boolean;
  engine: string;
}

// ── Browser — remote live view (daemon-owned Chromium) ──────────────────────
// Contract: docs/contracts/api.md "Browser — remote live view" + ws.md §1b.

/** Which engine renders a `mode:"live"` tab. `native` = the desktop app's
 *  WKWebView (desktop-only, no daemon state); `remote` = a daemon-owned
 *  Chromium streamed over `WS /ws/browser/{tab_id}/live` (desktop, PWA and
 *  remote web sessions). A property of the live session, not the tab row. */
export type BrowserLiveEngine = 'native' | 'remote';

/** The pluggable Chromium binary. `chrome` (default) = full Chrome for
 *  Testing in new headless mode (~180 MB download); `chrome-headless-shell` =
 *  the lighter headless-only shell (~98 MB). */
export type BrowserChromeBuild = 'chrome' | 'chrome-headless-shell';

export type BrowserEngineInstallState =
  | 'downloading'
  | 'verifying'
  | 'extracting'
  | 'installed'
  | 'failed';

export type BrowserLiveSessionState = 'starting' | 'ready' | 'crashed' | 'closed';

/** Who drives a live session. Agent actions pause while a human drives. */
export type BrowserLiveController = 'none' | 'human' | 'agent';

/** Daemon-wide remote-live settings (settings KV key `browser_live`).
 *  `PUT /browser/live/settings` takes a partial of this. */
export interface BrowserLiveSettings {
  /** Default `'chrome'`. */
  build: BrowserChromeBuild;
  /** "Show the window on this Mac" — launch Chrome with a visible window on
   *  the daemon host (still screencast-streamed). Default `false`; requires
   *  `build === 'chrome'`. */
  headed: boolean;
  /** Live sessions daemon-wide (1..=16, default 6). */
  max_sessions: number;
  /** Close a session with no viewer and no activity after this long
   *  (60..=86400, default 900). */
  idle_timeout_secs: number;
  /** Page-initiated downloads: refused, or saved (never opened) into the
   *  per-profile quarantine folder. Default `'quarantine'`. */
  downloads: 'block' | 'quarantine';
}

/** One pinned Chromium build as seen by `GET /browser/live/status`. */
export interface BrowserEngineBuildStatus {
  build: BrowserChromeBuild;
  /** Pinned Chrome for Testing version, e.g. `"149.0.7827.55"`. */
  version: string;
  platform: string;
  installed: boolean;
  /** Approximate download size in bytes (for the enable/download copy). */
  download_bytes: number;
  /** Human copy, e.g. `"Chrome for Testing — full browser (~180 MB)"`. */
  label: string;
  /** `false` when this daemon build ships no sha256 pin for it — install is
   *  refused (fail closed). */
  sha256_pinned: boolean;
  /** Where the binary lives when installed (or the `OTTO_CHROME_BIN` path). */
  path: string | null;
  /** `managed` = downloaded into the data dir; `env` = `OTTO_CHROME_BIN`. */
  source: 'managed' | 'env' | null;
}

/** The (single, daemon-wide) Chromium download job. */
export interface BrowserEngineInstallJob {
  build: BrowserChromeBuild;
  version: string;
  state: BrowserEngineInstallState;
  received_bytes: number;
  total_bytes: number | null;
  error: string | null;
  started_at: string;
  finished_at: string | null;
}

/** `GET /browser/live/status`. */
export interface BrowserLiveStatus {
  /** `false` off mac-arm64 — the remote engine can't be installed here. */
  platform_supported: boolean;
  builds: BrowserEngineBuildStatus[];
  settings: BrowserLiveSettings;
  /** The current/last install job this daemon run, if any. */
  install: BrowserEngineInstallJob | null;
  /** Running Chromium processes. */
  processes: number;
  /** Open live sessions (daemon-wide). */
  sessions: number;
}

/** `POST /browser/live/install` body. */
export interface BrowserEngineInstallReq {
  build?: BrowserChromeBuild;
}

/** Viewport in CSS px. */
export interface BrowserViewport {
  width: number;
  height: number;
  device_scale_factor?: number;
}

/** A tab's remote live session. Private to `owner_id` (plus ws Admin/root). */
export interface BrowserLiveSession {
  tab_id: Id;
  workspace_id: Id;
  owner_id: Id;
  engine: 'remote';
  build: BrowserChromeBuild;
  version: string;
  /** `'ephemeral'` (own incognito-like context, wiped on close) or a named
   *  persistent profile scoped to (workspace, owner, name). */
  profile: string;
  headed: boolean;
  state: BrowserLiveSessionState;
  url: string;
  title: string;
  loading: boolean;
  can_go_back: boolean;
  can_go_forward: boolean;
  viewport: Required<BrowserViewport>;
  controller: BrowserLiveController;
  controller_user_id: Id | null;
  /** Attached WS viewers. */
  viewers: number;
  created_at: string;
  last_activity_at: string;
}

/** `POST /browser/tabs/{id}/live` body. `engine` must be `'remote'` (a
 *  native tab needs no daemon session). `profile` matches `[a-z0-9_-]{1,40}`
 *  (default `'ephemeral'`). `url` defaults to the tab's url. */
export interface BrowserLiveCreateReq {
  engine?: 'remote';
  viewport?: BrowserViewport;
  profile?: string;
  url?: string;
}

export type BrowserLiveNavAction = 'goto' | 'back' | 'forward' | 'reload' | 'stop';

/** `POST /browser/tabs/{id}/live/nav` body — `url` required for `goto`. */
export interface BrowserLiveNavReq {
  action: BrowserLiveNavAction;
  url?: string;
}

/** `POST /browser/tabs/{id}/live/control` body. */
export interface BrowserLiveControlReq {
  action: 'take_over' | 'hand_back';
}

/** `POST /browser/tabs/{id}/live/screenshot` body — responds with the image
 *  bytes (`image/png` | `image/jpeg`), not JSON. */
export interface BrowserScreenshotReq {
  mode?: 'viewport' | 'full_page' | 'element';
  /** Required for `mode: 'element'`. */
  selector?: string;
  format?: 'png' | 'jpeg';
  /** JPEG quality 1..100. */
  quality?: number;
}

/** Header of a binary screencast frame on `WS /ws/browser/{tab_id}/live`:
 *  `[u8 version=1][u32 BE header length N][N bytes JSON header][image bytes]`. */
export interface BrowserLiveFrameHeader {
  seq: number;
  mime: 'image/jpeg';
  /** Image pixel size. */
  width: number;
  height: number;
  /** Viewport in CSS px — map pointer coords with these. */
  device_width: number;
  device_height: number;
  page_scale_factor: number;
  offset_top: number;
  scroll_x: number;
  scroll_y: number;
  timestamp: number;
}

/** CDP modifier bitmask: Alt=1, Ctrl=2, Meta=4, Shift=8. */
export type BrowserLiveModifiers = number;

/** Client → daemon frames on `WS /ws/browser/{tab_id}/live`. */
export type BrowserLiveClientMsg =
  | { type: 'ack'; seq: number }
  | {
      type: 'mouse';
      action: 'move' | 'down' | 'up' | 'wheel';
      x: number;
      y: number;
      button?: 'left' | 'middle' | 'right' | 'back' | 'forward' | 'none';
      buttons?: number;
      click_count?: number;
      delta_x?: number;
      delta_y?: number;
      modifiers?: BrowserLiveModifiers;
    }
  | {
      type: 'key';
      action: 'down' | 'up';
      key: string;
      code: string;
      text?: string;
      key_code?: number;
      location?: number;
      repeat?: boolean;
      modifiers?: BrowserLiveModifiers;
    }
  | { type: 'text'; text: string }
  | { type: 'ime'; text: string; selection_start: number; selection_end: number }
  | { type: 'paste'; text: string }
  | { type: 'nav'; action: BrowserLiveNavAction; url?: string }
  | { type: 'resize'; width: number; height: number; device_scale_factor?: number }
  | { type: 'control'; action: 'take_over' | 'hand_back' }
  | { type: 'dialog'; accept: boolean; prompt_text?: string };

/** Daemon → client JSON frames on `WS /ws/browser/{tab_id}/live` (binary
 *  frames are screencast images — see `BrowserLiveFrameHeader`). */
export type BrowserLiveServerMsg =
  | { type: 'state'; session: BrowserLiveSession }
  | { type: 'cursor'; cursor: string }
  | {
      type: 'dialog';
      dialog_type: 'alert' | 'confirm' | 'prompt' | 'beforeunload';
      message: string;
      default_prompt: string;
      url: string;
    }
  | { type: 'blocked'; host: string; reason: 'ssrf' }
  | { type: 'popup'; url: string }
  | { type: 'download'; status: 'blocked' | 'quarantined'; filename: string; bytes: number | null }
  | { type: 'approval'; approval_id: Id; status: 'pending' | 'approved' | 'denied'; title: string }
  | {
      type: 'error';
      code:
        | 'forbidden'
        | 'not_driver'
        | 'bad_frame'
        | 'nav_failed'
        | 'input_failed'
        | 'engine_unavailable';
      message: string;
    }
  | { type: 'closed'; reason: 'closed' | 'idle' | 'crashed' | 'revoked' | 'replaced' };
