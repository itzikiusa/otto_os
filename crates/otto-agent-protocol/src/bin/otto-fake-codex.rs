//! Test double: a tiny `codex app-server` over stdio. The program lives in `src/fakes/codex.rs`, so an
//! in-process test (`scripted`, feature `test-util`) runs the same code.

#[path = "../fake_io.rs"]
mod fake_io;
#[path = "../fake_script.rs"]
mod fake_script;
#[path = "../fakes/codex.rs"]
mod program;

fn main() {
    program::main()
}
