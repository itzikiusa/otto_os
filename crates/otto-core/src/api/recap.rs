//! Owner-local room recap archives and consent. Mirrors UI room recap types.
use super::{RoomAnnotation, RoomMember, RoomMessage, RoomPresentation};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecapState {
    AwaitingConsent,
    Ready,
    Capturing,
    Finalizing,
    Paused,
    Stopped,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecapCapture {
    pub pending_jobs: u32,
    pub id: String,
    pub state: RecapState,
    pub epoch: u64,
    pub consented_member_ids: Vec<String>,
    pub reason: Option<String>,
    pub started_at: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateRecapReq {
    #[serde(default)]
    pub allow_unavailable_speech: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecapEngineSettings {
    pub whisper_executable: String,
    pub whisper_model: String,
    pub language: String,
    pub threads: u8,
}
impl Default for RecapEngineSettings {
    fn default() -> Self {
        Self {
            whisper_executable: String::new(),
            whisper_model: String::new(),
            language: "auto".into(),
            threads: 2,
        }
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecapSummaryStatus {
    Idle,
    Queued,
    Running,
    Ready,
    Error,
    Cancelled,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecapMetadata {
    pub id: String,
    pub owner_id: String,
    pub room_id: String,
    pub session_id: String,
    pub session_title: String,
    pub created_at: String,
    pub updated_at: String,
    pub status: RecapState,
    pub capture_epoch: u64,
    pub bytes_used: u64,
    pub quota_bytes: u64,
    pub last_seq: u64,
    pub speech_available: bool,
    pub speech_error: Option<String>,
    pub summary_status: RecapSummaryStatus,
    pub summary_error: Option<String>,
    pub summary_through_seq: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecapSpeechSegment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecapDraft {
    pub overview: String,
    pub decisions: Vec<String>,
    pub actions: Vec<String>,
    pub open_questions: Vec<String>,
    pub coverage: Vec<String>,
    pub source_event_ids: Vec<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecapEvent {
    pub seq: u64,
    pub created_at: String,
    pub capture_epoch: u64,
    pub payload: RecapEventData,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RecapEventData {
    Capture {
        state: RecapState,
        reason: Option<String>,
        started_at: Option<String>,
    },
    Participants {
        members: Vec<RoomMember>,
    },
    Chat {
        message: RoomMessage,
    },
    Terminal {
        data_base64: String,
    },
    Presentation {
        operation: String,
        presentation: RoomPresentation,
    },
    Annotation {
        annotation: RoomAnnotation,
    },
    AnnotationsCleared {
        source_id: String,
        clear_epoch: u64,
    },
    AnnotationRemoved {
        source_id: String,
        annotation_id: String,
    },
    AudioPending {
        member_id: String,
        member_name: String,
        sequence: u64,
        offset_ms: u64,
        duration_ms: u64,
    },
    Speech {
        member_id: String,
        member_name: String,
        sequence: u64,
        offset_ms: u64,
        segments: Vec<RecapSpeechSegment>,
    },
    Screen {
        source_id: String,
        source_generation: u64,
        title: String,
        image_id: String,
        offset_ms: u64,
        text: String,
    },
    Gap {
        kind: String,
        member_id: Option<String>,
        source_id: Option<String>,
        reason: String,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecapDetail {
    pub metadata: RecapMetadata,
    pub events: Vec<RecapEvent>,
    pub next_cursor: Option<u64>,
    pub draft: Option<RecapDraft>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecapAudioReq {
    pub capture_epoch: u64,
    pub member_id: String,
    pub member_generation: u64,
    pub sequence: u64,
    pub offset_ms: u64,
    pub duration_ms: u64,
    pub wav_base64: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecapScreenReq {
    pub capture_epoch: u64,
    pub source_id: String,
    pub source_generation: u64,
    pub sequence: u64,
    pub offset_ms: u64,
    pub jpeg_base64: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecapGapReq {
    pub capture_epoch: u64,
    pub kind: String,
    pub member_id: Option<String>,
    pub source_id: Option<String>,
    pub reason: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecapQueued {
    pub queued: bool,
}
