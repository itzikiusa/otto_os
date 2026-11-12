//! The connection form's TLS block (`cfg.tls`) applied to MongoDB — for the
//! native driver's `ClientOptions` and for the mongosh script URI — on BOTH the
//! host/port path and the `conn_string` path (moved out of `mongodb.rs`).
//!
//! Precedence with a connection string: **the string wins**. Anything the
//! string states explicitly (`tls`/`ssl`, `tlsCAFile`, `tlsCertificateKeyFile`,
//! `tlsAllowInvalidCertificates`/`tlsInsecure`) is kept; the form only fills
//! what the string leaves unset. TLS itself is turned on by the form only when:
//! - the string already enables it (`tls=true`, `ssl=true`, `mongodb+srv`) and
//!   the form's mode is not Disabled — the form then contributes CA / client
//!   certificate / verify; or
//! - the string says nothing about TLS and the form's mode is **Required**.
//!   "Preferred" has no MongoDB equivalent (the driver has no opportunistic
//!   TLS), so it never switches a silent string to TLS.
//!
//! A string with `tls=false` / `ssl=false` keeps TLS off whatever the form says.
//! The form's `server_name` cannot be honoured: the mongodb crate always
//! verifies against (and sends SNI for) each server's own hostname.

use mongodb::options::{ClientOptions, Tls, TlsOptions};
use otto_core::Result;

use crate::tls::TlsFiles;
use crate::types::TlsConfig;

/// Merge the form's TLS into options parsed from a connection string.
pub(super) fn apply_form_tls(opts: &mut ClientOptions, tls: &TlsConfig) -> Result<()> {
    let act = match &opts.tls {
        Some(Tls::Disabled) => false,
        Some(Tls::Enabled(_)) => tls.enabled(),
        None => tls.required(),
    };
    if act {
        // Materialized only when used: a bad CA path must not fail a
        // connection whose string turned TLS off.
        let files = TlsFiles::materialize(tls)?;
        opts.tls = Some(merge_tls(opts.tls.take(), tls, files));
    }
    Ok(())
}

/// TLS for the host/port path: the form is the only source.
pub(super) fn form_tls(tls: &TlsConfig) -> Result<Option<Tls>> {
    if !tls.enabled() {
        return Ok(None);
    }
    Ok(Some(merge_tls(None, tls, TlsFiles::materialize(tls)?)))
}

/// Fill the unset fields of `current` (if TLS was already enabled) from the
/// form. Pure: the caller resolves `files`.
fn merge_tls(current: Option<Tls>, tls: &TlsConfig, files: TlsFiles) -> Tls {
    let mut o = match current {
        Some(Tls::Enabled(o)) => o,
        _ => TlsOptions::default(),
    };
    if o.allow_invalid_certificates.is_none() {
        o.allow_invalid_certificates = Some(!tls.verify);
    }
    if o.ca_file_path.is_none() {
        o.ca_file_path = files.ca;
    }
    if o.cert_key_file_path.is_none() {
        o.cert_key_file_path = files.client_pair.or(files.client_cert);
    }
    Tls::Enabled(o)
}

/// The lowercase option keys and the TLS state a mongodb URI states itself.
fn uri_tls_state(uri: &str) -> (Vec<String>, Option<bool>) {
    let keys: Vec<(String, String)> = uri
        .split_once('?')
        .map(|(_, q)| {
            q.split('&')
                .filter(|kv| !kv.is_empty())
                .map(|kv| {
                    let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
                    (k.to_ascii_lowercase(), v.to_ascii_lowercase())
                })
                .collect()
        })
        .unwrap_or_default();
    let mut state = uri
        .get(..14)
        .is_some_and(|s| s.eq_ignore_ascii_case("mongodb+srv://"))
        .then_some(true);
    for (k, v) in &keys {
        if k == "tls" || k == "ssl" {
            state = Some(v == "true");
        }
    }
    (keys.into_iter().map(|(k, _)| k).collect(), state)
}

/// The URI query parameters that carry the form's TLS to mongosh. `uri` is
/// the connection string (empty for the host/port path, where the form's
/// enabled modes all turn TLS on, as the native path does).
pub(super) fn mongosh_tls_params(uri: &str, tls: &TlsConfig) -> Result<Vec<String>> {
    let from_string = !uri.is_empty();
    let (keys, state) = uri_tls_state(uri);
    let act = match state {
        Some(false) => false,
        Some(true) => tls.enabled(),
        None if from_string => tls.required(),
        None => tls.enabled(),
    };
    if !act {
        return Ok(Vec::new());
    }
    let has = |k: &str| keys.iter().any(|x| x == k);
    let files = TlsFiles::materialize(tls)?;
    let mut out = Vec::new();
    if state != Some(true) {
        out.push("tls=true".to_string());
    }
    if !tls.verify && !has("tlsallowinvalidcertificates") && !has("tlsinsecure") {
        out.push("tlsAllowInvalidCertificates=true".to_string());
    }
    if let Some(ca) = files.ca.filter(|_| !has("tlscafile")) {
        out.push(format!("tlsCAFile={}", super::pct(&ca.to_string_lossy())));
    }
    if let Some(pair) = files
        .client_pair
        .or(files.client_cert)
        .filter(|_| !has("tlscertificatekeyfile"))
    {
        out.push(format!(
            "tlsCertificateKeyFile={}",
            super::pct(&pair.to_string_lossy())
        ));
    }
    Ok(out)
}

/// A connection string with the form's TLS appended for mongosh.
pub(super) fn conn_string_with_tls(uri: String, tls: &TlsConfig) -> Result<String> {
    let params = mongosh_tls_params(&uri, tls)?;
    if params.is_empty() {
        return Ok(uri);
    }
    let mut uri = uri;
    if !uri.contains('?') {
        // `mongodb://host:27017` needs the `/` before options.
        let authority_start = uri.find("://").map_or(0, |i| i + 3);
        if !uri[authority_start..].contains('/') {
            uri.push('/');
        }
        uri.push('?');
    } else if !uri.ends_with('?') && !uri.ends_with('&') {
        uri.push('&');
    }
    uri.push_str(&params.join("&"));
    Ok(uri)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TlsMode;
    use std::path::PathBuf;

    const CA: &str = "-----BEGIN CERTIFICATE-----\nMIIBfixture\n-----END CERTIFICATE-----\n";

    fn form(mode: TlsMode, verify: bool) -> TlsConfig {
        TlsConfig {
            mode,
            verify,
            ..TlsConfig::default()
        }
    }

    fn enabled(t: &Option<Tls>) -> &TlsOptions {
        match t {
            Some(Tls::Enabled(o)) => o,
            other => panic!("expected TLS enabled, got {other:?}"),
        }
    }

    fn files(ca: &str, pair: &str) -> TlsFiles {
        TlsFiles {
            ca: Some(PathBuf::from(ca)),
            client_pair: Some(PathBuf::from(pair)),
            ..TlsFiles::default()
        }
    }

    #[test]
    fn form_fills_what_the_string_left_unset() {
        let mut from_uri = TlsOptions::default();
        from_uri.ca_file_path = Some(PathBuf::from("/uri/ca.pem"));
        let merged = merge_tls(
            Some(Tls::Enabled(from_uri)),
            &form(TlsMode::Required, false),
            files("/form/ca.pem", "/form/pair.pem"),
        );
        let o = enabled(&Some(merged)).clone();
        assert_eq!(o.ca_file_path, Some(PathBuf::from("/uri/ca.pem")));
        assert_eq!(o.cert_key_file_path, Some(PathBuf::from("/form/pair.pem")));
        assert_eq!(o.allow_invalid_certificates, Some(true));
    }

    #[test]
    fn the_strings_own_verify_setting_wins() {
        let mut from_uri = TlsOptions::default();
        from_uri.allow_invalid_certificates = Some(false);
        let merged = merge_tls(
            Some(Tls::Enabled(from_uri)),
            &form(TlsMode::Required, false),
            TlsFiles::default(),
        );
        assert_eq!(
            enabled(&Some(merged)).allow_invalid_certificates,
            Some(false)
        );
    }

    #[test]
    fn string_that_disables_tls_is_left_alone() {
        let mut opts = ClientOptions::default();
        opts.tls = Some(Tls::Disabled);
        // A bad CA path is never even read when TLS stays off.
        let mut tls = form(TlsMode::Required, true);
        tls.ca_cert = Some("/nonexistent/otto/ca.pem".into());
        apply_form_tls(&mut opts, &tls).unwrap();
        assert_eq!(opts.tls, Some(Tls::Disabled));
    }

    #[test]
    fn silent_string_is_switched_on_only_by_required() {
        let mut opts = ClientOptions::default();
        apply_form_tls(&mut opts, &form(TlsMode::Preferred, true)).unwrap();
        assert!(opts.tls.is_none());
        apply_form_tls(&mut opts, &form(TlsMode::Disabled, true)).unwrap();
        assert!(opts.tls.is_none());
        apply_form_tls(&mut opts, &form(TlsMode::Required, true)).unwrap();
        assert_eq!(enabled(&opts.tls).allow_invalid_certificates, Some(false));
    }

    #[test]
    fn string_enabled_tls_with_verify_true_stays_strict() {
        // The SOCKS-tunnelled Atlas shape: tls=true in the string, form
        // verify:true, no CA — must stay a strict, system-roots handshake.
        let mut opts = ClientOptions::default();
        opts.tls = Some(Tls::Enabled(TlsOptions::default()));
        apply_form_tls(&mut opts, &form(TlsMode::Required, true)).unwrap();
        let o = enabled(&opts.tls);
        assert_eq!(o.allow_invalid_certificates, Some(false));
        assert!(o.ca_file_path.is_none() && o.cert_key_file_path.is_none());
    }

    #[test]
    fn host_port_path_enables_for_any_enabled_mode() {
        assert!(form_tls(&form(TlsMode::Disabled, true)).unwrap().is_none());
        let t = form_tls(&form(TlsMode::Preferred, false)).unwrap();
        assert_eq!(enabled(&t).allow_invalid_certificates, Some(true));
    }

    #[test]
    fn mongosh_params_respect_the_string() {
        let tls = form(TlsMode::Required, false);
        // Silent string → TLS turned on with the form's verify.
        assert_eq!(
            mongosh_tls_params("mongodb://h:1/db", &tls).unwrap(),
            vec!["tls=true", "tlsAllowInvalidCertificates=true"]
        );
        // tls=true already there, and its own insecure flag wins.
        assert!(
            mongosh_tls_params("mongodb://h:1/db?ssl=true&tlsInsecure=true", &tls)
                .unwrap()
                .is_empty()
        );
        // SRV implies TLS.
        assert_eq!(
            mongosh_tls_params("mongodb+srv://c.example.net/db", &tls).unwrap(),
            vec!["tlsAllowInvalidCertificates=true"]
        );
        // tls=false wins over a Required form.
        assert!(mongosh_tls_params("mongodb://h:1/?tls=false", &tls)
            .unwrap()
            .is_empty());
        // Preferred never switches a silent string on.
        assert!(
            mongosh_tls_params("mongodb://h:1/", &form(TlsMode::Preferred, true))
                .unwrap()
                .is_empty()
        );
        // Host/port path (no string): any enabled mode.
        assert_eq!(
            mongosh_tls_params("", &form(TlsMode::Preferred, true)).unwrap(),
            vec!["tls=true"]
        );
    }

    #[test]
    fn mongosh_params_add_the_ca_unless_the_string_names_one() {
        let mut tls = form(TlsMode::Required, true);
        tls.ca_cert = Some(CA.into());
        let params = mongosh_tls_params("mongodb://h:1/?tls=true", &tls).unwrap();
        assert_eq!(params.len(), 1);
        assert!(params[0].starts_with("tlsCAFile="), "{params:?}");
        assert!(
            mongosh_tls_params("mongodb://h:1/?tls=true&tlsCAFile=%2Fx.pem", &tls)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn conn_string_gets_a_well_formed_query() {
        let tls = form(TlsMode::Required, true);
        assert_eq!(
            conn_string_with_tls("mongodb://h:27017".into(), &tls).unwrap(),
            "mongodb://h:27017/?tls=true"
        );
        assert_eq!(
            conn_string_with_tls("mongodb://h:27017/db?replicaSet=rs".into(), &tls).unwrap(),
            "mongodb://h:27017/db?replicaSet=rs&tls=true"
        );
        assert_eq!(
            conn_string_with_tls("mongodb://h/db?tls=true".into(), &tls).unwrap(),
            "mongodb://h/db?tls=true"
        );
    }
}
