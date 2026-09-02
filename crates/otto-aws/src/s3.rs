//! S3 — read-only: buckets, prefix listing, head, text preview, streamed
//! download (§2.2). All `s3api` JSON except the download, which pipes
//! `aws s3 cp s3://… -` stdout straight into the response body.

use std::process::Stdio;

use otto_core::{Error, Result};
use otto_state::AwsAccountRow;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::accounts::AwsService;

/// Default / hard cap for `preview?max_bytes=`.
pub const PREVIEW_DEFAULT: u64 = 64 * 1024;
pub const PREVIEW_CAP: u64 = 1024 * 1024;
/// Download refusal threshold (§2.2).
pub const DOWNLOAD_CAP: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct S3Bucket {
    pub name: String,
    pub creation_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BucketsResp {
    pub buckets: Vec<S3Bucket>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct S3Object {
    pub key: String,
    pub size: u64,
    pub last_modified: Option<String>,
    pub storage_class: Option<String>,
    pub etag: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ObjectsResp {
    pub prefixes: Vec<String>,
    pub objects: Vec<S3Object>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_token: Option<String>,
    pub is_truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct HeadResp {
    pub key: String,
    pub size: u64,
    pub content_type: Option<String>,
    pub last_modified: Option<String>,
    pub etag: Option<String>,
    pub metadata: Value,
    pub storage_class: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PreviewResp {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub truncated: bool,
    pub content_type: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub binary: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ObjectsQuery {
    pub prefix: Option<String>,
    pub token: Option<String>,
    pub max: Option<u32>,
    pub region: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KeyQuery {
    pub key: String,
    pub max_bytes: Option<u64>,
    pub region: Option<String>,
}

// ---------------------------------------------------------------------------
// Normalizers (pure)
// ---------------------------------------------------------------------------

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(|x| x.as_str()).map(str::to_string)
}

pub fn normalize_buckets(v: &Value) -> Vec<S3Bucket> {
    v.get("Buckets")
        .and_then(|b| b.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|b| {
                    Some(S3Bucket {
                        name: s(b, "Name")?,
                        creation_date: s(b, "CreationDate"),
                        region: s(b, "BucketRegion"),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `list-objects-v2` (paginated via `--max-items`/`--starting-token`, so the
/// continuation lives in `NextToken`; a raw page carries
/// `NextContinuationToken`) → folders first, then objects. The prefix
/// "directory marker" object (key == prefix) is dropped.
pub fn normalize_objects(v: &Value, prefix: &str) -> ObjectsResp {
    let prefixes: Vec<String> = v
        .get("CommonPrefixes")
        .and_then(|p| p.as_array())
        .map(|arr| arr.iter().filter_map(|p| s(p, "Prefix")).collect())
        .unwrap_or_default();
    let objects: Vec<S3Object> = v
        .get("Contents")
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|o| {
                    let key = s(o, "Key")?;
                    if !prefix.is_empty() && key == prefix {
                        return None;
                    }
                    Some(S3Object {
                        key,
                        size: o.get("Size").and_then(|x| x.as_u64()).unwrap_or(0),
                        last_modified: s(o, "LastModified"),
                        storage_class: s(o, "StorageClass"),
                        etag: s(o, "ETag").map(|e| e.trim_matches('"').to_string()),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let next_token = s(v, "NextToken").or_else(|| s(v, "NextContinuationToken"));
    let is_truncated = next_token.is_some()
        || v.get("IsTruncated")
            .and_then(|x| x.as_bool())
            .unwrap_or(false);
    ObjectsResp {
        prefixes,
        objects,
        next_token,
        is_truncated,
    }
}

pub fn normalize_head(key: &str, v: &Value) -> HeadResp {
    HeadResp {
        key: key.to_string(),
        size: v.get("ContentLength").and_then(|x| x.as_u64()).unwrap_or(0),
        content_type: s(v, "ContentType"),
        last_modified: s(v, "LastModified"),
        etag: s(v, "ETag").map(|e| e.trim_matches('"').to_string()),
        metadata: v
            .get("Metadata")
            .cloned()
            .unwrap_or_else(|| Value::Object(Default::default())),
        storage_class: s(v, "StorageClass"),
    }
}

const TEXT_EXTS: &[&str] = &[
    "txt",
    "log",
    "json",
    "jsonl",
    "ndjson",
    "csv",
    "tsv",
    "yaml",
    "yml",
    "xml",
    "md",
    "html",
    "htm",
    "sql",
    "sh",
    "toml",
    "ini",
    "conf",
    "cfg",
    "env",
    "properties",
    "js",
    "ts",
    "py",
    "rb",
    "go",
    "rs",
];

/// Is this previewable as text? Text MIME families, the structured-data
/// application types, or — for the ubiquitous `application/octet-stream` /
/// `binary/octet-stream` / missing — a text-looking key extension (§2.2
/// "refuse non-text content types except JSON/CSV/YAML/log").
pub fn is_texty(content_type: Option<&str>, key: &str) -> bool {
    let ct = content_type
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if ct.starts_with("text/") {
        return true;
    }
    if matches!(
        ct.as_str(),
        "application/json"
            | "application/x-ndjson"
            | "application/jsonl"
            | "application/xml"
            | "application/yaml"
            | "application/x-yaml"
            | "application/csv"
            | "application/javascript"
            | "application/x-sh"
            | "application/sql"
            | "application/toml"
    ) {
        return true;
    }
    if ct.is_empty() || ct == "application/octet-stream" || ct == "binary/octet-stream" {
        let ext = key.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        // `.gz`-style suffixes are NOT text even if the stem is.
        return TEXT_EXTS.contains(&ext.as_str());
    }
    false
}

/// Bytes → preview text; a NUL byte in the sample means binary regardless of
/// the declared type.
pub fn preview_from_bytes(
    bytes: &[u8],
    content_type: Option<String>,
    total_size: Option<u64>,
    key: &str,
) -> PreviewResp {
    let texty = is_texty(content_type.as_deref(), key) && !bytes.contains(&0);
    if !texty {
        return PreviewResp {
            text: None,
            truncated: false,
            content_type,
            binary: true,
        };
    }
    let truncated = total_size.map(|t| t > bytes.len() as u64).unwrap_or(false);
    PreviewResp {
        text: Some(String::from_utf8_lossy(bytes).into_owned()),
        truncated,
        content_type,
        binary: false,
    }
}

/// Guard against `..`, leading `/` and control chars in a caller-supplied key
/// (keys are passed as argv, never a shell, but a tidy 400 beats a CLI error).
pub fn validate_key(key: &str) -> Result<()> {
    if key.is_empty() || key.len() > 1024 {
        return Err(Error::Invalid("key must be 1..1024 chars".into()));
    }
    if key.chars().any(|c| c.is_control()) {
        return Err(Error::Invalid("key contains control characters".into()));
    }
    Ok(())
}

pub fn validate_bucket(b: &str) -> Result<()> {
    if b.len() < 3
        || b.len() > 63
        || !b
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.')
    {
        return Err(Error::Invalid(format!("invalid bucket name '{b}'")));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Calls
// ---------------------------------------------------------------------------

pub async fn list_buckets(
    svc: &AwsService,
    a: &AwsAccountRow,
    region: Option<&str>,
) -> Result<BucketsResp> {
    let v = svc.run_json(a, region, &["s3api", "list-buckets"]).await?;
    Ok(BucketsResp {
        buckets: normalize_buckets(&v),
    })
}

pub async fn list_objects(
    svc: &AwsService,
    a: &AwsAccountRow,
    bucket: &str,
    q: &ObjectsQuery,
) -> Result<ObjectsResp> {
    validate_bucket(bucket)?;
    let prefix = q.prefix.clone().unwrap_or_default();
    let max = q.max.unwrap_or(500).clamp(1, 1000).to_string();
    let mut args: Vec<&str> = vec![
        "s3api",
        "list-objects-v2",
        "--bucket",
        bucket,
        "--delimiter",
        "/",
        "--max-items",
        &max,
    ];
    if !prefix.is_empty() {
        args.extend(["--prefix", prefix.as_str()]);
    }
    if let Some(t) = q.token.as_deref().filter(|t| !t.is_empty()) {
        args.extend(["--starting-token", t]);
    }
    let v = svc.run_json(a, q.region.as_deref(), &args).await?;
    Ok(normalize_objects(&v, &prefix))
}

pub async fn head_object(
    svc: &AwsService,
    a: &AwsAccountRow,
    bucket: &str,
    key: &str,
    region: Option<&str>,
) -> Result<HeadResp> {
    validate_bucket(bucket)?;
    validate_key(key)?;
    let v = svc
        .run_json(
            a,
            region,
            &["s3api", "head-object", "--bucket", bucket, "--key", key],
        )
        .await?;
    Ok(normalize_head(key, &v))
}

/// Ranged `get-object` into a temp file under `<data_dir>/tmp` (the CLI's
/// `get-object` insists on an outfile), then classify + read.
pub async fn preview(
    svc: &AwsService,
    a: &AwsAccountRow,
    bucket: &str,
    key: &str,
    max_bytes: Option<u64>,
    region: Option<&str>,
) -> Result<PreviewResp> {
    validate_bucket(bucket)?;
    validate_key(key)?;
    let max = max_bytes.unwrap_or(PREVIEW_DEFAULT).clamp(1, PREVIEW_CAP);
    let head = head_object(svc, a, bucket, key, region).await?;
    if !is_texty(head.content_type.as_deref(), key) {
        return Ok(PreviewResp {
            text: None,
            truncated: false,
            content_type: head.content_type,
            binary: true,
        });
    }
    // Scratch file at an Otto-owned location: <data_dir>/tmp/<fresh ULID>.
    // `bucket`/`key` only ever travel as argv to the CLI, never into the path.
    let tmp_dir = crate::paths::owned_dir(&svc.data_dir, "tmp")?;
    let tmp = crate::paths::owned_file(&tmp_dir, &otto_core::new_id(), "")?;
    let tmp_s = tmp.to_string_lossy().into_owned();
    let range = format!("bytes=0-{}", max - 1);
    let res = svc
        .run(
            a,
            region,
            &[
                "s3api",
                "get-object",
                "--bucket",
                bucket,
                "--key",
                key,
                "--range",
                &range,
                &tmp_s,
            ],
        )
        .await;
    let bytes = std::fs::read(&tmp).unwrap_or_default();
    let _ = std::fs::remove_file(&tmp);
    res?;
    Ok(preview_from_bytes(
        &bytes,
        head.content_type,
        Some(head.size),
        key,
    ))
}

/// A running `aws s3 cp s3://b/k -` whose stdout is streamed to the client;
/// dropping the stream kills the child (`kill_on_drop`).
pub struct DownloadStream {
    pub head: HeadResp,
    pub body: axum::body::Body,
}

pub async fn download(
    svc: &AwsService,
    a: &AwsAccountRow,
    bucket: &str,
    key: &str,
    region: Option<&str>,
) -> Result<DownloadStream> {
    validate_bucket(bucket)?;
    validate_key(key)?;
    let head = head_object(svc, a, bucket, key, region).await?;
    if head.size > DOWNLOAD_CAP {
        return Err(Error::PayloadTooLarge(format!(
            "object is {} bytes; the in-app download cap is 2 GiB — use the CLI",
            head.size
        )));
    }
    let (bin, env) = svc.bin_and_env(a, region).await?;
    let uri = format!("s3://{bucket}/{key}");
    let mut child = tokio::process::Command::new(&bin)
        .args(["s3", "cp", &uri, "-", "--no-progress"])
        .env_remove("AWS_PROFILE") // same rule as `cli::run_raw`
        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| Error::Internal(format!("spawn aws s3 cp: {e}")))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Internal("no stdout on aws s3 cp".into()))?;
    // The child rides along in the unfold state so it is killed (kill_on_drop)
    // exactly when the response body is dropped — client disconnect included.
    let stream = futures_util::stream::unfold((child, stdout), |(child, mut stdout)| async move {
        use tokio::io::AsyncReadExt;
        let mut buf = vec![0u8; 64 * 1024];
        match stdout.read(&mut buf).await {
            Ok(0) | Err(_) => None,
            Ok(n) => {
                buf.truncate(n);
                Some((
                    Ok::<_, std::io::Error>(bytes::Bytes::from(buf)),
                    (child, stdout),
                ))
            }
        }
    });
    svc.repo.touch_used(&a.id).await;
    Ok(DownloadStream {
        head,
        body: axum::body::Body::from_stream(stream),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_normalize() {
        let v: Value = serde_json::from_str(
            r#"{"Buckets": [{"Name": "logs-prod", "CreationDate": "2021-03-01T10:00:00+00:00"}, {"Name": "assets"}], "Owner": {"ID": "abc"}}"#,
        )
        .unwrap();
        let b = normalize_buckets(&v);
        assert_eq!(b.len(), 2);
        assert_eq!(b[0].name, "logs-prod");
        assert_eq!(
            b[0].creation_date.as_deref(),
            Some("2021-03-01T10:00:00+00:00")
        );
        assert!(b[1].creation_date.is_none());
        assert!(normalize_buckets(&serde_json::json!({})).is_empty());
    }

    #[test]
    fn objects_normalize_prefixes_marker_and_token() {
        let v: Value = serde_json::from_str(
            r#"{
              "Contents": [
                {"Key": "logs/", "LastModified": "2024-01-01T00:00:00+00:00", "ETag": "\"d41d8\"", "Size": 0, "StorageClass": "STANDARD"},
                {"Key": "logs/app.log", "LastModified": "2024-01-02T00:00:00+00:00", "ETag": "\"abc\"", "Size": 1234, "StorageClass": "STANDARD_IA"}
              ],
              "CommonPrefixes": [{"Prefix": "logs/2024/"}, {"Prefix": "logs/2025/"}],
              "IsTruncated": true,
              "NextToken": "eyJDb250aW51YXRpb25Ub2tlbiI6IG51bGx9"
            }"#,
        )
        .unwrap();
        let r = normalize_objects(&v, "logs/");
        assert_eq!(r.prefixes, vec!["logs/2024/", "logs/2025/"]);
        assert_eq!(r.objects.len(), 1, "directory marker dropped");
        assert_eq!(r.objects[0].key, "logs/app.log");
        assert_eq!(r.objects[0].size, 1234);
        assert_eq!(r.objects[0].etag.as_deref(), Some("abc"));
        assert_eq!(r.objects[0].storage_class.as_deref(), Some("STANDARD_IA"));
        assert!(r.is_truncated);
        assert_eq!(
            r.next_token.as_deref(),
            Some("eyJDb250aW51YXRpb25Ub2tlbiI6IG51bGx9")
        );

        let last: Value =
            serde_json::json!({"Contents": [{"Key": "a.txt", "Size": 1}], "IsTruncated": false});
        let r = normalize_objects(&last, "");
        assert!(!r.is_truncated && r.next_token.is_none());
        assert_eq!(r.objects[0].key, "a.txt");
    }

    #[test]
    fn head_normalize() {
        let v: Value = serde_json::from_str(
            r#"{"AcceptRanges": "bytes", "LastModified": "2024-05-05T00:00:00+00:00", "ContentLength": 42, "ETag": "\"e\"", "ContentType": "application/json", "Metadata": {"owner": "x"}}"#,
        )
        .unwrap();
        let h = normalize_head("k.json", &v);
        assert_eq!(h.size, 42);
        assert_eq!(h.content_type.as_deref(), Some("application/json"));
        assert_eq!(h.etag.as_deref(), Some("e"));
        assert_eq!(h.metadata["owner"], "x");
        assert!(h.storage_class.is_none());
    }

    #[test]
    fn texty_detection() {
        assert!(is_texty(Some("text/plain"), "x"));
        assert!(is_texty(Some("application/json; charset=utf-8"), "x"));
        assert!(is_texty(Some("application/octet-stream"), "app.log"));
        assert!(is_texty(Some("binary/octet-stream"), "data.csv"));
        assert!(is_texty(None, "conf.yaml"));
        assert!(!is_texty(Some("application/octet-stream"), "app.log.gz"));
        assert!(!is_texty(Some("image/png"), "shot.png"));
        assert!(
            !is_texty(Some("application/zip"), "a.json"),
            "declared binary wins over the extension"
        );
    }

    #[test]
    fn preview_marks_binary_and_truncation() {
        let p = preview_from_bytes(b"hello", Some("text/plain".into()), Some(10), "a.txt");
        assert_eq!(p.text.as_deref(), Some("hello"));
        assert!(p.truncated && !p.binary);
        let p = preview_from_bytes(b"he\0llo", Some("text/plain".into()), Some(6), "a.txt");
        assert!(p.binary && p.text.is_none());
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["binary"], true);
        let p = preview_from_bytes(b"x", Some("text/plain".into()), Some(1), "a.txt");
        assert!(!p.truncated);
        assert!(serde_json::to_value(&p).unwrap().get("binary").is_none());
    }

    #[test]
    fn validators() {
        assert!(validate_bucket("my-bucket.01").is_ok());
        assert!(validate_bucket("My_Bucket").is_err());
        assert!(validate_bucket("ab").is_err());
        assert!(validate_key("a/b c.txt").is_ok());
        assert!(validate_key("").is_err());
        assert!(validate_key("a\nb").is_err());
    }
}
