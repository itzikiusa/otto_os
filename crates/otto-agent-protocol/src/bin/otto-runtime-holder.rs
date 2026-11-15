//! Standalone `otto-runtime-holder` binary for structured-runtime holder
//! integration tests and direct launcher use.

use std::process::ExitCode;

fn main() -> ExitCode {
    ExitCode::from(otto_agent_protocol::holder::run_from_args_and_stdin().clamp(0, 255) as u8)
}
