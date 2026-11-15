//! Detached `runtime-holder` process infrastructure so structured runtime
//! sessions (`Claude`, `Codex`, `ACP`, `Pi`) survive `ottod` restarts cleanly.

pub mod client;
pub mod paths;
mod pipes;
pub mod protocol;
pub mod server;

pub use client::{
    adopt_socket, best_effort_shutdown_socket, spawn_held, AdoptedHolder, HeldChild, HolderClient,
    HolderEvent,
};
pub use paths::{list_holder_sockets, socket_path_for, HolderLauncher};
pub use protocol::{
    checkpoint_acked_seq, DaemonFrame, HolderCheckpoint, HolderChildCommand, HolderExitStatus,
    HolderFrame, HolderReady, HolderSignal, HolderSpawnSpec, HolderTurnSnapshot, LineRing,
    LineStream, SequencedLine, DEFAULT_EXIT_LINGER, ERROR_PREFIX, HOLDER_ENV, MAX_FRAME_BYTES,
    MAX_RING_BYTES, PROTO_VERSION, READY_PREFIX,
};
pub use server::{run_from_args_and_stdin, run_holder_cli};
