//! Built-in zero-setup macOS Speech Synthesis driver (`/usr/bin/say` + `/usr/bin/afconvert`).
//!
//! Strictly argv-only (never `sh -c`), uses `--` before user text so strings
//! starting with `-` can never inject flags, cleans up temp files automatically
//! via `tempfile::tempdir`, enforces a 60 s wall-clock timeout and `kill_on_drop`.

use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use otto_core::{Error, Result};

use super::{wav_duration_ms, TtsDriver, TtsSynthOutput, TtsSynthRequest};
use crate::types::{
    tts_mime_for_format, ProviderTestResult, TtsProviderRecord, TtsVoice, MAX_TTS_AUDIO_BYTES,
};

const SAY_BIN: &str = "/usr/bin/say";
const AFCONVERT_BIN: &str = "/usr/bin/afconvert";
const VOICE_LIST_TIMEOUT: Duration = Duration::from_secs(10);
const SYNTH_TIMEOUT: Duration = Duration::from_secs(60);

pub struct MacOsSayDriver;

/// Parse lines from `say -v '?'`:
/// `Samantha            en_US    # Hello! My name is Samantha.`
pub fn parse_say_voices(stdout: &str, provider_id: &str) -> Vec<TtsVoice> {
    let mut out = Vec::new();
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let (left, preview) = match trimmed.split_once('#') {
            Some((l, r)) => (l.trim_end(), Some(r.trim().to_string())),
            None => (trimmed, None),
        };
        // Last whitespace-separated token on the left is the locale (e.g. `en_US`).
        let mut parts = left.rsplitn(2, char::is_whitespace);
        let locale_cand = parts.next().unwrap_or("").trim();
        let name_cand = parts.next().unwrap_or("").trim();
        let (name, locale) = if !name_cand.is_empty()
            && locale_cand
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            && locale_cand.contains('_')
        {
            (name_cand.to_string(), Some(locale_cand.to_string()))
        } else {
            (left.to_string(), None)
        };
        if name.is_empty() {
            continue;
        }
        out.push(TtsVoice {
            id: name.clone(),
            name,
            locale,
            gender: None,
            preview_text: preview.filter(|s| !s.is_empty()),
            provider_id: provider_id.to_string(),
        });
    }
    out
}

fn validate_voice_name(voice: &str) -> Result<String> {
    let clean = voice.trim();
    if clean.is_empty() || clean.len() > 128 {
        return Err(Error::Invalid("invalid voice_id".into()));
    }
    if clean.starts_with('-') || clean.chars().any(|c| c.is_control()) {
        return Err(Error::Invalid(
            "voice_id contains disallowed characters".into(),
        ));
    }
    Ok(clean.to_string())
}

#[async_trait]
impl TtsDriver for MacOsSayDriver {
    async fn list_voices(
        &self,
        provider: &TtsProviderRecord,
        _api_key: Option<&str>,
    ) -> Result<Vec<TtsVoice>> {
        if !Path::new(SAY_BIN).is_file() {
            return Ok(Vec::new());
        }
        let mut cmd = tokio::process::Command::new(SAY_BIN);
        cmd.arg("-v")
            .arg("?")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let child = cmd
            .spawn()
            .map_err(|e| Error::Upstream(format!("failed to launch /usr/bin/say: {e}")))?;
        let out = tokio::time::timeout(VOICE_LIST_TIMEOUT, child.wait_with_output())
            .await
            .map_err(|_| Error::Upstream("listing macOS voices timed out".into()))?
            .map_err(|e| Error::Upstream(format!("reading macOS voices failed: {e}")))?;
        if !out.status.success() {
            return Err(Error::Upstream(format!(
                "say -v '?' exited with {}",
                out.status
            )));
        }
        let text = String::from_utf8_lossy(&out.stdout);
        Ok(parse_say_voices(&text, &provider.id))
    }

    async fn synthesize(
        &self,
        req: &TtsSynthRequest,
        _provider: &TtsProviderRecord,
        _api_key: Option<&str>,
    ) -> Result<TtsSynthOutput> {
        if !Path::new(SAY_BIN).is_file() {
            return Err(Error::Invalid(
                "macOS built-in TTS (/usr/bin/say) is unavailable on this host".into(),
            ));
        }
        let voice = validate_voice_name(&req.voice_id)?;
        let fmt = req.format.as_str();
        if !matches!(fmt, "wav" | "m4a" | "aiff") {
            return Err(Error::Invalid(format!(
                "macOS Built-in Voices supports wav, m4a, or aiff (got '{fmt}')"
            )));
        }
        // Standard speech rate ~180 WPM scaled by requested speed (0.5..=2.0).
        let wpm = ((180.0 * req.speed.clamp(0.5, 2.0)).round() as u32).clamp(80, 420);

        let tmp =
            tempfile::tempdir().map_err(|e| Error::Internal(format!("create TTS tempdir: {e}")))?;
        let aiff_path = tmp.path().join("speech.aiff");

        let mut say_cmd = tokio::process::Command::new(SAY_BIN);
        say_cmd
            .arg("-v")
            .arg(&voice)
            .arg("-r")
            .arg(wpm.to_string())
            .arg("-o")
            .arg(&aiff_path)
            .arg("--")
            .arg(&req.text)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let child = say_cmd
            .spawn()
            .map_err(|e| Error::Upstream(format!("failed to spawn /usr/bin/say: {e}")))?;
        let out = tokio::time::timeout(SYNTH_TIMEOUT, child.wait_with_output())
            .await
            .map_err(|_| Error::Upstream("macOS speech synthesis timed out".into()))?
            .map_err(|e| Error::Upstream(format!("macOS speech synthesis failed: {e}")))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(Error::Upstream(format!(
                "/usr/bin/say failed ({}): {}",
                out.status,
                err.trim()
            )));
        }

        let final_path = if fmt == "aiff" {
            aiff_path
        } else {
            if !Path::new(AFCONVERT_BIN).is_file() {
                return Err(Error::Internal(
                    "/usr/bin/afconvert is required for wav/m4a output".into(),
                ));
            }
            let conv_path = tmp.path().join(format!("speech.{fmt}"));
            let mut conv = tokio::process::Command::new(AFCONVERT_BIN);
            match fmt {
                "wav" => {
                    conv.args(["-f", "WAVE", "-d", "LEI16@22050"]);
                }
                "m4a" => {
                    conv.args(["-f", "m4af", "-d", "aac"]);
                }
                _ => unreachable!(),
            }
            conv.arg(&aiff_path)
                .arg(&conv_path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            let conv_child = conv
                .spawn()
                .map_err(|e| Error::Upstream(format!("failed to spawn afconvert: {e}")))?;
            let conv_out = tokio::time::timeout(SYNTH_TIMEOUT, conv_child.wait_with_output())
                .await
                .map_err(|_| Error::Upstream("audio conversion timed out".into()))?
                .map_err(|e| Error::Upstream(format!("audio conversion failed: {e}")))?;
            if !conv_out.status.success() {
                let err = String::from_utf8_lossy(&conv_out.stderr);
                return Err(Error::Upstream(format!(
                    "afconvert failed ({}): {}",
                    conv_out.status,
                    err.trim()
                )));
            }
            conv_path
        };

        let bytes = tokio::fs::read(&final_path)
            .await
            .map_err(|e| Error::Internal(format!("read synthesized audio: {e}")))?;
        if bytes.is_empty() {
            return Err(Error::Upstream(
                "speech synthesis produced empty audio".into(),
            ));
        }
        if bytes.len() > MAX_TTS_AUDIO_BYTES {
            return Err(Error::PayloadTooLarge(
                "synthesized audio exceeds 25 MB limit".into(),
            ));
        }
        let duration_ms = if fmt == "wav" {
            wav_duration_ms(&bytes)
        } else {
            None
        };
        let mime = tts_mime_for_format(fmt)
            .unwrap_or("application/octet-stream")
            .to_string();
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
        _api_key: Option<&str>,
    ) -> Result<ProviderTestResult> {
        let started = Instant::now();
        if !Path::new(SAY_BIN).is_file() {
            return Ok(ProviderTestResult {
                ok: false,
                latency_ms: Some(0),
                message: "/usr/bin/say not found on this host".into(),
                detected_version: None,
                details: None,
            });
        }
        let voices = self.list_voices(provider, None).await?;
        Ok(ProviderTestResult {
            ok: !voices.is_empty(),
            latency_ms: Some(started.elapsed().as_millis() as u64),
            message: format!(
                "macOS Speech Synthesis ready ({} voices installed)",
                voices.len()
            ),
            detected_version: Some("macOS say".into()),
            details: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_macos_say_voice_catalog() {
        let sample = "\
Albert              en_US    # Hello! My name is Albert.
Amélie              fr_CA    # Bonjour! Je m’appelle Amélie.
Bad News            en_US    # Hello! My name is Bad News.
";
        let voices = parse_say_voices(sample, "macos-say");
        assert_eq!(voices.len(), 3);
        assert_eq!(voices[0].id, "Albert");
        assert_eq!(voices[0].locale.as_deref(), Some("en_US"));
        assert_eq!(voices[1].name, "Amélie");
        assert_eq!(voices[1].locale.as_deref(), Some("fr_CA"));
        assert_eq!(voices[2].name, "Bad News");
        assert_eq!(
            voices[2].preview_text.as_deref(),
            Some("Hello! My name is Bad News.")
        );
    }

    #[test]
    fn rejects_flag_injection_voice_names() {
        assert!(validate_voice_name("-o").is_err());
        assert!(validate_voice_name("   ").is_err());
        assert!(validate_voice_name("Samantha").is_ok());
    }
}
