# term-e2e — terminal robustness harness

Protocol-level E2E for the `/ws/term/{id}` terminal pipeline. A headless-xterm
client that is a **faithful model of `ui/src/lib/components/Terminal.svelte`'s
lifecycle** (attach → one snapshot rebuild → live bytes; local-first resize with
ONE trailing-debounced PTY notification; no mid-stream rebuilds) is driven
against a real `ottod`, including **real `claude` and `codex` sessions** — the
sim/shell scenarios alone historically passed while real agents still broke.

```bash
cargo build -p ottod          # the harness spawns target/debug/ottod
cd tools/term-e2e && npm ci
node run.mjs                  # full matrix (spawns an ISOLATED throwaway daemon)
node run.mjs --fast           # skip the real-agent scenarios (no API spend)
node run.mjs sim-tui          # one scenario by name
OTTO_BASE=http://127.0.0.1:7700 OTTO_API_TOKEN=… node run.mjs   # against a live daemon
```

The isolated daemon uses a temp `OTTO_DATA_DIR`, port `7911` (`OTTO_E2E_PORT`),
`OTTO_SECRETS=file` (a debug build otherwise hangs on a Keychain prompt) and
`OTTO_SELF_IMPROVE=0`. It never touches real sessions or state. Real-agent
scenarios use the machine's logged-in `claude`/`codex` CLIs and spend a trivial
prompt each.

| Scenario | Asserts |
|---|---|
| `shell-history` | 250 numbered lines exactly once, live + after reconnect |
| `shell-resize-storm` | 5 resizes lose nothing, duplicate nothing; reconnect intact |
| `reflow-rejoin` | narrow→wide rejoins soft-wrapped lines (native xterm reflow) |
| `sim-tui` | SIGWINCH-repainting TUI (claude-style) survives drag resizes exactly-once, live + reconnect |
| `authority-stomp` | passive viewer can't stomp the typing owner's PTY grid — attach or close |
| `attach-latency` | attach+rebuild with a 4k-line history under `LATENCY_MS` (default 2500) |
| `real-claude` | real prompt → response; 4 resizes: growth ≤1, screen coherent, reconnect no amplification |
| `real-codex` | same; scrollback growth bounded by codex's own per-SIGWINCH re-emission (upstream parity), **no multiplication**, screen coherent |
