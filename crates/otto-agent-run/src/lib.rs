//! otto-agent-run — the primitives every engine uses to drive a CLI agent as a
//! PTY session and wait for its result: the result-file watcher with stuck /
//! deadline / recovery handling ([`agent_run`]), the transcript-based turn
//! oracle ([`turn_oracle`]), off-runtime blocking IO ([`offload`]) and screen
//! text helpers ([`screen`]), plus the pieces every assist engine shares: the
//! fenced-block fallback ([`fence`]) and the stat-gated live-preview poll
//! ([`live_file`]), and the resumable managed-session turn runner
//! ([`session_turn`]) with the fan-out + summarizer shape multi-agent
//! engines share ([`fanout`]), and the one assist turn engine Canvas, Design
//! Hall and mockups adapt ([`assist`]). Extracted from otto-server so leaf engine crates
//! (e.g. `otto-review`) can use them without depending on the server.

pub mod agent_run;
pub mod assist;
pub mod fanout;
pub mod fence;
pub mod live_file;
pub mod offload;
pub mod screen;
pub mod session_turn;
pub mod turn_oracle;
