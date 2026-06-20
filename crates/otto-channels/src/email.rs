//! Gmail App Password email sender (mobile plan Task 7.1).
//!
//! The foundation of the email-OTP share gate (Tasks 7.2–7.4): a user configures
//! their Gmail address + a 16-char **Gmail App Password** once, and Otto sends
//! one-time codes to share-link recipients over Gmail SMTP.
//!
//! Transport: `smtp.gmail.com:587` with **STARTTLS** + **SMTP AUTH** (the Gmail
//! address as username, the app password as password), via lettre's async
//! transport on the workspace's rustls/tokio stack. The app password is NEVER
//! stored here — it is handed to [`GmailSender`] by the route layer, which reads
//! it from the macOS Keychain (`otto-keychain`) on demand; the DB holds only an
//! opaque `secret_ref`.
//!
//! ## Validation (`verify`)
//! lettre can open + EHLO a connection (`test_connection`), but that does **not**
//! prove the credentials — Gmail accepts the connection regardless and only
//! rejects a bad app password at AUTH/`MAIL FROM` time. So [`GmailSender::verify`]
//! performs a real end-to-end check: it sends a tiny message **from the sender
//! to itself**, which forces the server through STARTTLS + AUTH and surfaces an
//! invalid app password as an error. (Gmail App Passwords have no cheap bare-AUTH
//! probe in lettre; the self-send is the cleanest reliable validation and lands
//! in the owner's own inbox.)

use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use otto_core::{Error, Result};

/// Gmail SMTP submission host. Port 587 + STARTTLS is selected by
/// [`AsyncSmtpTransport::starttls_relay`].
const GMAIL_SMTP_HOST: &str = "smtp.gmail.com";

/// A configured Gmail sender. Holds the from-address and the app password in
/// memory only for the lifetime of the call chain that built it; the password
/// is sourced from the Keychain by the caller and never persisted by this type.
pub struct GmailSender {
    /// The Gmail address mail is sent from (also the SMTP AUTH username).
    pub from_address: String,
    /// The 16-char Gmail App Password (SMTP AUTH password). Never logged.
    pub app_password: String,
}

impl GmailSender {
    /// Construct a sender from a Gmail address and its app password.
    pub fn new(from_address: impl Into<String>, app_password: impl Into<String>) -> Self {
        Self {
            from_address: from_address.into(),
            app_password: app_password.into(),
        }
    }

    /// Build the async STARTTLS transport to `smtp.gmail.com:587` authenticated
    /// with the configured address + app password.
    fn transport(&self) -> Result<AsyncSmtpTransport<Tokio1Executor>> {
        let creds = Credentials::new(self.from_address.clone(), self.app_password.clone());
        let mailer = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(GMAIL_SMTP_HOST)
            .map_err(|e| Error::Upstream(format!("gmail smtp transport: {e}")))?
            .credentials(creds)
            .build();
        Ok(mailer)
    }

    /// Build a plain-text [`Message`] from the sender to `to`.
    fn build(&self, to: &str, subject: &str, body: &str) -> Result<Message> {
        let from = self
            .from_address
            .parse()
            .map_err(|e| Error::Invalid(format!("invalid sender address '{}': {e}", self.from_address)))?;
        let to = to
            .parse()
            .map_err(|e| Error::Invalid(format!("invalid recipient address '{to}': {e}")))?;
        Message::builder()
            .from(from)
            .to(to)
            .subject(subject)
            .header(ContentType::TEXT_PLAIN)
            .body(body.to_string())
            .map_err(|e| Error::Invalid(format!("build email: {e}")))
    }

    /// Send a plain-text email to `to` over Gmail SMTP (STARTTLS + AUTH).
    pub async fn send(&self, to: &str, subject: &str, body: &str) -> Result<()> {
        let message = self.build(to, subject, body)?;
        let mailer = self.transport()?;
        mailer
            .send(message)
            .await
            .map_err(|e| Error::Upstream(format!("gmail send: {e}")))?;
        Ok(())
    }

    /// Validate the configured Gmail address + app password by sending a tiny
    /// message from the sender to **itself** — exercising STARTTLS + SMTP AUTH
    /// end-to-end. A wrong app password (or 2FA/app-password misconfig) surfaces
    /// here as an [`Error::Upstream`]. On success the sender is proven and the
    /// route may mark it verified.
    ///
    /// (lettre exposes `test_connection()`, but that only checks reachability —
    /// it does not authenticate — so it cannot validate the app password. The
    /// self-addressed probe send is the chosen, documented validation.)
    pub async fn verify(&self) -> Result<()> {
        self.send(
            &self.from_address,
            "Otto email sender verified",
            "This message confirms your Otto email sender (Gmail App Password) is configured correctly.",
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // NOTE: these tests deliberately do NOT touch real Gmail. They exercise the
    // pure, network-free paths (message + transport construction and input
    // validation). Live SMTP send/verify is covered manually / in the
    // consolidated E2E (it requires a real Gmail App Password + network).

    #[test]
    fn builds_a_plaintext_message() {
        let s = GmailSender::new("me@gmail.com", "app-password");
        let msg = s
            .build("dest@example.com", "Subj", "hello")
            .expect("message builds");
        let raw = String::from_utf8(msg.formatted()).unwrap();
        assert!(raw.contains("From: me@gmail.com"));
        assert!(raw.contains("To: dest@example.com"));
        assert!(raw.contains("Subject: Subj"));
        // The app password must never leak into the serialized message.
        assert!(!raw.contains("app-password"));
    }

    #[test]
    fn rejects_a_malformed_sender_address() {
        let s = GmailSender::new("not-an-email", "pw");
        let err = s.build("dest@example.com", "s", "b").unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "got {err:?}");
    }

    #[test]
    fn rejects_a_malformed_recipient_address() {
        let s = GmailSender::new("me@gmail.com", "pw");
        let err = s.build("not-an-email", "s", "b").unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "got {err:?}");
    }

    // Async so the pooled transport's `Drop` runs inside a tokio runtime
    // (lettre's `pool` feature requires one at drop time). Construction is still
    // network-free — no connection is opened until a send.
    #[tokio::test]
    async fn transport_builds_for_valid_host() {
        let s = GmailSender::new("me@gmail.com", "pw");
        assert!(s.transport().is_ok());
    }
}
