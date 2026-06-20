//! Kafka wire-protocol bits the SSH proxy needs: parse request headers, clamp
//! the `ApiVersions` response, and rewrite broker addresses in `Metadata` /
//! `FindCoordinator` responses. Pure and synchronous so it is exhaustively
//! unit-testable; the async plumbing in [`super::runtime`] calls into it.
//!
//! Only three response types are inspected. By clamping `ApiVersions` so
//! librdkafka requests `Metadata` ≤ v8 and `FindCoordinator` ≤ v2, those two
//! responses are always the **non-flexible** wire format (classic int32 array
//! counts, int16-length strings, response header v0 = just the correlation id).
//! `ApiVersions` itself always uses response header v0, and its body may be
//! flexible (compact) for request v≥3 — handled in [`clamp_api_versions`].
//!
//! All integers are big-endian (network order).

use otto_core::{Error, Result};

/// Kafka api keys we care about.
pub const API_METADATA: i16 = 3;
pub const API_FIND_COORDINATOR: i16 = 10;
pub const API_VERSIONS: i16 = 18;

/// Highest non-flexible versions; we clamp `ApiVersions` to these so the
/// responses we rewrite never use the flexible (compact) encoding.
pub const METADATA_MAX: i16 = 8;
pub const FIND_COORDINATOR_MAX: i16 = 2;

/// The fields the proxy tracks from a request header (the first 8 bytes after
/// the length prefix — identical across all request-header versions).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReqHeader {
    pub api_key: i16,
    pub api_version: i16,
    pub correlation_id: i32,
}

/// Parse `api_key` / `api_version` / `correlation_id` from a request frame body
/// (the bytes after the 4-byte length prefix). These occupy fixed offsets in
/// every request-header version, so we never need to know the header version.
pub fn parse_request_header(buf: &[u8]) -> Option<ReqHeader> {
    if buf.len() < 8 {
        return None;
    }
    Some(ReqHeader {
        api_key: i16::from_be_bytes([buf[0], buf[1]]),
        api_version: i16::from_be_bytes([buf[2], buf[3]]),
        correlation_id: i32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]),
    })
}

/// Whether a given (api_key, api_version) uses the flexible (compact) encoding.
/// Only meaningful for the keys we parse.
pub fn is_flexible(api_key: i16, api_version: i16) -> bool {
    match api_key {
        API_METADATA => api_version >= 9,
        API_FIND_COORDINATOR => api_version >= 3,
        API_VERSIONS => api_version >= 3,
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Byte cursor
// ---------------------------------------------------------------------------

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }
    fn pos(&self) -> usize {
        self.pos
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or_else(trunc)?;
        if end > self.buf.len() {
            return Err(trunc());
        }
        let s = &self.buf[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    fn i16(&mut self) -> Result<i16> {
        let b = self.take(2)?;
        Ok(i16::from_be_bytes([b[0], b[1]]))
    }
    fn i32(&mut self) -> Result<i32> {
        let b = self.take(4)?;
        Ok(i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    /// Non-nullable STRING: int16 length then bytes.
    fn string(&mut self) -> Result<&'a str> {
        let len = self.i16()?;
        if len < 0 {
            return Err(Error::Upstream("kafka: unexpected null string".into()));
        }
        let b = self.take(len as usize)?;
        std::str::from_utf8(b).map_err(|_| Error::Upstream("kafka: invalid utf8".into()))
    }
    /// Protobuf-style unsigned varint (used by flexible/compact encodings).
    fn uvarint(&mut self) -> Result<u64> {
        let mut val: u64 = 0;
        let mut shift = 0u32;
        loop {
            let byte = self.take(1)?[0];
            val |= ((byte & 0x7f) as u64) << shift;
            if byte & 0x80 == 0 {
                return Ok(val);
            }
            shift += 7;
            if shift >= 64 {
                return Err(Error::Upstream("kafka: varint overflow".into()));
            }
        }
    }
    fn rest(&self) -> &'a [u8] {
        &self.buf[self.pos..]
    }
}

fn trunc() -> Error {
    Error::Upstream("kafka: truncated frame".into())
}

fn put_i16(out: &mut Vec<u8>, v: i16) {
    out.extend_from_slice(&v.to_be_bytes());
}
fn put_i32(out: &mut Vec<u8>, v: i32) {
    out.extend_from_slice(&v.to_be_bytes());
}
/// Write a non-nullable STRING (int16 length + bytes).
fn put_string(out: &mut Vec<u8>, s: &str) {
    put_i16(out, s.len() as i16);
    out.extend_from_slice(s.as_bytes());
}

// ---------------------------------------------------------------------------
// ApiVersions clamp
// ---------------------------------------------------------------------------

/// Cap the advertised max version of `Metadata` (→ 8) and `FindCoordinator`
/// (→ 2) in an `ApiVersions` **response body** (the bytes after the 4-byte
/// correlation id; ApiVersions always uses response-header v0). `flexible` is
/// whether the request version was ≥ 3 (compact encoding). The edit is in place
/// and never changes the body length. Malformed input is left untouched.
pub fn clamp_api_versions(body: &mut [u8], flexible: bool) {
    let patches = {
        let mut r = Reader::new(body);
        if r.take(4).is_err() || r.i16().is_err() {
            return; // correlation id + error_code
        }
        let count = if flexible {
            match r.uvarint() {
                Ok(c) => c.saturating_sub(1) as i64,
                Err(_) => return,
            }
        } else {
            match r.i32() {
                Ok(c) => c as i64,
                Err(_) => return,
            }
        };
        let mut patches: Vec<(usize, i16)> = Vec::new();
        for _ in 0..count {
            let api_key = match r.i16() {
                Ok(v) => v,
                Err(_) => break,
            };
            if r.i16().is_err() {
                break; // min_version
            }
            let max_off = r.pos();
            let max = match r.i16() {
                Ok(v) => v,
                Err(_) => break,
            };
            if flexible {
                let _ = r.uvarint(); // per-entry tag buffer
            }
            let cap = match api_key {
                API_METADATA => Some(METADATA_MAX),
                API_FIND_COORDINATOR => Some(FIND_COORDINATOR_MAX),
                _ => None,
            };
            if let Some(cap) = cap {
                if max > cap {
                    patches.push((max_off, cap));
                }
            }
        }
        patches
    };
    for (off, val) in patches {
        body[off..off + 2].copy_from_slice(&val.to_be_bytes());
    }
}

// ---------------------------------------------------------------------------
// Metadata response rewrite (non-flexible, v0–v8)
// ---------------------------------------------------------------------------

/// The real broker endpoints (`host`, `port`) advertised in a non-flexible
/// `Metadata` response body (bytes after the correlation id). Used to pre-open
/// a local listener per broker before rewriting.
pub fn metadata_broker_endpoints(body: &[u8], version: i16) -> Result<Vec<(String, u16)>> {
    let mut r = Reader::new(body);
    r.take(4)?; // correlation id
    if version >= 3 {
        r.take(4)?; // throttle_time_ms
    }
    let count = r.i32()?;
    let mut out = Vec::new();
    for _ in 0..count.max(0) {
        r.i32()?; // node_id
        let host = r.string()?.to_string();
        let port = r.i32()? as u16;
        if version >= 1 {
            skip_nullable_string(&mut r)?; // rack
        }
        out.push((host, port));
    }
    Ok(out)
}

/// Rewrite each broker's `host`/`port` in a non-flexible `Metadata` response
/// body via `resolve(host, port) -> (new_host, new_port)`. Returns a fresh body
/// (length differs); bytes past the brokers array are copied verbatim.
pub fn rewrite_metadata<F>(body: &[u8], version: i16, resolve: F) -> Result<Vec<u8>>
where
    F: Fn(&str, u16) -> (String, u16),
{
    let mut r = Reader::new(body);
    let mut out = Vec::with_capacity(body.len() + 16);
    out.extend_from_slice(r.take(4)?); // correlation id
    if version >= 3 {
        out.extend_from_slice(r.take(4)?); // throttle_time_ms
    }
    let count = r.i32()?;
    put_i32(&mut out, count);
    for _ in 0..count.max(0) {
        put_i32(&mut out, r.i32()?); // node_id
        let host = r.string()?.to_string();
        let port = r.i32()? as u16;
        let (nh, np) = resolve(&host, port);
        put_string(&mut out, &nh);
        put_i32(&mut out, np as i32);
        if version >= 1 {
            copy_nullable_string(&mut r, &mut out)?; // rack
        }
    }
    out.extend_from_slice(r.rest());
    Ok(out)
}

// ---------------------------------------------------------------------------
// FindCoordinator response rewrite (non-flexible, v0–v2)
// ---------------------------------------------------------------------------

/// The coordinator endpoint advertised in a non-flexible `FindCoordinator`
/// response (None on error / empty host).
pub fn find_coordinator_endpoint(body: &[u8], version: i16) -> Result<Option<(String, u16)>> {
    let mut r = Reader::new(body);
    r.take(4)?; // correlation id
    if version >= 1 {
        r.take(4)?; // throttle_time_ms
    }
    r.i16()?; // error_code
    if version >= 1 {
        skip_nullable_string(&mut r)?; // error_message
    }
    r.i32()?; // node_id
    let host = r.string()?.to_string();
    let port = r.i32()? as u16;
    if host.is_empty() {
        return Ok(None);
    }
    Ok(Some((host, port)))
}

/// Rewrite the coordinator `host`/`port` in a non-flexible `FindCoordinator`
/// response body. An error response (empty host) is copied verbatim.
pub fn rewrite_find_coordinator<F>(body: &[u8], version: i16, resolve: F) -> Result<Vec<u8>>
where
    F: Fn(&str, u16) -> (String, u16),
{
    let mut r = Reader::new(body);
    let mut out = Vec::with_capacity(body.len() + 8);
    out.extend_from_slice(r.take(4)?); // correlation id
    if version >= 1 {
        out.extend_from_slice(r.take(4)?); // throttle_time_ms
    }
    put_i16(&mut out, r.i16()?); // error_code
    if version >= 1 {
        copy_nullable_string(&mut r, &mut out)?; // error_message
    }
    put_i32(&mut out, r.i32()?); // node_id
    let host = r.string()?.to_string();
    let port = r.i32()?; // keep as i32: error responses carry -1
    if host.is_empty() {
        put_string(&mut out, &host);
        put_i32(&mut out, port); // verbatim
    } else {
        let (nh, np) = resolve(&host, port as u16);
        put_string(&mut out, &nh);
        put_i32(&mut out, np as i32);
    }
    out.extend_from_slice(r.rest());
    Ok(out)
}

fn skip_nullable_string(r: &mut Reader) -> Result<()> {
    let len = r.i16()?;
    if len >= 0 {
        r.take(len as usize)?;
    }
    Ok(())
}

fn copy_nullable_string(r: &mut Reader, out: &mut Vec<u8>) -> Result<()> {
    let len = r.i16()?;
    put_i16(out, len);
    if len >= 0 {
        out.extend_from_slice(r.take(len as usize)?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put_str(out: &mut Vec<u8>, s: &str) {
        out.extend_from_slice(&(s.len() as i16).to_be_bytes());
        out.extend_from_slice(s.as_bytes());
    }
    fn put_nullable(out: &mut Vec<u8>, s: Option<&str>) {
        match s {
            Some(s) => put_str(out, s),
            None => out.extend_from_slice(&(-1i16).to_be_bytes()),
        }
    }

    #[test]
    fn parses_request_header() {
        // api_key=3 (Metadata), version=9, correlation=42, then a client_id.
        let mut buf = Vec::new();
        buf.extend_from_slice(&3i16.to_be_bytes());
        buf.extend_from_slice(&9i16.to_be_bytes());
        buf.extend_from_slice(&42i32.to_be_bytes());
        put_str(&mut buf, "rdkafka");
        let h = parse_request_header(&buf).unwrap();
        assert_eq!(h.api_key, API_METADATA);
        assert_eq!(h.api_version, 9);
        assert_eq!(h.correlation_id, 42);
        assert!(parse_request_header(&[0, 1, 2]).is_none());
    }

    #[test]
    fn flexible_classification() {
        assert!(!is_flexible(API_METADATA, 8));
        assert!(is_flexible(API_METADATA, 9));
        assert!(!is_flexible(API_FIND_COORDINATOR, 2));
        assert!(is_flexible(API_FIND_COORDINATOR, 3));
        assert!(is_flexible(API_VERSIONS, 3));
    }

    /// Build a non-flexible Metadata v4 response body (after the length prefix):
    /// correlation id, throttle, brokers[2], then arbitrary trailing bytes.
    fn metadata_v4(corr: i32) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&corr.to_be_bytes()); // correlation id
        b.extend_from_slice(&0i32.to_be_bytes()); // throttle (v>=3)
        b.extend_from_slice(&2i32.to_be_bytes()); // broker count
        // broker 0
        b.extend_from_slice(&101i32.to_be_bytes());
        put_str(&mut b, "b-1.msk.amazonaws.com");
        b.extend_from_slice(&9094i32.to_be_bytes());
        put_nullable(&mut b, Some("use1-az1")); // rack
                                                 // broker 1
        b.extend_from_slice(&102i32.to_be_bytes());
        put_str(&mut b, "b-2.msk.amazonaws.com");
        b.extend_from_slice(&9094i32.to_be_bytes());
        put_nullable(&mut b, None); // rack = null
        b.extend_from_slice(b"TRAILING-TOPICS-BYTES"); // opaque tail
        b
    }

    #[test]
    fn metadata_endpoints_and_rewrite() {
        let body = metadata_v4(7);
        let eps = metadata_broker_endpoints(&body, 4).unwrap();
        assert_eq!(
            eps,
            vec![
                ("b-1.msk.amazonaws.com".to_string(), 9094),
                ("b-2.msk.amazonaws.com".to_string(), 9094),
            ]
        );

        // Map each broker to a distinct local port.
        let out = rewrite_metadata(&body, 4, |host, port| {
            assert_eq!(port, 9094);
            let lp = if host.starts_with("b-1") { 30001 } else { 30002 };
            ("127.0.0.1".to_string(), lp)
        })
        .unwrap();

        // Re-parse the rewritten body: both hosts localhost, ports remapped,
        // node ids + rack + trailing bytes preserved.
        let eps2 = metadata_broker_endpoints(&out, 4).unwrap();
        assert_eq!(
            eps2,
            vec![
                ("127.0.0.1".to_string(), 30001),
                ("127.0.0.1".to_string(), 30002),
            ]
        );
        assert!(out.ends_with(b"TRAILING-TOPICS-BYTES"));
        // correlation id intact.
        assert_eq!(i32::from_be_bytes([out[0], out[1], out[2], out[3]]), 7);
    }

    #[test]
    fn find_coordinator_v1_rewrite() {
        // v1: corr, throttle, error_code=0, error_message=null, node=5, host, port
        let mut b = Vec::new();
        b.extend_from_slice(&9i32.to_be_bytes()); // corr
        b.extend_from_slice(&0i32.to_be_bytes()); // throttle
        b.extend_from_slice(&0i16.to_be_bytes()); // error_code
        put_nullable(&mut b, None); // error_message
        b.extend_from_slice(&5i32.to_be_bytes()); // node_id
        put_str(&mut b, "b-3.msk.amazonaws.com");
        b.extend_from_slice(&9094i32.to_be_bytes());

        assert_eq!(
            find_coordinator_endpoint(&b, 1).unwrap(),
            Some(("b-3.msk.amazonaws.com".to_string(), 9094))
        );
        let out = rewrite_find_coordinator(&b, 1, |_h, _p| ("127.0.0.1".into(), 40005)).unwrap();
        assert_eq!(
            find_coordinator_endpoint(&out, 1).unwrap(),
            Some(("127.0.0.1".to_string(), 40005))
        );
    }

    #[test]
    fn find_coordinator_error_passthrough() {
        // Error response: empty host, port -1 → not rewritten.
        let mut b = Vec::new();
        b.extend_from_slice(&1i32.to_be_bytes()); // corr
        b.extend_from_slice(&0i32.to_be_bytes()); // throttle
        b.extend_from_slice(&15i16.to_be_bytes()); // COORDINATOR_NOT_AVAILABLE
        put_nullable(&mut b, Some("not available"));
        b.extend_from_slice(&(-1i32).to_be_bytes()); // node_id
        put_str(&mut b, ""); // host
        b.extend_from_slice(&(-1i32).to_be_bytes()); // port
        assert_eq!(find_coordinator_endpoint(&b, 1).unwrap(), None);
        let out = rewrite_find_coordinator(&b, 1, |_, _| ("x".into(), 1)).unwrap();
        assert_eq!(out, b); // unchanged
    }

    #[test]
    fn clamp_api_versions_flexible() {
        // Flexible ApiVersions v3 response: corr, error_code, compact array of 3
        // entries (Produce, Metadata, FindCoordinator), throttle, tag buffer.
        let mut b = Vec::new();
        b.extend_from_slice(&3i32.to_be_bytes()); // corr
        b.extend_from_slice(&0i16.to_be_bytes()); // error_code
        b.push(4); // compact array len = 3 (+1)
        let entry = |out: &mut Vec<u8>, key: i16, min: i16, max: i16| {
            out.extend_from_slice(&key.to_be_bytes());
            out.extend_from_slice(&min.to_be_bytes());
            out.extend_from_slice(&max.to_be_bytes());
            out.push(0); // per-entry tag buffer
        };
        entry(&mut b, 0, 0, 9); // Produce — untouched
        entry(&mut b, API_METADATA, 0, 12); // Metadata 12 → clamp to 8
        entry(&mut b, API_FIND_COORDINATOR, 0, 4); // FindCoordinator 4 → clamp to 2
        b.extend_from_slice(&0i32.to_be_bytes()); // throttle
        b.push(0); // top-level tag buffer

        clamp_api_versions(&mut b, true);

        // Re-read the entries' max values.
        let mut r = Reader::new(&b);
        r.take(4).unwrap();
        r.i16().unwrap();
        let n = r.uvarint().unwrap() - 1;
        let mut maxes = Vec::new();
        for _ in 0..n {
            let key = r.i16().unwrap();
            r.i16().unwrap();
            let max = r.i16().unwrap();
            r.uvarint().unwrap();
            maxes.push((key, max));
        }
        assert_eq!(maxes, vec![(0, 9), (API_METADATA, 8), (API_FIND_COORDINATOR, 2)]);
    }

    #[test]
    fn clamp_api_versions_classic() {
        // Non-flexible v1: corr, error_code, int32 array of 1 (Metadata=11), throttle.
        let mut b = Vec::new();
        b.extend_from_slice(&3i32.to_be_bytes());
        b.extend_from_slice(&0i16.to_be_bytes());
        b.extend_from_slice(&1i32.to_be_bytes()); // count
        b.extend_from_slice(&API_METADATA.to_be_bytes());
        b.extend_from_slice(&0i16.to_be_bytes());
        b.extend_from_slice(&11i16.to_be_bytes()); // max=11
        b.extend_from_slice(&0i32.to_be_bytes()); // throttle
        clamp_api_versions(&mut b, false);
        // max field is at offset 4+2+4+2+2 = 14
        assert_eq!(i16::from_be_bytes([b[14], b[15]]), 8);
    }
}
