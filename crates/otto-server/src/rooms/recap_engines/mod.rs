//! Local recognition and subscription-authenticated draft summaries.
mod chunks;
mod process;
mod speech;
mod summary;
mod vision;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineConfig {
    pub whisper_executable: String,
    pub whisper_model: String,
    pub language: String,
    pub threads: u8,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpeechSegment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SummaryDraft {
    pub overview: String,
    pub decisions: Vec<String>,
    pub actions: Vec<String>,
    pub open_questions: Vec<String>,
    pub coverage: Vec<String>,
    pub source_event_ids: Vec<u64>,
}
pub use speech::{capabilities, transcribe, validate_wav};
pub use summary::summarize;
pub use vision::{jpeg_dimensions, recognize_screen, run_ocr_stdio};
