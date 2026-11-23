//! Extensible HTTP TTS drivers (`openai_compat`, `kokoro`, `elevenlabs`, `custom_http`).
//! All outbound calls are SSRF-guarded through `crate::net` (`otto-netguard`).

use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::Engine;
use otto_core::{Error, Result};
use serde_json::{json, Value};

use super::{wav_duration_ms, TtsDriver, TtsSynthOutput, TtsSynthRequest};
use crate::net::{apply_auth_header, build_http_client, read_bounded_bytes, validate_endpoint_url};
use crate::types::{
    tts_mime_for_format, ProviderTestResult, TtsProviderRecord, TtsVisemeTimeline, TtsVoice,
    MAX_TTS_AUDIO_BYTES,
};

const HTTP_TTS_TIMEOUT: Duration = Duration::from_secs(90);
const VOICE_FETCH_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_JSON_BYTES: usize = 512 * 1024;
const VISEMES_TIMEOUT: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------------------
// OpenAI-compatible & Kokoro-FastAPI
// ---------------------------------------------------------------------------

pub struct OpenAiCompatDriver {
    pub is_kokoro: bool,
}

fn default_openai_voices(provider_id: &str) -> Vec<TtsVoice> {
    [
        "alloy", "ash", "coral", "echo", "fable", "nova", "onyx", "sage", "shimmer",
    ]
    .iter()
    .map(|v| TtsVoice {
        id: (*v).into(),
        name: capitalize(v),
        locale: Some("en_US".into()),
        gender: None,
        preview_text: None,
        provider_id: provider_id.into(),
    })
    .collect()
}

fn default_kokoro_voices(provider_id: &str) -> Vec<TtsVoice> {
    [
        ("af_heart", "Heart (US Female)", "en_US", "female"),
        ("af_bella", "Bella (US Female)", "en_US", "female"),
        ("am_adam", "Adam (US Male)", "en_US", "male"),
        ("am_michael", "Michael (US Male)", "en_US", "male"),
        ("bf_emma", "Emma (UK Female)", "en_GB", "female"),
        ("bm_george", "George (UK Male)", "en_GB", "male"),
    ]
    .iter()
    .map(|(id, name, loc, g)| TtsVoice {
        id: (*id).into(),
        name: (*name).into(),
        locale: Some((*loc).into()),
        gender: Some((*g).into()),
        preview_text: None,
        provider_id: provider_id.into(),
    })
    .collect()
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(first) => format!("{}{}", first.to_uppercase(), c.as_str()),
        None => String::new(),
    }
}

fn speech_endpoint(base: &reqwest::Url) -> Result<reqwest::Url> {
    let path = base.path().trim_end_matches('/');
    if path.ends_with("/audio/speech") || path.ends_with("/speech") {
        Ok(base.clone())
    } else if path.ends_with("/v1") {
        base.join("audio/speech")
            .map_err(|e| Error::Invalid(format!("speech url: {e}")))
    } else {
        base.join("v1/audio/speech")
            .map_err(|e| Error::Invalid(format!("speech url: {e}")))
    }
}

fn voices_endpoint(base: &reqwest::Url) -> Option<reqwest::Url> {
    let path = base.path().trim_end_matches('/');
    if path.ends_with("/v1") {
        base.join("audio/voices").ok()
    } else {
        base.join("v1/audio/voices").ok()
    }
}

/// `GET <endpoint>/v1/audio/voices`. A server without one (404/405) or with
/// no list lists nothing and the driver offers its defaults; an unreachable
/// or failing server is an error, so no default list is cached in its place.
async fn fetch_listed_voices(
    raw: &str,
    provider: &TtsProviderRecord,
    api_key: Option<&str>,
) -> Result<Vec<TtsVoice>> {
    let has_creds = api_key.is_some_and(|k| !k.trim().is_empty());
    let base = validate_endpoint_url(raw, has_creds, provider.allow_local_network)?;
    let Some(url) = voices_endpoint(&base) else {
        return Ok(Vec::new());
    };
    let client = build_http_client(provider.allow_local_network, VOICE_FETCH_TIMEOUT)?;
    let resp = apply_auth_header(client.get(url), api_key, provider.auth_header.as_deref())
        .send()
        .await
        .map_err(|e| Error::Upstream(format!("TTS voices request failed: {e}")))?;
    let status = resp.status().as_u16();
    if matches!(status, 404 | 405) {
        return Ok(Vec::new());
    }
    if !resp.status().is_success() {
        return Err(Error::Upstream(format!(
            "TTS voices returned HTTP {status}"
        )));
    }
    let bytes = read_bounded_bytes(resp, MAX_JSON_BYTES).await?;
    let Ok(val) = serde_json::from_slice::<Value>(&bytes) else {
        return Ok(Vec::new());
    };
    let Some(arr) = val
        .get("voices")
        .and_then(|v| v.as_array())
        .or_else(|| val.as_array())
    else {
        return Ok(Vec::new());
    };
    Ok(arr
        .iter()
        .filter_map(|item| {
            if let Some(s) = item.as_str() {
                return Some(TtsVoice {
                    id: s.to_string(),
                    name: s.to_string(),
                    locale: None,
                    gender: None,
                    preview_text: None,
                    provider_id: provider.id.clone(),
                });
            }
            let id = item
                .get("id")
                .or_else(|| item.get("voice_id"))
                .and_then(|v| v.as_str())?;
            let text = |k: &str| item.get(k).and_then(|v| v.as_str()).map(str::to_string);
            Some(TtsVoice {
                id: id.to_string(),
                name: text("name").unwrap_or_else(|| id.to_string()),
                locale: text("locale"),
                gender: text("gender"),
                preview_text: None,
                provider_id: provider.id.clone(),
            })
        })
        .collect())
}

#[async_trait]
impl TtsDriver for OpenAiCompatDriver {
    async fn list_voices(
        &self,
        provider: &TtsProviderRecord,
        api_key: Option<&str>,
    ) -> Result<Vec<TtsVoice>> {
        if let Some(raw) = provider
            .endpoint_url
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            let listed = fetch_listed_voices(raw, provider, api_key).await?;
            if !listed.is_empty() {
                return Ok(listed);
            }
        }
        Ok(if self.is_kokoro {
            default_kokoro_voices(&provider.id)
        } else {
            default_openai_voices(&provider.id)
        })
    }

    async fn synthesize(
        &self,
        req: &TtsSynthRequest,
        provider: &TtsProviderRecord,
        api_key: Option<&str>,
    ) -> Result<TtsSynthOutput> {
        let raw = provider
            .endpoint_url
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| Error::Invalid("TTS provider has no endpoint_url configured".into()))?;
        let has_creds = api_key.is_some_and(|k| !k.trim().is_empty());
        let base = validate_endpoint_url(raw, has_creds, provider.allow_local_network)?;
        let url = speech_endpoint(&base)?;
        let client = build_http_client(provider.allow_local_network, HTTP_TTS_TIMEOUT)?;

        let model = req
            .model
            .as_deref()
            .or(provider.default_model.as_deref())
            .unwrap_or(if self.is_kokoro { "kokoro" } else { "tts-1" });
        let response_format = match req.format.as_str() {
            "m4a" => "aac",
            other => other,
        };
        let payload = json!({
            "model": model,
            "input": req.text,
            "voice": req.voice_id,
            "response_format": response_format,
            "speed": req.speed.clamp(0.5, 2.0),
        });
        let mut post = client.post(url.clone()).json(&payload);
        if self.is_kokoro {
            // Viseme contract with the Forge voice server: ask it to align the clip.
            post = post.header("X-Otto-Visemes", "1");
        }
        let resp = apply_auth_header(post, api_key, provider.auth_header.as_deref())
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("TTS HTTP request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let err_body = read_bounded_bytes(resp, 16 * 1024)
                .await
                .unwrap_or_default();
            return Err(Error::Upstream(format!(
                "TTS provider returned HTTP {status}: {}",
                String::from_utf8_lossy(&err_body)
                    .chars()
                    .take(300)
                    .collect::<String>()
            )));
        }

        let visemes_id = resp
            .headers()
            .get("x-otto-visemes-id")
            .and_then(|v| v.to_str().ok())
            .filter(|_| self.is_kokoro)
            .map(str::to_string);
        let bytes = read_bounded_bytes(resp, MAX_TTS_AUDIO_BYTES).await?;
        if bytes.is_empty() {
            return Err(Error::Upstream("TTS provider returned empty audio".into()));
        }
        let visemes = match visemes_id {
            Some(id) => fetch_visemes(&client, &url, &id, api_key, provider).await,
            None => None,
        };
        let duration_ms = if req.format == "wav" {
            wav_duration_ms(&bytes)
        } else {
            None
        };
        let mime = tts_mime_for_format(&req.format)
            .unwrap_or("audio/mpeg")
            .to_string();
        Ok(TtsSynthOutput {
            bytes,
            format: req.format.clone(),
            mime,
            duration_ms,
            visemes,
        })
    }

    async fn test_connection(
        &self,
        provider: &TtsProviderRecord,
        api_key: Option<&str>,
    ) -> Result<ProviderTestResult> {
        test_http_tts_endpoint(provider, api_key).await
    }
}

/// Fetch the timeline a kokoro server aligned for the clip it just made:
/// `GET <speech url's parent>/visemes/<id>` (`/v1/audio/visemes/<id>`, same
/// origin and client as the speech call, ≤ 512 KiB, 10 s). Rhubarb shape:
/// `{duration_sec, cues: [{start, end, value: A–H | X}]}`. Any failure or a
/// malformed timeline leaves the clip without visemes.
async fn fetch_visemes(
    client: &reqwest::Client,
    speech_url: &reqwest::Url,
    id: &str,
    api_key: Option<&str>,
    provider: &TtsProviderRecord,
) -> Option<TtsVisemeTimeline> {
    // An opaque id, but only one path segment: it can never leave the origin.
    let id_ok = !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !id_ok {
        return None;
    }
    let parent = speech_url
        .path()
        .trim_end_matches('/')
        .rsplit_once('/')
        .map_or("", |(p, _)| p);
    let mut url = speech_url.clone();
    url.set_path(&format!("{parent}/visemes/{id}"));
    url.set_query(None);
    let resp = apply_auth_header(
        client.get(url).timeout(VISEMES_TIMEOUT),
        api_key,
        provider.auth_header.as_deref(),
    )
    .send()
    .await
    .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let bytes = read_bounded_bytes(resp, MAX_JSON_BYTES).await.ok()?;
    let timeline: TtsVisemeTimeline = serde_json::from_slice(&bytes).ok()?;
    let valid = timeline.duration_sec.is_finite()
        && timeline.duration_sec >= 0.0
        && timeline.cues.iter().all(|c| {
            c.start.is_finite()
                && c.end.is_finite()
                && 0.0 <= c.start
                && c.start <= c.end
                && matches!(
                    c.value.as_str(),
                    "A" | "B" | "C" | "D" | "E" | "F" | "G" | "H" | "X"
                )
        });
    valid.then_some(timeline)
}

// ---------------------------------------------------------------------------
// ElevenLabs
// ---------------------------------------------------------------------------

pub struct ElevenLabsDriver;

const DEFAULT_ELEVENLABS_BASE: &str = "https://api.elevenlabs.io";

#[async_trait]
impl TtsDriver for ElevenLabsDriver {
    async fn list_voices(
        &self,
        provider: &TtsProviderRecord,
        api_key: Option<&str>,
    ) -> Result<Vec<TtsVoice>> {
        let Some(key) = api_key.filter(|k| !k.trim().is_empty()) else {
            return Ok(Vec::new());
        };
        let raw = provider
            .endpoint_url
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(DEFAULT_ELEVENLABS_BASE);
        let base = validate_endpoint_url(raw, true, provider.allow_local_network)?;
        let url = base
            .join("v1/voices")
            .map_err(|e| Error::Invalid(format!("elevenlabs voices url: {e}")))?;
        let client = build_http_client(provider.allow_local_network, VOICE_FETCH_TIMEOUT)?;
        let resp = client
            .get(url)
            .header("xi-api-key", key)
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("ElevenLabs voices request failed: {e}")))?;
        if !resp.status().is_success() {
            return Err(Error::Upstream(format!(
                "ElevenLabs voices returned HTTP {}",
                resp.status().as_u16()
            )));
        }
        let bytes = read_bounded_bytes(resp, MAX_JSON_BYTES).await?;
        let val: Value = serde_json::from_slice(&bytes)
            .map_err(|e| Error::Upstream(format!("ElevenLabs voices JSON: {e}")))?;
        let voices = val
            .get("voices")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| {
                        let id = v.get("voice_id")?.as_str()?.to_string();
                        let name = v
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or(&id)
                            .to_string();
                        let labels = v.get("labels");
                        let gender = labels
                            .and_then(|l| l.get("gender"))
                            .and_then(|g| g.as_str())
                            .map(str::to_string);
                        let locale = labels
                            .and_then(|l| l.get("accent"))
                            .and_then(|a| a.as_str())
                            .map(str::to_string);
                        Some(TtsVoice {
                            id,
                            name,
                            locale,
                            gender,
                            preview_text: None,
                            provider_id: provider.id.clone(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(voices)
    }

    async fn synthesize(
        &self,
        req: &TtsSynthRequest,
        provider: &TtsProviderRecord,
        api_key: Option<&str>,
    ) -> Result<TtsSynthOutput> {
        let key = api_key
            .filter(|k| !k.trim().is_empty())
            .ok_or_else(|| Error::Invalid("ElevenLabs requires an API key in Keychain".into()))?;
        let raw = provider
            .endpoint_url
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(DEFAULT_ELEVENLABS_BASE);
        let base = validate_endpoint_url(raw, true, provider.allow_local_network)?;
        if req
            .voice_id
            .chars()
            .any(|c| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
        {
            return Err(Error::Invalid("invalid ElevenLabs voice_id".into()));
        }
        let mut url = base
            .join(&format!("v1/text-to-speech/{}", req.voice_id))
            .map_err(|e| Error::Invalid(format!("elevenlabs tts url: {e}")))?;
        let fmt = if req.format == "wav" { "wav" } else { "mp3" };
        if fmt == "wav" {
            url.set_query(Some("output_format=pcm_22050"));
        }
        let client = build_http_client(provider.allow_local_network, HTTP_TTS_TIMEOUT)?;
        let model_id = req
            .model
            .as_deref()
            .or(provider.default_model.as_deref())
            .unwrap_or("eleven_multilingual_v2");
        let mut payload = json!({
            "text": req.text,
            "model_id": model_id,
        });
        if let Some(vs) = &req.voice_settings {
            payload["voice_settings"] = vs.clone();
        }
        let resp = client
            .post(url)
            .header("xi-api-key", key)
            .json(&payload)
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("ElevenLabs synthesis failed: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let err_body = read_bounded_bytes(resp, 16 * 1024)
                .await
                .unwrap_or_default();
            return Err(Error::Upstream(format!(
                "ElevenLabs returned HTTP {status}: {}",
                String::from_utf8_lossy(&err_body)
                    .chars()
                    .take(300)
                    .collect::<String>()
            )));
        }
        let bytes = read_bounded_bytes(resp, MAX_TTS_AUDIO_BYTES).await?;
        if bytes.is_empty() {
            return Err(Error::Upstream("ElevenLabs returned empty audio".into()));
        }
        // `pcm_22050` is headerless 16-bit mono PCM: the `.wav` gets its RIFF header.
        let (bytes, duration_ms) = if fmt == "wav" {
            let wav = pcm16_mono_wav(bytes, 22_050);
            let ms = wav_duration_ms(&wav);
            (wav, ms)
        } else {
            (bytes, None)
        };
        let mime = tts_mime_for_format(fmt).unwrap_or("audio/mpeg").to_string();
        Ok(TtsSynthOutput {
            bytes,
            format: fmt.to_string(),
            mime,
            duration_ms,
            visemes: None,
        })
    }

    async fn test_connection(
        &self,
        provider: &TtsProviderRecord,
        api_key: Option<&str>,
    ) -> Result<ProviderTestResult> {
        let started = Instant::now();
        let voices = self.list_voices(provider, api_key).await?;
        Ok(ProviderTestResult {
            ok: !voices.is_empty(),
            latency_ms: Some(started.elapsed().as_millis() as u64),
            message: if voices.is_empty() {
                "No voices returned (verify API key)".into()
            } else {
                format!("Connected ({} voices available)", voices.len())
            },
            detected_version: None,
            details: None,
        })
    }
}

/// A 44-byte RIFF/WAVE header (PCM, mono, 16-bit) in front of raw samples.
fn pcm16_mono_wav(pcm: Vec<u8>, sample_rate: u32) -> Vec<u8> {
    let data_len = pcm.len() as u32;
    let mut wav = Vec::with_capacity(44 + pcm.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    wav.extend_from_slice(&2u16.to_le_bytes()); // block align
    wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.extend(pcm);
    wav
}

// ---------------------------------------------------------------------------
// Custom HTTP TTS
// ---------------------------------------------------------------------------

pub struct CustomHttpTtsDriver;

#[async_trait]
impl TtsDriver for CustomHttpTtsDriver {
    async fn list_voices(
        &self,
        provider: &TtsProviderRecord,
        _api_key: Option<&str>,
    ) -> Result<Vec<TtsVoice>> {
        let voice = provider
            .default_voice
            .clone()
            .unwrap_or_else(|| "default".into());
        Ok(vec![TtsVoice {
            id: voice.clone(),
            name: voice,
            locale: None,
            gender: None,
            preview_text: None,
            provider_id: provider.id.clone(),
        }])
    }

    async fn synthesize(
        &self,
        req: &TtsSynthRequest,
        provider: &TtsProviderRecord,
        api_key: Option<&str>,
    ) -> Result<TtsSynthOutput> {
        let raw = provider
            .endpoint_url
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| Error::Invalid("custom TTS provider has no endpoint_url".into()))?;
        let has_creds = api_key.is_some_and(|k| !k.trim().is_empty());
        let url = validate_endpoint_url(raw, has_creds, provider.allow_local_network)?;
        let client = build_http_client(provider.allow_local_network, HTTP_TTS_TIMEOUT)?;

        let payload = json!({
            "text": req.text,
            "voice": req.voice_id,
            "speed": req.speed.clamp(0.5, 2.0),
            "format": req.format,
            "model": req.model.as_deref().or(provider.default_model.as_deref()),
        });
        let resp = apply_auth_header(
            client.post(url).json(&payload),
            api_key,
            provider.auth_header.as_deref(),
        )
        .send()
        .await
        .map_err(|e| Error::Upstream(format!("custom TTS request failed: {e}")))?;
        let status = resp.status();
        let ct = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !status.is_success() {
            let err_body = read_bounded_bytes(resp, 16 * 1024)
                .await
                .unwrap_or_default();
            return Err(Error::Upstream(format!(
                "custom TTS returned HTTP {}: {}",
                status.as_u16(),
                String::from_utf8_lossy(&err_body)
                    .chars()
                    .take(300)
                    .collect::<String>()
            )));
        }
        let (bytes, visemes) = if ct.contains("application/json") {
            let raw_json =
                read_bounded_bytes(resp, MAX_TTS_AUDIO_BYTES * 4 / 3 + 64 * 1024).await?;
            let val: Value = serde_json::from_slice(&raw_json)
                .map_err(|e| Error::Upstream(format!("custom TTS JSON parse failed: {e}")))?;
            let b64 = val
                .get("audio_b64")
                .or_else(|| val.get("audio"))
                .or_else(|| val.get("data"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    Error::Upstream("custom TTS JSON lacked audio_b64/audio/data".into())
                })?;
            let clean = b64.split_once(',').map(|(_, r)| r).unwrap_or(b64);
            let audio = base64::engine::general_purpose::STANDARD
                .decode(clean.trim())
                .map_err(|e| Error::Upstream(format!("custom TTS invalid base64: {e}")))?;
            let vis = val
                .get("visemes")
                .cloned()
                .and_then(|v| serde_json::from_value::<TtsVisemeTimeline>(v).ok());
            (audio, vis)
        } else {
            (read_bounded_bytes(resp, MAX_TTS_AUDIO_BYTES).await?, None)
        };
        if bytes.is_empty() {
            return Err(Error::Upstream("custom TTS returned empty audio".into()));
        }
        let duration_ms = if req.format == "wav" {
            wav_duration_ms(&bytes)
        } else {
            None
        };
        let mime = tts_mime_for_format(&req.format)
            .unwrap_or("audio/wav")
            .to_string();
        Ok(TtsSynthOutput {
            bytes,
            format: req.format.clone(),
            mime,
            duration_ms,
            visemes,
        })
    }

    async fn test_connection(
        &self,
        provider: &TtsProviderRecord,
        api_key: Option<&str>,
    ) -> Result<ProviderTestResult> {
        test_http_tts_endpoint(provider, api_key).await
    }
}

async fn test_http_tts_endpoint(
    provider: &TtsProviderRecord,
    api_key: Option<&str>,
) -> Result<ProviderTestResult> {
    let started = Instant::now();
    let Some(raw) = provider
        .endpoint_url
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    else {
        return Ok(ProviderTestResult {
            ok: false,
            latency_ms: Some(0),
            message: "No endpoint_url configured".into(),
            detected_version: None,
            details: None,
        });
    };
    let has_creds = api_key.is_some_and(|k| !k.trim().is_empty());
    let url = validate_endpoint_url(raw, has_creds, provider.allow_local_network)?;
    let client = build_http_client(provider.allow_local_network, Duration::from_secs(10))?;
    let probe_url = url.join("/health").unwrap_or_else(|_| url.clone());
    let res = apply_auth_header(
        client.get(probe_url),
        api_key,
        provider.auth_header.as_deref(),
    )
    .send()
    .await;
    let latency_ms = started.elapsed().as_millis() as u64;
    match res {
        Ok(r) if r.status().is_success() || r.status().as_u16() == 405 => Ok(ProviderTestResult {
            ok: true,
            latency_ms: Some(latency_ms),
            message: format!("Reachable (HTTP {})", r.status().as_u16()),
            detected_version: None,
            details: None,
        }),
        Ok(r) => Ok(ProviderTestResult {
            ok: false,
            latency_ms: Some(latency_ms),
            message: format!("Endpoint returned HTTP {}", r.status().as_u16()),
            detected_version: None,
            details: None,
        }),
        Err(e) => Ok(ProviderTestResult {
            ok: false,
            latency_ms: Some(latency_ms),
            message: format!("Connection failed: {e}"),
            detected_version: None,
            details: None,
        }),
    }
}
