# otto-debug: tool facts

The source of truth is `docs/contracts/ui-commands.json` (block "IDE agent debugging"); the
user-facing guide is `docs/features/agent-ui-control.md` §5 "IDE debugging".

| Tool | Risk | Confirm |
|---|---|---|
| `ide_debug_start` | execute | every start and restart, never remembered |
| `ide_debug_control` | navigate | none (`stop` ends the run) |
| `ide_debug_breakpoints` | navigate | none |
| `ide_debug_inspect` | read | none — runs no code |
| `ide_debug_evaluate` | execute | every expression, shown verbatim |
| `ide_debug_request` | local_write | per host:port, may be allowed for the session; localhost only |
| `ide_debug_db` | read | production connections always ask |

Timing: one call waits at most about 100 s (the bridge's 120 s deadline, a little longer while the
user decides a confirm). A first cold Go build with Testcontainers measured ~50 s on a laptop, so a
`running` answer followed by `ide_debug_control {action: "wait"}` is normal.
