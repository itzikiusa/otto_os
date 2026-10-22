// Contract mirror: notifications. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md + ws.md — change them together.

import type {
  Id,
} from './core';

// ---------------------------------------------------------------------------
// Notifications (notification center)
// ---------------------------------------------------------------------------

export type NoticeKind = 'credential' | 'session' | 'system';
export type NoticeSeverity = 'info' | 'warn' | 'error';

/** What the UI does when a notice's action button is clicked. */
export type NoticeAction =
  | { type: 'open_url'; url: string }
  | { type: 'open_session'; session_id: Id }
  /** `target` e.g. "claude" | "codex" | "git:<id>" | "issue:<id>". */
  | { type: 'reauth'; target: string }
  /** In-app route (hash path without `#/`) — automation notices:
   *  `workflows/<wf>/runs/<run>`, `scheduled-tasks/<task>/runs/<run>`,
   *  `loops/<loop>`, `personal-agents/<agent>/runs`. `workspace_id` is the
   *  workspace the target lives in (the UI switches to it first). */
  | { type: 'open_route'; route: string; workspace_id?: Id };

/** A persisted notification shown in the notification center. */
export interface Notice {
  id: Id;
  created_at: string;
  read: boolean;
  kind: NoticeKind;
  severity: NoticeSeverity;
  title: string;
  body: string;
  source_key: string | null;
  action: NoticeAction | null;
}

/** Reply of `POST /notifications/read` / `POST /notifications/dismiss`
 *  (`{ids}` ≤ 500): rows actually changed — foreign, global-for-non-root,
 *  unknown and already-read ids are skipped. */
export interface BulkNoticeResult {
  changed: number;
}

export interface NotificationSettings {
  expiry_threshold_days: number;
  native_enabled: boolean;
  session_events: boolean;
  /** Native banner for "Session awaiting input" when you are not watching
   *  that session (default true; older daemons omit it → treat as true). */
  native_on_waiting?: boolean;
}
