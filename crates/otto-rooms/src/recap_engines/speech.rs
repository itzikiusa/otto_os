use super::*;
use otto_core::{Error, Result};
pub fn validate_wav(bytes: &[u8]) -> Result<u64> {
    let invalid =
        || Error::Invalid("Expected up to 30 seconds of 16 kHz mono 16-bit PCM WAV".into());
    if bytes.len() < 46 || bytes.len() > 960_044 {
        return Err(invalid());
    }
    let u16_at = |n| u16::from_le_bytes([bytes[n], bytes[n + 1]]);
    let u32_at = |n| u32::from_le_bytes(bytes[n..n + 4].try_into().unwrap());
    if &bytes[..4] != b"RIFF"
        || &bytes[8..16] != b"WAVEfmt "
        || &bytes[36..40] != b"data"
        || u32_at(4) as usize != bytes.len() - 8
        || u32_at(16) != 16
        || u16_at(20) != 1
        || u16_at(22) != 1
        || u32_at(24) != 16000
        || u32_at(28) != 32000
        || u16_at(32) != 2
        || u16_at(34) != 16
        || u32_at(40) as usize != bytes.len() - 44
        || !(bytes.len() - 44).is_multiple_of(2)
    {
        return Err(invalid());
    }
    Ok((bytes.len() as u64 - 44) / 32)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn wav(samples: usize) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend(b"RIFF");
        b.extend((36 + samples as u32 * 2).to_le_bytes());
        b.extend(b"WAVEfmt ");
        b.extend(16u32.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(16000u32.to_le_bytes());
        b.extend(32000u32.to_le_bytes());
        b.extend(2u16.to_le_bytes());
        b.extend(16u16.to_le_bytes());
        b.extend(b"data");
        b.extend((samples as u32 * 2).to_le_bytes());
        b.resize(44 + samples * 2, 0);
        b
    }
    #[test]
    fn accepts_only_bounded_mono_pcm() {
        assert_eq!(validate_wav(&wav(16000)).unwrap(), 1000);
        for bytes in [wav(0), wav(480001), b"not audio".to_vec()] {
            assert!(validate_wav(&bytes).is_err());
        }
        let mut stereo = wav(16000);
        stereo[22] = 2;
        assert!(validate_wav(&stereo).is_err());
        let mut false_size = wav(16000);
        false_size[40] ^= 1;
        assert!(validate_wav(&false_size).is_err());
        let mut rate = wav(16000);
        rate[24] = 0;
        assert!(validate_wav(&rate).is_err());
    }
}

fn config_valid(config: &EngineConfig) -> Result<()> {
    if !(1..=4).contains(&config.threads)
        || !matches!(config.language.as_str(), "auto" | "en" | "he")
    {
        return Err(Error::Invalid(
            "Choose auto, en or he and 1–4 recognition threads".into(),
        ));
    }
    for value in [&config.whisper_executable, &config.whisper_model] {
        if !std::path::Path::new(value).is_absolute() || value.len() > 4096 {
            return Err(Error::Invalid(
                "Choose absolute paths for whisper-cli and a multilingual model".into(),
            ));
        }
    }
    Ok(())
}
fn parse_segments(bytes: &[u8], duration: u64) -> Result<Vec<SpeechSegment>> {
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|_| Error::Upstream("Speech recognizer returned invalid JSON".into()))?;
    let entries = value
        .get("transcription")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            Error::Upstream("Speech recognizer omitted timestamped transcription".into())
        })?;
    let mut segments = Vec::new();
    for entry in entries {
        let text = entry
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Upstream("Invalid speech segment".into()))?
            .trim();
        let start = entry.pointer("/offsets/from").and_then(|v| v.as_u64());
        let end = entry.pointer("/offsets/to").and_then(|v| v.as_u64());
        let (Some(start_ms), Some(end_ms)) = (start, end) else {
            return Err(Error::Upstream(
                "Speech recognizer omitted segment offsets".into(),
            ));
        };
        if start_ms > end_ms
            || start_ms > duration
            || end_ms > duration.saturating_add(1000)
            || text.len() > 16384
        {
            return Err(Error::Upstream(
                "Speech recognizer returned out-of-range segment".into(),
            ));
        }
        if !text.is_empty() {
            segments.push(SpeechSegment {
                start_ms,
                end_ms: end_ms.min(duration),
                text: text.into(),
            });
        }
    }
    Ok(segments)
}
pub async fn transcribe(config: &EngineConfig, wav: &[u8]) -> Result<Vec<SpeechSegment>> {
    config_valid(config)?;
    let duration = validate_wav(wav)?;
    let dir = tempfile::tempdir().map_err(|e| Error::Internal(e.to_string()))?;
    let input = dir.path().join("input.wav");
    let output = dir.path().join("transcript");
    tokio::fs::write(&input, wav)
        .await
        .map_err(|e| Error::Internal(e.to_string()))?;
    let args = vec![
        "--model".into(),
        config.whisper_model.clone(),
        "--file".into(),
        input.display().to_string(),
        "--language".into(),
        config.language.clone(),
        "--threads".into(),
        config.threads.to_string(),
        "--output-json".into(),
        "--output-file".into(),
        output.display().to_string(),
        "--no-prints".into(),
    ];
    process::run(
        &config.whisper_executable,
        &args,
        b"",
        dir.path(),
        std::time::Duration::from_secs(120),
    )
    .await?;
    let file = tokio::fs::File::open(output.with_extension("json")).await.map_err(|_|Error::Upstream("Speech recognizer did not create a transcript; verify whisper.cpp CLI compatibility".into()))?;
    use tokio::io::AsyncReadExt;
    let mut bytes = Vec::new();
    file.take(262145)
        .read_to_end(&mut bytes)
        .await
        .map_err(|e| Error::Internal(e.to_string()))?;
    if bytes.len() > 262144 {
        return Err(Error::Upstream(
            "Speech transcript exceeded chunk limit".into(),
        ));
    }
    parse_segments(&bytes, duration)
}
pub async fn capabilities(config: &EngineConfig) -> Result<serde_json::Value> {
    let error = config_valid(config).err().map(|e| e.to_string());
    let executable = tokio::fs::metadata(&config.whisper_executable).await.ok();
    let model = tokio::fs::metadata(&config.whisper_model).await.ok();
    #[cfg(unix)]
    let executable_ready = {
        use std::os::unix::fs::PermissionsExt;
        executable.is_some_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    #[cfg(not(unix))]
    let executable_ready = executable.is_some_and(|m| m.is_file());
    let model_ready = model.is_some_and(|m| m.is_file() && m.len() > 1_000_000);
    let dir = tempfile::tempdir().map_err(|e| Error::Internal(e.to_string()))?;
    let codex = process::run_capture(
        "codex",
        &["login".into(), "status".into()],
        b"",
        dir.path(),
        std::time::Duration::from_secs(5),
    )
    .await;
    let codex_ready = codex.is_ok_and(|(out, err)| {
        String::from_utf8_lossy(&out).contains("ChatGPT")
            || String::from_utf8_lossy(&err).contains("ChatGPT")
    });
    Ok(
        serde_json::json!({"speech_ready":error.is_none() && executable_ready && model_ready,
        "executable_ready":executable_ready,"model_ready":model_ready,"configuration_error":error,
        "codex_subscription_ready":codex_ready,"screen_text_ready":cfg!(target_os="macos"),
        "languages":["auto","en","he"],"setup":"Install whisper.cpp and a multilingual GGML model, then select their absolute paths. Run codex login for summary access."}),
    )
}

#[cfg(test)]
mod segment_tests {
    use super::*;
    #[test]
    fn timestamped_segments_preserve_hebrew_and_reject_bad_offsets() {
        let value =
            serde_json::json!({"transcription":[{"text":" שלום ","offsets":{"from":0,"to":900}}]});
        let result = parse_segments(&serde_json::to_vec(&value).unwrap(), 1000).unwrap();
        assert_eq!(
            result[0],
            SpeechSegment {
                start_ms: 0,
                end_ms: 900,
                text: "שלום".into()
            }
        );
        for value in [
            serde_json::json!({}),
            serde_json::json!({"transcription":[{"text":"a","offsets":{"from":900,"to":1}}]}),
            serde_json::json!({"transcription":[{"text":"a","offsets":{"from":0,"to":90000}}]}),
        ] {
            assert!(parse_segments(&serde_json::to_vec(&value).unwrap(), 1000).is_err());
        }
    }
    #[test]
    fn settings_require_local_paths_and_bounded_threads() {
        let mut config = EngineConfig {
            whisper_executable: "whisper-cli".into(),
            whisper_model: "model.bin".into(),
            language: "he".into(),
            threads: 2,
        };
        assert!(config_valid(&config).is_err());
        config.whisper_executable = "/tmp/whisper-cli".into();
        config.whisper_model = "/tmp/model.bin".into();
        assert!(config_valid(&config).is_ok());
        config.threads = 8;
        assert!(config_valid(&config).is_err());
    }
}
