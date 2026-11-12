//! Redis transport trust configuration. System roots remain the default;
//! explicit CA bundles and client credentials retain redis-rs semantics.
use crate::types::{self, TlsConfig};
use rustls::{
    pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer},
    ClientConfig, RootCertStore,
};
use std::sync::Arc;

pub(super) fn connector(tls: &TlsConfig) -> otto_core::Result<tokio_rustls::TlsConnector> {
    let mut roots = RootCertStore::empty();
    if let Some(ca) = tls.ca_cert.as_deref().filter(|s| !s.trim().is_empty()) {
        // Inline PEM or a path to a PEM file (crate::tls::resolve_pem).
        let ca = crate::tls::pem_text("ca_cert", ca)?;
        for cert in CertificateDer::pem_slice_iter(ca.as_bytes()) {
            roots
                .add(cert.map_err(|e| types::invalid(e.to_string()))?)
                .map_err(|e| types::invalid(e.to_string()))?;
        }
        if roots.is_empty() {
            return Err(types::invalid("redis: CA bundle has no certificates"));
        }
    } else {
        let loaded = rustls_native_certs::load_native_certs();
        for cert in loaded.certs {
            roots.add(cert).map_err(|e| types::invalid(e.to_string()))?;
        }
        if roots.is_empty() && tls.verify {
            return Err(types::upstream("redis: no system TLS roots available"));
        }
    }
    let builder =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(|e| types::invalid(e.to_string()))?;
    let builder = if tls.verify {
        builder.with_root_certificates(roots)
    } else {
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(Unverified(
                rustls::crypto::ring::default_provider(),
            )))
    };
    let cert = tls.client_cert.as_deref().filter(|s| !s.trim().is_empty());
    let key = tls.client_key.as_deref().filter(|s| !s.trim().is_empty());
    let config = match (cert, key) {
        (Some(cert), Some(key)) => {
            let cert_pem = crate::tls::pem_text("client_cert", cert)?;
            let key_pem = crate::tls::pem_text("client_key", key)?;
            let certs = CertificateDer::pem_slice_iter(cert_pem.as_bytes())
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|e| types::invalid(e.to_string()))?;
            let key = PrivateKeyDer::from_pem_slice(key_pem.as_bytes())
                .map_err(|e| types::invalid(e.to_string()))?;
            builder
                .with_client_auth_cert(certs, key)
                .map_err(|e| types::invalid(e.to_string()))?
        }
        (None, None) => builder.with_no_client_auth(),
        _ => {
            return Err(types::invalid(
                "redis: client TLS certificate and key must both be provided",
            ))
        }
    };
    Ok(tokio_rustls::TlsConnector::from(Arc::new(config)))
}
#[derive(Debug)]
struct Unverified(rustls::crypto::CryptoProvider);
impl rustls::client::danger::ServerCertVerifier for Unverified {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &rustls::pki_types::ServerName<'_>,
        _: &[u8],
        _: rustls::pki_types::UnixTime,
    ) -> std::result::Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            signature,
            &self.0.signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            signature,
            &self.0.signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tls_with_ca(ca: String) -> TlsConfig {
        TlsConfig {
            mode: types::TlsMode::Required,
            ca_cert: Some(ca),
            ..TlsConfig::default()
        }
    }

    #[test]
    fn ca_given_as_a_file_path_is_loaded() {
        let ca = rcgen::generate_simple_self_signed(vec!["redis.fixture".into()]).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("ca.pem");
        std::fs::write(&file, ca.cert.pem()).unwrap();
        assert!(connector(&tls_with_ca(file.to_string_lossy().into_owned())).is_ok());
        // Inline PEM keeps working exactly as before.
        assert!(connector(&tls_with_ca(ca.cert.pem())).is_ok());
    }

    #[test]
    fn missing_ca_path_fails_naming_the_file() {
        let err = connector(&tls_with_ca("/nonexistent/otto/redis-ca.pem".into()))
            .err()
            .expect("a missing CA file is an error");
        let msg = err.to_string();
        assert!(msg.contains("/nonexistent/otto/redis-ca.pem"), "{msg}");
    }
}
