//! Built-in & extensible Text-to-Speech (TTS) engine:
//! - `macos_say` — built-in zero-setup offline macOS `/usr/bin/say` + `/usr/bin/afconvert`
//! - `openai_compat` — OpenAI-compatible `/v1/audio/speech`
//! - `kokoro` — Kokoro-FastAPI local/remote `/v1/audio/speech` + `/v1/audio/voices`
//! - `elevenlabs` — ElevenLabs `/v1/text-to-speech/{voice_id}` + `/v1/voices`
//! - `custom_http` — Custom HTTP TTS endpoint

pub mod http_drivers;
pub mod macos_say;

use std::sync::Arc;

use async_trait::async_trait;
use otto_core::Result;
use serde_json::Value;

use crate::types::{ProviderTestResult, TtsProviderRecord, TtsVisemeTimeline, TtsVoice};

#[derive(Debug, Clone)]
pub struct TtsSynthRequest {
    pub voice_id: String,
    pub text: String,
    pub speed: f32,
    pub format: String,
    pub model: Option<String>,
    pub voice_settings: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct TtsSynthOutput {
    pub bytes: Vec<u8>,
    pub format: String,
    pub mime: String,
    pub duration_ms: Option<i64>,
    pub visemes: Option<TtsVisemeTimeline>,
}

#[async_trait]
pub trait TtsDriver: Send + Sync {
    async fn list_voices(
        &self,
        provider: &TtsProviderRecord,
        api_key: Option<&str>,
    ) -> Result<Vec<TtsVoice>>;

    async fn synthesize(
        &self,
        req: &TtsSynthRequest,
        provider: &TtsProviderRecord,
        api_key: Option<&str>,
    ) -> Result<TtsSynthOutput>;

    async fn test_connection(
        &self,
        provider: &TtsProviderRecord,
        api_key: Option<&str>,
    ) -> Result<ProviderTestResult>;
}

pub fn driver_for_tts_kind(kind: &str) -> Arc<dyn TtsDriver> {
    match kind {
        "macos_say" => Arc::new(macos_say::MacOsSayDriver),
        "elevenlabs" => Arc::new(http_drivers::ElevenLabsDriver),
        "kokoro" => Arc::new(http_drivers::OpenAiCompatDriver { is_kokoro: true }),
        "openai_compat" => Arc::new(http_drivers::OpenAiCompatDriver { is_kokoro: false }),
        _ => Arc::new(http_drivers::CustomHttpTtsDriver),
    }
}

/// Best-effort WAV duration parser from RIFF header (`fmt ` + `data` chunks).
pub fn wav_duration_ms(bytes: &[u8]) -> Option<i64> {
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let mut pos = 12usize;
    let mut byte_rate: Option<u32> = None;
    let mut data_len: Option<u32> = None;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes([
            bytes[pos + 4],
            bytes[pos + 5],
            bytes[pos + 6],
            bytes[pos + 7],
        ]);
        let chunk_start = pos + 8;
        if id == b"fmt " && chunk_start + 16 <= bytes.len() {
            let br = u32::from_le_bytes([
                bytes[chunk_start + 8],
                bytes[chunk_start + 9],
                bytes[chunk_start + 10],
                bytes[chunk_start + 11],
            ]);
            if br > 0 {
                byte_rate = Some(br);
            }
        } else if id == b"data" {
            data_len = Some(size);
        }
        let step = (size as usize).saturating_add(size as usize & 1);
        pos = chunk_start.checked_add(step)?;
    }
    let (br, dl) = (byte_rate?, data_len?);
    Some(((u64::from(dl) * 1000) / u64::from(br)) as i64)
}
