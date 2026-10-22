// Shell-owned stores: imported statically (the sidebar, status bar and session
// view read them from the first frame), so these routes never lazy-load.
import { ws } from '../../stores/workspace.svelte';
import { notifications } from '../../stores/notifications.svelte';
import { activity } from '../../stores/activity.svelte';
import { proof } from '../../stores/proof.svelte';
import { transcript } from '../../stores/transcript.svelte';
import { git } from '../../stores/git.svelte';
import { assistant } from '../../stores/assistant.svelte';
import { uiControl } from '../../stores/uiControl.svelte';
import type { EventOf, EventRoutes } from './types';

/** The workspace store's own events (`ws.applyEvent` switches on them). */
const toWorkspace = (ev: EventOf<'session_status' | 'session_created' | 'session_archive_changed' | 'session_renamed' | 'session_attention' | 'notice'>): void => {
  ws.applyEvent(ev);
};

type TranscriptEvent = EventOf<'transcript_appended' | 'transcript_live' | 'artifact_added' | 'history_index_progress'>;
/** Conversation view: live tail deltas (+ artifact chips) for the open
 *  session's chat; History page index progress. The activity store also takes
 *  the artifact / index events (Outputs panel + rescan bar). */
const toTranscript = (ev: TranscriptEvent): void => {
  transcript.applyEvent(ev);
  if (ev.type !== 'transcript_appended' && ev.type !== 'transcript_live') activity.applyEvent(ev);
};

/** Assistant: thread turns/cards, task board, needs-you badge, limits. */
const toAssistant = (ev: EventOf<'assistant_turn' | 'assistant_task_update' | 'assistant_needs_you' | 'assistant_limit'>): void => {
  assistant.applyEvent(ev);
};

const toActivity = (ev: EventOf<'trail_appended' | 'tasks_updated'>): void => {
  activity.applyEvent(ev);
};

export const shellRoutes: EventRoutes = {
  notification: (ev) => {
    notifications.ingest(ev.notice);
    // A "waiting"/blocked notice (Claude's Notification hook) means a session
    // is blocked on the operator — raise the sticky "needs you" flag, distinct
    // from plain idle. Keyed off the stable source_key.
    const n = ev.notice;
    if (n.source_key?.endsWith(':waiting') && n.action?.type === 'open_session') {
      const at = Date.parse(n.created_at);
      ws.markNeedsYou(n.action.session_id, { noticeAt: Number.isFinite(at) ? at : undefined });
    }
  },
  session_status: toWorkspace,
  session_created: toWorkspace,
  session_archive_changed: toWorkspace,
  session_renamed: toWorkspace,
  session_attention: toWorkspace,
  notice: toWorkspace,
  // The UI-control grant lives in session meta; a removed session drops its prompt.
  session_meta_updated: (ev) => {
    uiControl.applyEvent(ev);
    ws.applyEvent(ev);
  },
  session_removed: (ev) => {
    activity.forget(ev.session_id);
    uiControl.applyEvent(ev);
    ws.applyEvent(ev);
  },
  trail_appended: toActivity,
  tasks_updated: toActivity,
  transcript_appended: toTranscript,
  transcript_live: toTranscript,
  artifact_added: toTranscript,
  history_index_progress: toTranscript,
  assistant_turn: toAssistant,
  assistant_task_update: toAssistant,
  assistant_needs_you: toAssistant,
  assistant_limit: toAssistant,
  // An agent asked to drive the UI: the session's pane shows the prompt.
  ui_control_requested: (ev) => uiControl.applyEvent(ev),
  // A watched repo changed on disk: the Git page re-reads its status.
  repo_status_changed: (ev) => git.applyRepoChanged(ev.repo_id, ev.paths),
  // Proof page list/detail + sidebar proof chips refresh on the event.
  proof_pack_updated: (ev) => proof.applyEvent(ev),
};
