//! Keeping deep `.await` chains inside a 2 MiB worker stack in debug builds.

use std::future::Future;
use std::pin::Pin;

/// A boxed, sendable future.
pub type Boxed<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// `make()`'s future, built in this short-lived frame and boxed.
///
/// An unoptimized `async fn`'s poll frame keeps a slot for every future it
/// builds, and nested polls keep all those frames live at once, so a deep
/// chain (workflow run → node → step runner → runtime turn → spawn) grows the
/// stack by the sum of its futures. `Box::pin(f()).await` still builds `f`'s
/// future in the caller's frame; `boxed(|| f()).await` builds it here, so
/// the caller's frame and future hold one pointer.
pub fn boxed<'a, F, Fut>(make: F) -> Boxed<'a, Fut::Output>
where
    F: FnOnce() -> Fut,
    Fut: Future + Send + 'a,
{
    Box::pin(make())
}
