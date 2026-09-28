//! Revocable authorization carried to the actual PTY writer, not just enqueue.
use std::io::{self, Write};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

#[derive(Clone)]
pub struct InputAuthorization {
    pub epoch: Arc<AtomicU64>,
    pub expected: u64,
}

pub(crate) fn write_authorized(
    writer: &mut dyn Write,
    mut data: &[u8],
    authorization: Option<&InputAuthorization>,
) -> io::Result<()> {
    while !data.is_empty() {
        if authorization.is_some_and(|a| a.epoch.load(Ordering::Acquire) != a.expected) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "terminal control changed",
            ));
        }
        // A write attempt already in flight cannot be recalled, even when the
        // OS blocks it. Bound that attempt; revalidate every subsequent chunk
        // so a queued large paste is not a durable grant after revocation.
        let chunk = if authorization.is_some() {
            &data[..data.len().min(256)]
        } else {
            data
        };
        match writer.write(chunk) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => data = &data[n..],
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queued_input_with_revoked_epoch_never_reaches_writer() {
        let auth = InputAuthorization {
            epoch: Arc::new(AtomicU64::new(2)),
            expected: 1,
        };
        let mut bytes = Vec::new();
        assert!(write_authorized(&mut bytes, b"old command", Some(&auth)).is_err());
        assert!(bytes.is_empty());
        let fresh = InputAuthorization {
            expected: 2,
            ..auth
        };
        write_authorized(&mut bytes, b"new", Some(&fresh)).unwrap();
        assert_eq!(bytes, b"new");
    }
    #[test]
    fn revocation_discards_unwritten_tail_after_partial_write() {
        struct Partial {
            bytes: Vec<u8>,
            epoch: Arc<AtomicU64>,
        }
        impl Write for Partial {
            fn write(&mut self, data: &[u8]) -> io::Result<usize> {
                self.bytes.push(data[0]);
                self.epoch.store(2, Ordering::Release);
                Ok(1)
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let auth = InputAuthorization {
            epoch: Arc::new(AtomicU64::new(1)),
            expected: 1,
        };
        let mut writer = Partial {
            bytes: Vec::new(),
            epoch: auth.epoch.clone(),
        };
        assert!(write_authorized(&mut writer, b"abc", Some(&auth)).is_err());
        assert_eq!(
            writer.bytes, b"a",
            "already accepted byte remains, queued tail must not"
        );
    }
    #[test]
    fn a_revoked_large_paste_has_at_most_one_small_inflight_chunk() {
        struct RevokeOnWrite {
            bytes: Vec<u8>,
            epoch: Arc<AtomicU64>,
        }
        impl Write for RevokeOnWrite {
            fn write(&mut self, data: &[u8]) -> io::Result<usize> {
                self.bytes.extend_from_slice(data);
                self.epoch.store(2, Ordering::Release);
                Ok(data.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let auth = InputAuthorization {
            epoch: Arc::new(AtomicU64::new(1)),
            expected: 1,
        };
        let mut writer = RevokeOnWrite {
            bytes: vec![],
            epoch: auth.epoch.clone(),
        };
        assert!(write_authorized(&mut writer, &vec![b'x'; 65536], Some(&auth)).is_err());
        assert_eq!(writer.bytes.len(), 256);
    }

    #[test]
    fn ordinary_write_is_unchanged() {
        let mut bytes = Vec::new();
        write_authorized(&mut bytes, b"abc", None).unwrap();
        assert_eq!(bytes, b"abc");
    }
}
