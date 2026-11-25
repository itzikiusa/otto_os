---
description: Use when the user explicitly asks you to debug with Otto ("use Otto to debug …", "debug this test in Otto") — drive Otto's IDE debugger through the otto_ui_ide_debug_* tools so the user watches every stop. Covers picking breakpoints, starting a test / main / run config (Go build tags, Testcontainers), firing the request at a local server, reading a Stop, explaining each stop and waiting, DB Watch, and cleanup. Do not use for ordinary debugging the user did not ask Otto for, for attaching to running processes, or for remote / production targets.
category: development
version: 1
---

# Debug with Otto

The user wants to watch the debugging in Otto's IDE and steer it: you set the
breakpoints, start the program, and stop at each one to explain what the values
show. They decide when to move on. Use this only when they asked for it in
words; the tools run code on their Mac outside your sandbox, so every start and
every evaluated expression asks them first.

## Workflow

1. **Get control.** If no `otto_ui_ide_debug_*` tools are listed, call
   `otto_ui_request_control` and tell the user to click Allow in Otto. The tools
   appear once they do.
2. **Read the suspected path first.** Pick 2–4 breakpoints at its decision
   points: where input is parsed, where a branch is chosen, just before the
   value you suspect is written or returned. A breakpoint on every line makes
   the user click through noise.
3. **Start.** Call `ide_debug_start` with:
   - `target`: `{kind: "test", file, name}` for a test (a testify suite method
     is `TestSuite/TestMethod`), `{kind: "main", file}` for a program, or
     `{kind: "config", name}` for a run configuration;
   - `build_tags` when the file has a `//go:build` line (`["component"]`);
   - `breakpoints`, and for a server `ready_pattern` (its "listening on" log
     text) or `ready_port`, so the call returns as soon as it is up;
   - `db_watch: true` when the bug is about data the code writes or reads.
   Tell the user in one line what will run; they see one confirm with the
   command line, folder and any `env` you pass.
4. **When it answers `state: running`.** A server is up: send the request with
   `ide_debug_request` (localhost only). A test still building (a cold Go build
   with Testcontainers can take a minute): call `ide_debug_control` with
   `action: "wait"`.
5. **At each stop, explain and wait.** Read the Stop (below), say what the
   values show in at most 3 sentences and what "proceed" will do next, then end
   your turn. The user may answer in the conversation or press Proceed in Otto.
6. **Follow their answer.**
   - "proceed": `ide_debug_control {action: "continue"}` to the next breakpoint
     you set, or `step_over` when you said you would go line by line.
   - A question about a value: `ide_debug_inspect` with `paths`
     (`["req.Header", "cfg.DB.Host"]`); it runs no code.
   - "step into" / "run to line 40": the matching `action`.
7. **Finish.** `ide_debug_control {action: "stop"}` (it removes your
   breakpoints), then give the cause you found and the fix you propose. Edit
   code only after the user agrees; to try the fix, call `ide_debug_start`
   again (a restart asks again).

## Reading a Stop

| Field | Use it for |
|---|---|
| `location`, `source` | Where it stopped; the marked line is about to run. |
| `locals`, `changed` | The first scope's values by name, and which changed since the last stop. Objects show as `Type{…}` with a `ref` you can pass to `ide_debug_inspect`. |
| `frames` | Who called this (top 10). Pass a frame's `id` as `frame_id` to inspect its values. |
| `breakpoints` | Each of yours with `verified` and the line it bound to. Move one that did not bind to a line with code. |
| `output_tail` | Program output since your last call: errors, logs, "listening on". |
| `db` | Rows added / removed / changed since the previous stop (with DB Watch on). |
| `user_moved` | The user stepped themselves: describe the new stop and ask before moving. |
| `request` | The response to your request, when it arrived after a stop. |

Values that look like secrets come back as `[redacted]`; the user sees the
real value in the IDE and can tell you if it matters.

## Output format at a stop

> Stopped at `handlers/orders.go:57` in `CreateOrder`: `req.Items` is empty
> although the request had two items, so the decoder dropped the body.
> Proceed continues to the write in `repo.Save` (line 88).

## Language notes

- **Go**: tagged tests need `build_tags`. Delve may move a breakpoint to the
  next line with code; check the bound `line`.
- **Python**: works without installing anything (built-in bridge).
- **Node / TypeScript**: Vitest breakpoints can land a line off without source
  maps; prefer the function's first statement.
- **Java**: Maven and Gradle tests by class + method name.

See `examples/session.md` for a whole session and a non-trigger, and
`references/tools.md` for each tool's confirm and timing.

## Care points

- `ide_debug_evaluate` can change the program's state (call functions, assign).
  Use `ide_debug_inspect` to read; when an expression has side effects, say so
  in your message before calling it, because the user approves each one.
- Debug local code only. Requests to other hosts and loader or toolchain
  variables in `env` (`PATH`, `GOFLAGS`, `NODE_OPTIONS`, …) are refused; secrets
  belong in the project's environment, not in `env`.
- Stop the session when you are done, so no server keeps running on the user's
  ports.
- If a call answers `cancelled_by_user`, the user declined: ask what they want
  instead of retrying.
