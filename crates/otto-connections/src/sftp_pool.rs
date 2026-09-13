//! Bounded transport reuse. Authorization remains the caller's responsibility
//! on every request; the cache key also isolates users and profile revisions.
use otto_core::{Error, Result};
use otto_ssh::{SftpParams, SftpSession};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

const CAPACITY: usize = 32;
const IDLE: Duration = Duration::from_secs(60);
#[derive(Hash, Eq, PartialEq)]
struct Key {
    user: String,
    connection: String,
    params: SftpParams,
}
struct Entry {
    session: Arc<SftpSession>,
    touched: Instant,
}
#[derive(Default)]
pub(crate) struct SftpPool {
    entries: Mutex<HashMap<Key, Entry>>,
    started: AtomicBool,
    program: Option<std::path::PathBuf>,
}
impl SftpPool {
    #[cfg(test)]
    pub fn with_program(program: std::path::PathBuf) -> Self {
        Self {
            program: Some(program),
            ..Self::default()
        }
    }
    pub async fn acquire(
        self: &Arc<Self>,
        user: &str,
        connection: &str,
        params: SftpParams,
    ) -> Result<Arc<SftpSession>> {
        if !self.started.swap(true, Ordering::AcqRel) {
            let weak = Arc::downgrade(self);
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    let Some(pool) = weak.upgrade() else { break };
                    pool.prune().await;
                }
            });
        }
        let key = Key {
            user: user.into(),
            connection: connection.into(),
            params,
        };
        let mut entries = self.entries.lock().await;
        if let Some(entry) = entries.get_mut(&key) {
            entry.touched = Instant::now();
            return Ok(entry.session.clone());
        }
        entries.retain(|_, entry| {
            entry.touched.elapsed() < IDLE || Arc::strong_count(&entry.session) > 1
        });
        if entries.len() >= CAPACITY {
            let oldest = entries
                .iter()
                .filter(|(_, e)| Arc::strong_count(&e.session) == 1)
                .min_by_key(|(_, e)| e.touched)
                .map(|(k, _)| Key {
                    user: k.user.clone(),
                    connection: k.connection.clone(),
                    params: k.params.clone(),
                });
            if let Some(key) = oldest {
                entries.remove(&key);
            } else {
                return Err(Error::Conflict(
                    "all SFTP sessions are busy; retry after a transfer finishes".into(),
                ));
            }
        }
        let session = Arc::new(match &self.program {
            Some(program) => SftpSession::with_program(key.params.clone(), program.clone())?,
            None => SftpSession::new(key.params.clone())?,
        });
        entries.insert(
            key,
            Entry {
                session: session.clone(),
                touched: Instant::now(),
            },
        );
        Ok(session)
    }
    #[cfg(test)]
    pub(crate) async fn clear_for_benchmark(&self) {
        self.entries.lock().await.clear();
    }

    async fn prune(&self) {
        self.entries
            .lock()
            .await
            .retain(|_, e| e.touched.elapsed() < IDLE || Arc::strong_count(&e.session) > 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn params() -> SftpParams {
        SftpParams {
            host: "fixture".into(),
            port: None,
            user: None,
            identity_file: None,
            jump: None,
        }
    }
    #[tokio::test]
    async fn reuse_is_scoped_to_actor_and_profile_and_idle_entries_expire() {
        let pool = Arc::new(SftpPool::default());
        let a = pool.acquire("alice", "conn", params()).await.unwrap();
        assert!(Arc::ptr_eq(
            &a,
            &pool.acquire("alice", "conn", params()).await.unwrap()
        ));
        let b = pool.acquire("bob", "conn", params()).await.unwrap();
        assert!(!Arc::ptr_eq(&a, &b));
        let mut changed = params();
        changed.port = Some(2222);
        let c = pool.acquire("alice", "conn", changed).await.unwrap();
        assert!(!Arc::ptr_eq(&a, &c));
        for entry in pool.entries.lock().await.values_mut() {
            entry.touched = Instant::now() - IDLE - Duration::from_secs(1);
        }
        pool.prune().await;
        assert_eq!(
            pool.entries.lock().await.len(),
            3,
            "active leases survive cleanup"
        );
        drop((a, b, c));
        pool.prune().await;
        assert!(pool.entries.lock().await.is_empty());
    }
}
