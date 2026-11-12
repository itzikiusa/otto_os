//! TLS material helpers shared by drivers.
//!
//! A `TlsConfig` PEM field holds EITHER inline PEM (what imports store) OR a
//! file path (what the connection form's "CA certificate path" field and its
//! file picker send). [`resolve_pem`] is the one place that tells them apart,
//! so every driver accepts both:
//!
//! - inline PEM (the value contains a `-----BEGIN` header) is used as is;
//! - anything else is a path: a leading `~/` is expanded, the file is read and
//!   must contain PEM, and a missing / unreadable / non-PEM file fails with a
//!   user-facing error that names the path.
//!
//! Engines consume TLS differently: sqlx (MySQL/Postgres) and the mongodb
//! crate want *file paths*, while others can take in-memory PEM
//! ([`pem_text`]). A path value is handed to the client as the ORIGINAL file;
//! only inline PEM is persisted, to an owner-only (0600) temp file named by a
//! content hash, so repeated calls reuse the same file.

use std::io::Write;
use std::path::{Path, PathBuf};

use otto_core::{Error, Result};

use crate::types::TlsConfig;

/// Largest PEM file read from a path. A CA bundle (the whole Mozilla set is
/// ~250 KB) fits easily; anything bigger is not a certificate file.
const MAX_PEM_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// Where a TLS field's PEM came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PemSource {
    /// The field held the PEM itself.
    Inline(String),
    /// The field held a path; `path` is the expanded path, `pem` its contents.
    File { path: PathBuf, pem: String },
}

impl PemSource {
    pub fn pem(&self) -> &str {
        match self {
            PemSource::Inline(pem) | PemSource::File { pem, .. } => pem,
        }
    }
}

/// Human label for a `TlsConfig` field name, for error messages.
fn label(field: &str) -> &str {
    match field {
        "ca_cert" => "CA certificate",
        "client_cert" => "client certificate",
        "client_key" => "client key",
        other => other,
    }
}

/// True when `value` is inline PEM rather than a path.
pub fn is_inline_pem(value: &str) -> bool {
    value.contains("-----BEGIN")
}

/// Resolve one `TlsConfig` PEM field (`field` is its name, used in errors):
/// inline PEM is returned as is (re-wrapped first when it arrived on one line,
/// see [`rewrap_pem`]), otherwise the value is read as a file path.
pub fn resolve_pem(field: &str, value: &str) -> Result<PemSource> {
    let what = label(field);
    if is_inline_pem(value) {
        let pem = rewrap_pem(value).map_err(|why| Error::Invalid(format!("TLS {what}: {why}")))?;
        return Ok(PemSource::Inline(pem.into_owned()));
    }
    let shown = value.trim();
    let path = otto_core::paths::expand_home_strict(shown)
        .map_err(|e| Error::Invalid(format!("TLS {what}: {e}")))?;
    if !path.is_absolute() {
        return Err(Error::Invalid(format!(
            "TLS {what}: {shown:?} is neither PEM text nor an absolute path \
             (use /full/path/file.pem or ~/path/file.pem)"
        )));
    }
    let pem = read_pem_file(&path)
        .map_err(|why| Error::Invalid(format!("TLS {what} file {shown:?}: {why}")))?;
    Ok(PemSource::File { path, pem })
}

/// Re-wrap inline PEM that lost its line breaks. Browsers strip newlines from
/// text pasted into a single-line input, so a certificate arrives as
/// `-----BEGIN CERTIFICATE-----MIIB…-----END CERTIFICATE-----`, which PEM
/// parsers reject. Every block (a chain holds several) becomes its header line,
/// the base64 body in 64-character lines and its footer line; text outside the
/// blocks is dropped. A value that already has line breaks is returned as is.
/// Anything but base64 (and spaces) between a block's markers is an error.
fn rewrap_pem(value: &str) -> std::result::Result<std::borrow::Cow<'_, str>, String> {
    const BEGIN: &str = "-----BEGIN ";
    if value.contains('\n') {
        return Ok(std::borrow::Cow::Borrowed(value));
    }
    let mut out = String::new();
    let mut rest = value;
    while let Some(start) = rest.find(BEGIN) {
        let after = &rest[start + BEGIN.len()..];
        let label_end = after
            .find("-----")
            .ok_or("a -----BEGIN marker is not closed")?;
        let label = &after[..label_end];
        if label.is_empty()
            || !label
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == ' ')
        {
            return Err(format!("{label:?} is not a PEM block label"));
        }
        let tail = &after[label_end + "-----".len()..];
        let footer = format!("-----END {label}-----");
        let body_end = tail
            .find(&footer)
            .ok_or_else(|| format!("the {label} block has no {footer} line"))?;
        let body: String = tail[..body_end]
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        let base64 = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=');
        if body.is_empty() || !body.bytes().all(base64) {
            return Err(format!(
                "the {label} block holds something other than base64 between its markers"
            ));
        }
        out.push_str(&format!("{BEGIN}{label}-----\n"));
        // All ASCII (checked above), so every 64-byte chunk is a whole str.
        for line in body.as_bytes().chunks(64) {
            out.push_str(&String::from_utf8_lossy(line));
            out.push('\n');
        }
        out.push_str(&footer);
        out.push('\n');
        rest = &tail[body_end + footer.len()..];
    }
    Ok(std::borrow::Cow::Owned(out))
}

/// The PEM text of a field, whichever way it was given — for clients that take
/// in-memory PEM (Redis, ClickHouse native/HTTP).
pub fn pem_text(field: &str, value: &str) -> Result<String> {
    Ok(match resolve_pem(field, value)? {
        PemSource::Inline(pem) | PemSource::File { pem, .. } => pem,
    })
}

fn read_pem_file(path: &Path) -> std::result::Result<String, String> {
    let meta = std::fs::metadata(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => "file not found".to_string(),
        std::io::ErrorKind::PermissionDenied => "permission denied".to_string(),
        _ => format!("cannot read: {e}"),
    })?;
    if !meta.is_file() {
        return Err("not a regular file".into());
    }
    if meta.len() > MAX_PEM_FILE_BYTES {
        return Err(format!(
            "too large for a PEM file ({} bytes, limit {MAX_PEM_FILE_BYTES})",
            meta.len()
        ));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read: {e}"))?;
    match String::from_utf8(bytes) {
        Ok(text) if is_inline_pem(&text) => Ok(text),
        _ => Err("does not contain PEM (expected a -----BEGIN … block; \
                  convert a DER file with `openssl x509 -inform der -in FILE -out FILE.pem`)"
            .into()),
    }
}

/// Persist inline PEM to `$TMPDIR/otto-dbv-<prefix>-<sha256>.pem`, owner-only.
///
/// The file is created with mode 0600 from the start (a fresh `tempfile`,
/// which is 0600, renamed into place) — never written world-readable and then
/// tightened. An existing file is reused only when it is already owner-only;
/// one left world-readable by an older build is atomically replaced.
fn temp_pem(prefix: &str, pem: &str) -> Result<PathBuf> {
    temp_pem_in(&std::env::temp_dir(), prefix, pem)
}

fn temp_pem_in(dir: &Path, prefix: &str, pem: &str) -> Result<PathBuf> {
    use sha2::{Digest, Sha256};
    use std::os::unix::fs::PermissionsExt;
    let digest = hex::encode(Sha256::digest(pem.as_bytes()));
    let path = dir.join(format!("otto-dbv-{prefix}-{}.pem", &digest[..32]));
    if let Ok(meta) = std::fs::symlink_metadata(&path) {
        if meta.is_file() && meta.permissions().mode() & 0o077 == 0 {
            return Ok(path);
        }
    }
    let io = |e: std::io::Error| Error::Internal(format!("write tls pem: {e}"));
    let mut tmp = tempfile::Builder::new()
        .prefix(".otto-dbv-")
        .suffix(".pem.tmp")
        .tempfile_in(dir)
        .map_err(io)?;
    tmp.write_all(pem.as_bytes()).map_err(io)?;
    tmp.flush().map_err(io)?;
    tmp.persist(&path).map_err(|e| io(e.error))?;
    Ok(path)
}

/// The on-disk path for one field: the original file for a path value, an
/// owner-only temp file for inline PEM.
fn field_path(prefix: &str, src: &PemSource) -> Result<PathBuf> {
    match src {
        PemSource::File { path, .. } => Ok(path.clone()),
        PemSource::Inline(pem) => temp_pem(prefix, pem),
    }
}

/// On-disk paths for the TLS material in a [`TlsConfig`]. Absent fields are
/// `None`. `client_pair` is a combined cert+key file (some clients want one).
#[derive(Debug, Clone, Default)]
pub struct TlsFiles {
    pub ca: Option<PathBuf>,
    pub client_cert: Option<PathBuf>,
    pub client_key: Option<PathBuf>,
    pub client_pair: Option<PathBuf>,
}

impl TlsFiles {
    /// Resolve every PEM field of `cfg` (inline or path) to a file path.
    pub fn materialize(cfg: &TlsConfig) -> Result<TlsFiles> {
        let get = |field: &str, v: &Option<String>| -> Result<Option<PemSource>> {
            v.as_deref()
                .filter(|s| !s.trim().is_empty())
                .map(|s| resolve_pem(field, s))
                .transpose()
        };
        let ca = get("ca_cert", &cfg.ca_cert)?;
        let cert = get("client_cert", &cfg.client_cert)?;
        let key = get("client_key", &cfg.client_key)?;
        let mut files = TlsFiles::default();
        if let Some(src) = &ca {
            files.ca = Some(field_path("ca", src)?);
        }
        if let Some(src) = &cert {
            files.client_cert = Some(field_path("cert", src)?);
        }
        if let Some(src) = &key {
            files.client_key = Some(field_path("key", src)?);
        }
        if let (Some(cert), Some(key)) = (&cert, &key) {
            // The combined file holds the private key, so it is always a 0600
            // temp file even when both halves came from paths.
            let combined = format!("{}\n{}\n", cert.pem().trim_end(), key.pem().trim_end());
            files.client_pair = Some(temp_pem("pair", &combined)?);
        }
        Ok(files)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    const CA: &str = "-----BEGIN CERTIFICATE-----\nMIIBfixture\n-----END CERTIFICATE-----\n";
    const KEY: &str = "-----BEGIN PRIVATE KEY-----\nMIIEfixture\n-----END PRIVATE KEY-----\n";

    fn msg(e: Error) -> String {
        match e {
            Error::Invalid(m) => m,
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn inline_pem_is_used_verbatim() {
        assert_eq!(
            resolve_pem("ca_cert", CA).unwrap(),
            PemSource::Inline(CA.to_string())
        );
        assert_eq!(pem_text("ca_cert", CA).unwrap(), CA);
    }

    /// A 100-character base64 body: wraps to one 64- and one 36-char line.
    fn body(c: char) -> String {
        std::iter::repeat_n(c, 100).collect()
    }

    fn wrapped(label: &str, c: char) -> String {
        let b = body(c);
        format!(
            "-----BEGIN {label}-----\n{}\n{}\n-----END {label}-----\n",
            &b[..64],
            &b[64..]
        )
    }

    #[test]
    fn single_line_pem_is_rewrapped() {
        let one = format!(
            "-----BEGIN CERTIFICATE-----{}-----END CERTIFICATE-----",
            body('A')
        );
        assert_eq!(
            pem_text("ca_cert", &one).unwrap(),
            wrapped("CERTIFICATE", 'A')
        );
        // Some browsers turn the line breaks into spaces instead.
        let spaced = format!(
            "-----BEGIN PRIVATE KEY----- {} {} -----END PRIVATE KEY-----",
            &body('k')[..50],
            &body('k')[50..]
        );
        assert_eq!(
            pem_text("client_key", &spaced).unwrap(),
            wrapped("PRIVATE KEY", 'k')
        );
    }

    #[test]
    fn single_line_chain_rewraps_every_block() {
        let chain = format!(
            "-----BEGIN CERTIFICATE-----{}-----END CERTIFICATE----- \
             -----BEGIN CERTIFICATE-----{}-----END CERTIFICATE-----",
            body('A'),
            body('B')
        );
        assert_eq!(
            pem_text("ca_cert", &chain).unwrap(),
            format!(
                "{}{}",
                wrapped("CERTIFICATE", 'A'),
                wrapped("CERTIFICATE", 'B')
            )
        );
    }

    #[test]
    fn well_formed_pem_is_left_unchanged() {
        let pem = format!(
            "{}{}",
            wrapped("CERTIFICATE", 'A'),
            wrapped("CERTIFICATE", 'B')
        );
        assert_eq!(pem_text("ca_cert", &pem).unwrap(), pem);
        assert_eq!(pem_text("ca_cert", CA).unwrap(), CA);
    }

    #[test]
    fn garbage_between_single_line_markers_is_rejected() {
        for bad in [
            "-----BEGIN CERTIFICATE-----not base64!-----END CERTIFICATE-----",
            "-----BEGIN CERTIFICATE----------END CERTIFICATE-----",
            "-----BEGIN CERTIFICATE-----QUJD",
            "-----BEGIN CERTIFICATE-----QUJD-----END PRIVATE KEY-----",
            "-----BEGIN cert-----QUJD-----END cert-----",
        ] {
            let m = msg(resolve_pem("ca_cert", bad).unwrap_err());
            assert!(m.starts_with("TLS CA certificate: "), "{bad}: {m}");
        }
    }

    #[test]
    fn absolute_path_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("ca.pem");
        std::fs::write(&file, CA).unwrap();
        let src = resolve_pem("ca_cert", file.to_str().unwrap()).unwrap();
        assert_eq!(
            src,
            PemSource::File {
                path: file.clone(),
                pem: CA.to_string()
            }
        );
    }

    #[test]
    fn tilde_path_is_expanded_against_home() {
        let home = PathBuf::from(std::env::var("HOME").expect("HOME set in tests"));
        let dir = tempfile::tempdir_in(&home).unwrap();
        let name = dir
            .path()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        std::fs::write(dir.path().join("ca.pem"), CA).unwrap();
        let src = resolve_pem("ca_cert", &format!("~/{name}/ca.pem")).unwrap();
        match src {
            PemSource::File { path, pem } => {
                assert_eq!(path, home.join(&name).join("ca.pem"));
                assert_eq!(pem, CA);
            }
            other => panic!("expected a file source, got {other:?}"),
        }
    }

    #[test]
    fn missing_file_names_the_path() {
        let m = msg(resolve_pem("ca_cert", "/nonexistent/otto/ca.pem").unwrap_err());
        assert!(m.contains("/nonexistent/otto/ca.pem"), "{m}");
        assert!(m.contains("CA certificate"), "{m}");
        assert!(m.contains("not found"), "{m}");
    }

    #[test]
    fn non_pem_file_is_rejected_with_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("ca.der");
        std::fs::write(&file, [0x30u8, 0x82, 0x01, 0xff, 0x00]).unwrap();
        let m = msg(resolve_pem("client_cert", file.to_str().unwrap()).unwrap_err());
        assert!(m.contains("ca.der"), "{m}");
        assert!(m.contains("does not contain PEM"), "{m}");
        let txt = dir.path().join("notes.txt");
        std::fs::write(&txt, "hello").unwrap();
        assert!(resolve_pem("ca_cert", txt.to_str().unwrap()).is_err());
    }

    #[test]
    fn relative_and_other_user_paths_are_rejected() {
        assert!(msg(resolve_pem("ca_cert", "certs/ca.pem").unwrap_err()).contains("absolute"));
        assert!(resolve_pem("ca_cert", "~bob/ca.pem").is_err());
    }

    #[test]
    fn materialize_passes_path_values_through_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let ca = dir.path().join("ca.pem");
        std::fs::write(&ca, CA).unwrap();
        let cfg = TlsConfig {
            ca_cert: Some(ca.to_string_lossy().into_owned()),
            ..TlsConfig::default()
        };
        let files = TlsFiles::materialize(&cfg).unwrap();
        assert_eq!(files.ca.as_deref(), Some(ca.as_path()));
        assert!(files.client_pair.is_none());
    }

    #[test]
    fn materialize_fails_on_a_missing_path() {
        let cfg = TlsConfig {
            client_key: Some("/nonexistent/otto/key.pem".into()),
            ..TlsConfig::default()
        };
        let m = msg(TlsFiles::materialize(&cfg).unwrap_err());
        assert!(m.contains("client key") && m.contains("/nonexistent/otto/key.pem"));
    }

    #[test]
    fn inline_pem_temp_files_are_owner_only_and_reused() {
        let dir = tempfile::tempdir().unwrap();
        let a = temp_pem_in(dir.path(), "key", KEY).unwrap();
        let mode = std::fs::metadata(&a).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(std::fs::read_to_string(&a).unwrap(), KEY);
        assert_eq!(temp_pem_in(dir.path(), "key", KEY).unwrap(), a);
    }

    #[test]
    fn world_readable_leftover_is_replaced_owner_only() {
        let dir = tempfile::tempdir().unwrap();
        let first = temp_pem_in(dir.path(), "key", KEY).unwrap();
        std::fs::set_permissions(&first, std::fs::Permissions::from_mode(0o644)).unwrap();
        let again = temp_pem_in(dir.path(), "key", KEY).unwrap();
        assert_eq!(again, first);
        let mode = std::fs::metadata(&again).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(std::fs::read_to_string(&again).unwrap(), KEY);
    }
}
