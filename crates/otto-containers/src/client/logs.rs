//! Docker multiplexed log framing decoder and plain TTY fallback.

use crate::types::ContainerLogLine;

/// Decode either Docker's 8-byte multiplexed log framing (`stdout`/`stderr`) or
/// plain TTY output into structured lines.
pub fn decode_docker_log_stream(raw: &[u8], timestamps: bool) -> Vec<ContainerLogLine> {
    if raw.is_empty() {
        return Vec::new();
    }
    if let Some(frames) = try_demux_docker_frames(raw) {
        let mut lines = Vec::new();
        for (stream, payload) in frames {
            let text = String::from_utf8_lossy(payload);
            for line in text.lines() {
                lines.push(split_log_line(stream, line, timestamps));
            }
        }
        return lines;
    }
    let text = String::from_utf8_lossy(raw);
    text.lines()
        .map(|line| split_log_line("tty", line, timestamps))
        .collect()
}

fn try_demux_docker_frames(raw: &[u8]) -> Option<Vec<(&'static str, &[u8])>> {
    let mut pos = 0usize;
    let mut frames = Vec::new();
    while pos < raw.len() {
        if raw.len() - pos < 8 {
            return None;
        }
        let stream_byte = raw[pos];
        if raw[pos + 1] != 0 || raw[pos + 2] != 0 || raw[pos + 3] != 0 {
            return None;
        }
        let stream = match stream_byte {
            1 => "stdout",
            2 => "stderr",
            0 => "stdin",
            _ => return None,
        };
        let len =
            u32::from_be_bytes([raw[pos + 4], raw[pos + 5], raw[pos + 6], raw[pos + 7]]) as usize;
        pos += 8;
        let end = pos.checked_add(len)?;
        if end > raw.len() {
            // Allow final frame truncation cleanly if the response hit the log cap.
            frames.push((stream, &raw[pos..]));
            return Some(frames);
        }
        frames.push((stream, &raw[pos..end]));
        pos = end;
    }
    Some(frames)
}

fn split_log_line(stream: &str, raw_line: &str, timestamps: bool) -> ContainerLogLine {
    if timestamps {
        if let Some((ts, rest)) = raw_line.split_once(' ') {
            if ts.len() >= 20 && ts.contains('T') {
                return ContainerLogLine {
                    stream: stream.to_string(),
                    timestamp: Some(ts.to_string()),
                    text: rest.to_string(),
                };
            }
        }
    }
    ContainerLogLine {
        stream: stream.to_string(),
        timestamp: None,
        text: raw_line.to_string(),
    }
}
