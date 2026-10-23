//! `multipart/form-data` bodies as the UI encodes them.

use super::*;

/// One multipart/form-data field as encoded by the UI. A `file` field carries
/// the file's bytes base64-encoded in `value`.
#[derive(Deserialize)]
pub(crate) struct MultipartField {
    #[serde(default)]
    key: String,
    #[serde(default, rename = "type")]
    field_type: String,
    #[serde(default)]
    value: String,
    #[serde(default)]
    filename: String,
    #[serde(default)]
    missing_content: bool,
}

/// Build a reqwest multipart form from the request body. Accepts either a JSON
/// array of [`MultipartField`]s or the legacy `k=v&...` text encoding.
pub(crate) fn build_multipart(
    body: &str,
    vars: &serde_json::Map<String, Value>,
) -> Result<reqwest::multipart::Form, String> {
    use base64::engine::general_purpose::STANDARD as B64;
    use base64::Engine;

    let mut form = reqwest::multipart::Form::new();
    if body.trim_start().starts_with('[') {
        let fields = serde_json::from_str::<Vec<MultipartField>>(body)
            .map_err(|e| format!("Invalid multipart fields: {e}"))?;
        for f in fields {
            let key = substitute(&f.key, vars);
            if key.trim().is_empty() {
                continue;
            }
            if f.field_type == "file" {
                if f.missing_content {
                    return Err(format!("Reselect the file for multipart field '{key}': history does not retain file contents"));
                }
                let bytes = B64
                    .decode(f.value.trim())
                    .map_err(|_| format!("Invalid base64 for multipart field '{key}'"))?;
                let filename = if f.filename.is_empty() {
                    "file".to_string()
                } else {
                    substitute(&f.filename, vars)
                };
                let mut part = reqwest::multipart::Part::bytes(bytes).file_name(filename);
                if let Some(mime) = guess_mime(&f.filename) {
                    // guess_mime only returns valid static MIME strings.
                    part = part.mime_str(mime).expect("valid static mime");
                }
                form = form.part(key, part);
            } else {
                form = form.text(key, substitute(&f.value, vars));
            }
        }
        return Ok(form);
    }
    // Legacy `k=v&...` (percent-encoded) text fields.
    for pair in body.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = match pair.split_once('=') {
            Some((k, v)) => (percent_decode(k), percent_decode(v)),
            None => (percent_decode(pair), String::new()),
        };
        let key = substitute(&k, vars);
        if key.trim().is_empty() {
            continue;
        }
        form = form.text(key, substitute(&v, vars));
    }
    Ok(form)
}

/// Minimal filename-extension → MIME guess for common upload types.
pub(crate) fn guess_mime(filename: &str) -> Option<&'static str> {
    let ext = filename.rsplit('.').next()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "json" => "application/json",
        "csv" => "text/csv",
        "txt" => "text/plain",
        "xml" => "application/xml",
        "zip" => "application/zip",
        "gz" => "application/gzip",
        "html" | "htm" => "text/html",
        _ => return None,
    })
}
