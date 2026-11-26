//! Test double: a tiny ACP agent over stdio. The program lives in `src/fakes/agent.rs`, so an
//! in-process test (`scripted`, feature `test-util`) runs the same code.

#[path = "../fake_io.rs"]
mod fake_io;
#[path = "../fake_script.rs"]
mod fake_script;
#[path = "../fakes/agent.rs"]
mod program;

fn main() {
    program::main()
}
