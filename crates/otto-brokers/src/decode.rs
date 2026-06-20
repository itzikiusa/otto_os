//! Message payload decoders for the viewer.
//!
//! A Kafka key/value is opaque bytes. We render it as the most useful form:
//! pretty JSON, UTF-8 text, a schemaless **Protobuf** wire dump (field number →
//! value — the "gRPC viewer" without needing a `.proto`), Confluent-framed
//! **Avro** decoded via a schema registry, or hex/base64 for raw binary.

use crate::types::{DecodedPayload, ValueFormat};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde_json::{json, Value};

/// Above this size we skip attaching `raw_base64` (the viewer keeps a small N of
/// messages; this just bounds memory for the occasional huge payload).
const MAX_RAW_B64: usize = 256 * 1024;

fn b64(bytes: &[u8]) -> String {
    B64.encode(bytes)
}

fn raw_for(bytes: &[u8]) -> Option<String> {
    (bytes.len() <= MAX_RAW_B64).then(|| b64(bytes))
}

/// True for text with no control characters other than tab/newline/return.
fn is_texty(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| !c.is_control() || matches!(c, '\t' | '\n' | '\r'))
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
}

/// Decode a key/value for display. `None` is a tombstone / absent key.
pub fn decode_payload(bytes: Option<&[u8]>, requested: ValueFormat) -> DecodedPayload {
    let Some(bytes) = bytes else {
        return DecodedPayload {
            format: "null".into(),
            text: String::new(),
            schema_id: None,
            raw_base64: None,
        };
    };
    match requested {
        ValueFormat::Hex => DecodedPayload {
            format: "hex".into(),
            text: hex::encode(bytes),
            schema_id: None,
            raw_base64: raw_for(bytes),
        },
        ValueFormat::Base64 => DecodedPayload {
            format: "base64".into(),
            text: b64(bytes),
            schema_id: None,
            raw_base64: raw_for(bytes),
        },
        ValueFormat::Utf8 => as_string(bytes),
        ValueFormat::Json => match serde_json::from_slice::<Value>(bytes) {
            Ok(v) => DecodedPayload {
                format: "json".into(),
                text: pretty(&v),
                schema_id: None,
                raw_base64: raw_for(bytes),
            },
            Err(_) => as_string(bytes),
        },
        ValueFormat::Protobuf => match protobuf_wire_to_json(bytes) {
            Some(v) => DecodedPayload {
                format: "protobuf".into(),
                text: pretty(&v),
                schema_id: None,
                raw_base64: raw_for(bytes),
            },
            None => as_hex(bytes),
        },
        // Avro is handled by the caller (needs an async registry lookup); if it
        // reaches here we have no schema, so fall back to auto.
        ValueFormat::Avro | ValueFormat::Auto => auto_decode(bytes),
    }
}

fn as_string(bytes: &[u8]) -> DecodedPayload {
    match std::str::from_utf8(bytes) {
        Ok(s) if is_texty(s) => DecodedPayload {
            format: "string".into(),
            text: s.to_string(),
            schema_id: None,
            raw_base64: raw_for(bytes),
        },
        _ => as_hex(bytes),
    }
}

fn as_hex(bytes: &[u8]) -> DecodedPayload {
    DecodedPayload {
        format: "hex".into(),
        text: hex::encode(bytes),
        schema_id: None,
        raw_base64: raw_for(bytes),
    }
}

/// Auto: JSON → texty UTF-8 → plausible Protobuf → hex.
fn auto_decode(bytes: &[u8]) -> DecodedPayload {
    if let Ok(v) = serde_json::from_slice::<Value>(bytes) {
        // A bare number/string is valid JSON but rarely what the user wants;
        // only treat objects/arrays as "json", everything else as a string.
        if v.is_object() || v.is_array() {
            return DecodedPayload {
                format: "json".into(),
                text: pretty(&v),
                schema_id: None,
                raw_base64: raw_for(bytes),
            };
        }
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        if is_texty(s) {
            return DecodedPayload {
                format: "string".into(),
                text: s.to_string(),
                schema_id: None,
                raw_base64: raw_for(bytes),
            };
        }
    }
    if let Some(v) = protobuf_wire_to_json(bytes) {
        return DecodedPayload {
            format: "protobuf".into(),
            text: pretty(&v),
            schema_id: None,
            raw_base64: raw_for(bytes),
        };
    }
    as_hex(bytes)
}

/// Build the decoded value for an already-known JSON value (e.g. Avro output),
/// tagging it with the registry schema id.
pub fn avro_payload(value: &Value, schema_id: i32, raw: &[u8]) -> DecodedPayload {
    DecodedPayload {
        format: "avro".into(),
        text: pretty(value),
        schema_id: Some(schema_id),
        raw_base64: raw_for(raw),
    }
}

// ---------------------------------------------------------------------------
// Confluent wire framing
// ---------------------------------------------------------------------------

/// Confluent serializers prefix the payload with a magic byte `0x00` and a
/// 4-byte big-endian schema id. Returns `(schema_id, body)` when framed.
pub fn confluent_frame(bytes: &[u8]) -> Option<(i32, &[u8])> {
    if bytes.len() >= 5 && bytes[0] == 0x00 {
        let id = i32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
        Some((id, &bytes[5..]))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Schemaless Protobuf wire decoder
// ---------------------------------------------------------------------------

const MAX_PROTO_DEPTH: usize = 16;

fn read_varint(buf: &[u8], pos: &mut usize) -> Option<u64> {
    let mut result: u64 = 0;
    let mut shift = 0u32;
    loop {
        if *pos >= buf.len() || shift >= 64 {
            return None;
        }
        let b = buf[*pos];
        *pos += 1;
        result |= ((b & 0x7f) as u64) << shift;
        if b & 0x80 == 0 {
            return Some(result);
        }
        shift += 7;
    }
}

/// Decode raw Protobuf wire bytes into a JSON object keyed by field number.
/// Strict: every byte must be consumed and at least one field present, so
/// random binary rarely passes (keeps `auto` from mislabeling).
pub fn protobuf_wire_to_json(buf: &[u8]) -> Option<Value> {
    decode_proto(buf, 0)
}

fn decode_proto(buf: &[u8], depth: usize) -> Option<Value> {
    if depth > MAX_PROTO_DEPTH || buf.is_empty() {
        return None;
    }
    let mut map = serde_json::Map::new();
    let mut pos = 0usize;
    while pos < buf.len() {
        let tag = read_varint(buf, &mut pos)?;
        let field = tag >> 3;
        let wire = tag & 0x7;
        if field == 0 {
            return None;
        }
        let value = match wire {
            0 => {
                let v = read_varint(buf, &mut pos)?;
                json!(v)
            }
            1 => {
                if pos + 8 > buf.len() {
                    return None;
                }
                let arr: [u8; 8] = buf[pos..pos + 8].try_into().ok()?;
                pos += 8;
                json!(u64::from_le_bytes(arr))
            }
            2 => {
                let len = read_varint(buf, &mut pos)? as usize;
                if pos + len > buf.len() {
                    return None;
                }
                let sub = &buf[pos..pos + len];
                pos += len;
                // Prefer readable text: real nested messages begin with a
                // low/control tag byte and fail `is_texty`, so this only catches
                // genuine string fields (and keeps short strings like "hi" from
                // being mis-decoded as a nested message).
                match std::str::from_utf8(sub) {
                    Ok(s) if is_texty(s) => json!(s),
                    _ => match decode_proto(sub, depth + 1) {
                        Some(nested) => nested,
                        None => json!({ "@bytes_b64": b64(sub) }),
                    },
                }
            }
            5 => {
                if pos + 4 > buf.len() {
                    return None;
                }
                let arr: [u8; 4] = buf[pos..pos + 4].try_into().ok()?;
                pos += 4;
                json!(u32::from_le_bytes(arr))
            }
            // 3/4 (start/end group) are deprecated and unsupported here.
            _ => return None,
        };
        let key = field.to_string();
        match map.get_mut(&key) {
            Some(Value::Array(arr)) => arr.push(value),
            Some(existing) => {
                let prev = existing.take();
                *existing = json!([prev, value]);
            }
            None => {
                map.insert(key, value);
            }
        }
    }
    if map.is_empty() {
        None
    } else {
        Some(Value::Object(map))
    }
}

// ---------------------------------------------------------------------------
// Avro → JSON
// ---------------------------------------------------------------------------

/// Decode an Avro datum (single-object, no header) against `schema_json`.
pub fn avro_to_json(schema_json: &str, body: &[u8]) -> anyhow::Result<Value> {
    use apache_avro::Schema;
    let schema = Schema::parse_str(schema_json)?;
    let mut cursor = std::io::Cursor::new(body);
    let value = apache_avro::from_avro_datum(&schema, &mut cursor, None)?;
    Ok(avro_value_to_json(value))
}

fn avro_value_to_json(v: apache_avro::types::Value) -> Value {
    use apache_avro::types::Value as A;
    match v {
        A::Null => Value::Null,
        A::Boolean(b) => json!(b),
        A::Int(i) | A::Date(i) | A::TimeMillis(i) => json!(i),
        A::Long(i)
        | A::TimeMicros(i)
        | A::TimestampMillis(i)
        | A::TimestampMicros(i)
        | A::LocalTimestampMillis(i)
        | A::LocalTimestampMicros(i) => json!(i),
        A::Float(f) => json!(f),
        A::Double(f) => json!(f),
        A::Bytes(b) | A::Fixed(_, b) => json!(b64(&b)),
        A::String(s) | A::Enum(_, s) => json!(s),
        A::Uuid(u) => json!(u.to_string()),
        A::Union(_, inner) => avro_value_to_json(*inner),
        A::Array(items) => Value::Array(items.into_iter().map(avro_value_to_json).collect()),
        A::Map(m) => Value::Object(
            m.into_iter()
                .map(|(k, val)| (k, avro_value_to_json(val)))
                .collect(),
        ),
        A::Record(fields) => Value::Object(
            fields
                .into_iter()
                .map(|(k, val)| (k, avro_value_to_json(val)))
                .collect(),
        ),
        other => json!(format!("{other:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_json_object() {
        let d = decode_payload(Some(br#"{"a":1,"b":[2,3]}"#), ValueFormat::Auto);
        assert_eq!(d.format, "json");
        assert!(d.text.contains("\"a\""));
    }

    #[test]
    fn decode_plain_text() {
        let d = decode_payload(Some(b"hello world"), ValueFormat::Auto);
        assert_eq!(d.format, "string");
        assert_eq!(d.text, "hello world");
    }

    #[test]
    fn decode_binary_falls_back_to_hex() {
        let d = decode_payload(Some(&[0xff, 0xfe, 0x00, 0x01, 0x99]), ValueFormat::Auto);
        // 0xff 0xfe ... is not a valid protobuf message → hex.
        assert_eq!(d.format, "hex");
        assert!(d.raw_base64.is_some());
    }

    #[test]
    fn decode_null_tombstone() {
        let d = decode_payload(None, ValueFormat::Auto);
        assert_eq!(d.format, "null");
    }

    #[test]
    fn protobuf_wire_roundtrip() {
        // message { field 1 (varint) = 150; field 2 (string) = "hi" }
        // 0x08 0x96 0x01  | 0x12 0x02 0x68 0x69
        let bytes = [0x08, 0x96, 0x01, 0x12, 0x02, 0x68, 0x69];
        let v = protobuf_wire_to_json(&bytes).expect("decodes");
        assert_eq!(v["1"], json!(150));
        assert_eq!(v["2"], json!("hi"));
    }

    #[test]
    fn protobuf_nested_and_repeated() {
        // field 1 = 1; field 1 = 2 (repeated) ; field 3 = nested{ field1="x" }
        // 0x08 0x01 | 0x08 0x02 | 0x1a 0x03 0x0a 0x01 0x78
        let bytes = [0x08, 0x01, 0x08, 0x02, 0x1a, 0x03, 0x0a, 0x01, 0x78];
        let v = protobuf_wire_to_json(&bytes).expect("decodes");
        assert_eq!(v["1"], json!([1, 2]));
        assert_eq!(v["3"]["1"], json!("x"));
    }

    #[test]
    fn confluent_frame_parse() {
        let mut buf = vec![0x00, 0x00, 0x00, 0x00, 0x07];
        buf.extend_from_slice(b"body");
        let (id, body) = confluent_frame(&buf).unwrap();
        assert_eq!(id, 7);
        assert_eq!(body, b"body");
        assert!(confluent_frame(b"xx").is_none());
    }

    #[test]
    fn avro_record_decodes() {
        let schema = r#"{"type":"record","name":"U","fields":[
            {"name":"id","type":"long"},{"name":"name","type":"string"}]}"#;
        // Encode with apache-avro to get bytes, then decode back.
        use apache_avro::types::Record;
        use apache_avro::{to_avro_datum, Schema};
        let s = Schema::parse_str(schema).unwrap();
        let mut rec = Record::new(&s).unwrap();
        rec.put("id", 42i64);
        rec.put("name", "neo");
        let body = to_avro_datum(&s, rec).unwrap();
        let v = avro_to_json(schema, &body).unwrap();
        assert_eq!(v["id"], json!(42));
        assert_eq!(v["name"], json!("neo"));
    }
}
