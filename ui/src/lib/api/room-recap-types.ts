import type {RoomAnnotation, RoomMember, RoomMessage, RoomPresentation} from './room-types';
export type RecapState = 'awaiting_consent' | 'ready' | 'capturing' | 'finalizing' | 'paused' | 'stopped';
export interface RecapCapture {id: string; state: RecapState; epoch: number; pending_jobs: number; consented_member_ids: string[]; reason: string | null; started_at: string | null}
export interface RecapEngineSettings {whisper_executable: string; whisper_model: string; language: string; threads: number}
export interface RecapCapabilities {speech_ready: boolean; executable_ready: boolean; model_ready: boolean; configuration_error: string | null; codex_subscription_ready: boolean; screen_text_ready: boolean; languages: string[]; setup: string}
export type RecapSummaryStatus = 'idle' | 'queued' | 'running' | 'ready' | 'error' | 'cancelled';
export interface RecapMetadata {
  id: string; owner_id: string; room_id: string; session_id: string; session_title: string;
  created_at: string; updated_at: string; status: RecapState; capture_epoch: number;
  bytes_used: number; quota_bytes: number; last_seq: number; speech_available: boolean;
  speech_error: string | null; summary_status: RecapSummaryStatus; summary_error: string | null;
  summary_through_seq: number | null;
}
/** Cheap owner-local metadata: opaque revisions never contain event/draft bodies. */
export interface RecapRevision {metadata: RecapMetadata; events_revision: string; draft_revision: string | null}
export interface RecapSpeechSegment {start_ms: number; end_ms: number; text: string}
export interface RecapDraft {overview: string; decisions: string[]; actions: string[]; open_questions: string[]; coverage: string[]; source_event_ids: number[]}
export type RecapEventData =
  | {type: 'capture'; state: RecapState; reason: string | null; started_at: string | null}
  | {type: 'participants'; members: RoomMember[]}
  | {type: 'chat'; message: RoomMessage}
  | {type: 'terminal'; data_base64: string}
  | {type: 'presentation'; operation: string; presentation: RoomPresentation}
  | {type: 'annotation'; annotation: RoomAnnotation}
  | {type: 'annotations_cleared'; source_id: string; clear_epoch: number}
  | {type: 'annotation_removed'; source_id: string; annotation_id: string}
  | {type: 'audio_pending'; member_id: string; member_name: string; sequence: number; offset_ms: number; duration_ms: number}
  | {type: 'speech'; member_id: string; member_name: string; sequence: number; offset_ms: number; segments: RecapSpeechSegment[]}
  | {type: 'screen'; source_id: string; source_generation: number; title: string; image_id: string; offset_ms: number; text: string}
  | {type: 'gap'; kind: string; member_id: string | null; source_id: string | null; reason: string};
export interface RecapEvent {seq: number; created_at: string; capture_epoch: number; payload: RecapEventData}
export interface RecapDetail {metadata: RecapMetadata; events: RecapEvent[]; next_cursor: number | null; draft: RecapDraft | null}
export interface RecapAudioReq {capture_epoch: number; member_id: string; member_generation: number; sequence: number; offset_ms: number; duration_ms: number; wav_base64: string}
export interface RecapScreenReq {capture_epoch: number; source_id: string; source_generation: number; sequence: number; offset_ms: number; jpeg_base64: string}
export interface RecapGapReq {capture_epoch: number; kind: string; member_id?: string | null; source_id?: string | null; reason: string}
