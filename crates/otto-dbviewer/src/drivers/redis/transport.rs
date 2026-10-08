//! A cached redis-rs connection whose plaintext stream validates RESP budgets.
//! An interrupted/failed request retires the shared connection; the next cache
//! acquisition reconnects. A command is never automatically replayed.
use super::framing::{Budget, Frames, Limits};
use crate::types::{self, ResolvedConfig};
use redis::{
    aio::{ConnectionLike, MultiplexedConnection},
    Cmd, Pipeline, RedisFuture, RedisResult, Value,
};
use std::{
    io,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    task::{Context, Poll},
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

struct Bounded<S> {
    inner: S,
    frames: Frames,
    failure: Arc<std::sync::Mutex<Option<String>>>,
    alive: Arc<AtomicBool>,
}
impl<S: AsyncRead + Unpin> AsyncRead for Bounded<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        output: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if output.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        let this = self.get_mut();
        let mut bytes = [0; 8192];
        let capacity = output.remaining().min(bytes.len());
        let mut read = ReadBuf::new(&mut bytes[..capacity]);
        match Pin::new(&mut this.inner).poll_read(cx, &mut read) {
            Poll::Ready(Ok(())) => {
                if read.filled().is_empty() {
                    this.alive.store(false, Ordering::Release);
                }
                if let Err(error) = this.frames.accept(read.filled()) {
                    *this.failure.lock().unwrap_or_else(|e| e.into_inner()) =
                        Some(error.to_string());
                    this.alive.store(false, Ordering::Release);
                    return Poll::Ready(Err(error));
                }
                output.put_slice(read.filled());
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Err(error)) => {
                this.alive.store(false, Ordering::Release);
                Poll::Ready(Err(error))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}
impl<S: AsyncWrite + Unpin> AsyncWrite for Bounded<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, bytes)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}
struct Owner {
    failure: Arc<std::sync::Mutex<Option<String>>>,
    gate: tokio::sync::Mutex<()>,
    budget: Arc<std::sync::Mutex<Budget>>,
    alive: Arc<AtomicBool>,
    driver: tokio::task::AbortHandle,
}
impl Owner {
    async fn enter(&self) -> RedisResult<tokio::sync::MutexGuard<'_, ()>> {
        tokio::time::timeout(Duration::from_secs(30), self.gate.lock())
            .await
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Redis request timed out waiting for the cached connection",
                )
                .into()
            })
    }
    fn retire(&self) {
        self.alive.store(false, Ordering::Release);
        self.driver.abort();
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        self.retire();
    }
}
struct RequestGuard {
    owner: Arc<Owner>,
    completed: bool,
}
impl RequestGuard {
    fn finish<T>(&mut self, result: &RedisResult<T>) {
        self.completed = true;
        if result
            .as_ref()
            .is_err_and(|e| e.is_unrecoverable_error() || e.is_timeout())
        {
            self.owner.retire();
        }
    }
}
impl Drop for RequestGuard {
    fn drop(&mut self) {
        if !self.completed {
            self.owner.retire();
        }
    }
}

#[derive(Clone)]
pub(super) struct Connection {
    conn: MultiplexedConnection,
    owner: Arc<Owner>,
}
impl Connection {
    pub fn is_alive(&self) -> bool {
        self.owner.alive.load(Ordering::Acquire) && !self.owner.driver.is_finished()
    }
    pub fn ensure_alive(&self) -> otto_core::Result<()> {
        if self.is_alive() {
            return Ok(());
        }
        let reason = self
            .owner
            .failure
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .unwrap_or_else(|| "Redis connection interrupted; retry to reconnect".into());
        Err(types::upstream(reason))
    }
    pub async fn connect(
        cfg: &ResolvedConfig,
        info: &redis::RedisConnectionInfo,
    ) -> otto_core::Result<Self> {
        tokio::time::timeout(Duration::from_secs(10), async {
            let stream = tokio::net::TcpStream::connect((cfg.host.as_str(), cfg.port))
                .await
                .map_err(types::upstream)?;
            stream.set_nodelay(true).map_err(types::upstream)?;
            if cfg.tls.enabled() {
                let connector = super::tls::connector(&cfg.tls)?;
                let hostname = cfg
                    .tls
                    .server_name
                    .clone()
                    .filter(|s| !s.is_empty())
                    .or_else(|| cfg.param_str("__tunnel_host"))
                    .unwrap_or_else(|| cfg.host.clone());
                let name = rustls::pki_types::ServerName::try_from(hostname)
                    .map_err(|e| types::invalid(e.to_string()))?;
                let stream = connector
                    .connect(name, stream)
                    .await
                    .map_err(types::upstream)?;
                Self::from_stream(stream, info, Limits::default())
                    .await
                    .map_err(types::upstream)
            } else {
                Self::from_stream(stream, info, Limits::default())
                    .await
                    .map_err(types::upstream)
            }
        })
        .await
        .map_err(|_| types::upstream("redis: connection/handshake timed out after 10s"))?
    }
    async fn from_stream<S: AsyncRead + AsyncWrite + Unpin + Send + 'static>(
        stream: S,
        info: &redis::RedisConnectionInfo,
        limits: Limits,
    ) -> RedisResult<Self> {
        let alive = Arc::new(AtomicBool::new(true));
        let failure = Arc::new(std::sync::Mutex::new(None));
        let frames = Frames::new(limits);
        let budget = frames.budget.clone();
        let bounded = Bounded {
            inner: stream,
            frames,
            failure: failure.clone(),
            alive: alive.clone(),
        };
        let config =
            redis::AsyncConnectionConfig::new().set_response_timeout(Some(Duration::from_secs(30)));
        let (conn, driver) = MultiplexedConnection::new_with_config(info, bounded, config).await?;
        // Driver owns only `alive`, never Owner: the last cached/in-flight clone
        // must abort its task instead of leaving a reference cycle behind.
        let driver_alive = alive.clone();
        let task = tokio::spawn(async move {
            driver.await;
            driver_alive.store(false, Ordering::Release);
        });
        Ok(Self {
            conn,
            owner: Arc::new(Owner {
                failure,
                gate: tokio::sync::Mutex::new(()),
                budget,
                alive,
                driver: task.abort_handle(),
            }),
        })
    }
    fn guard(&self) -> RedisResult<RequestGuard> {
        if !self.is_alive() {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "Redis connection retired; retry to reconnect",
            )
            .into());
        }
        Ok(RequestGuard {
            owner: self.owner.clone(),
            completed: false,
        })
    }
}
impl ConnectionLike for Connection {
    fn req_packed_command<'a>(&'a mut self, cmd: &'a Cmd) -> RedisFuture<'a, Value> {
        Box::pin(async move {
            let owner = self.owner.clone();
            let _operation = owner.enter().await?;
            let mut guard = self.guard()?;
            *owner.budget.lock().unwrap_or_else(|e| e.into_inner()) = Budget::default();
            let result = self.conn.req_packed_command(cmd).await;
            guard.finish(&result);
            result
        })
    }
    fn req_packed_commands<'a>(
        &'a mut self,
        pipe: &'a Pipeline,
        offset: usize,
        count: usize,
    ) -> RedisFuture<'a, Vec<Value>> {
        Box::pin(async move {
            let owner = self.owner.clone();
            let _operation = owner.enter().await?;
            let mut guard = self.guard()?;
            *owner.budget.lock().unwrap_or_else(|e| e.into_inner()) = Budget::default();
            let result = self.conn.req_packed_commands(pipe, offset, count).await;
            guard.finish(&result);
            result
        })
    }
    fn get_db(&self) -> i64 {
        self.conn.get_db()
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/redis_transport.rs"]
mod tests;
