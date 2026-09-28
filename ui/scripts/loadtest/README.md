# Parallel-load test (CPU and RAM vs parallel agents)

`loadtest.mjs` measures what Otto costs the machine as the number of busy
agents grows: per-process CPU % and RSS (ottod, ClickHouse, agent processes,
the webview's renderer/GPU/browser processes), page metrics (JS heap, DOM,
layouts/s, style/script time, long tasks, frame jank) and terminal/events
WebSocket bytes and API requests per second, for several UI states (tiled
agents, a focused agent, Home, Git, Database).

It is the round-3 driver (perf review r3-11) turned into a repo script.

## Isolation: it never touches your running Otto

- It starts its **own** `ottod` with a temp data dir, a temp `HOME` (so the
  daemon never reads your real `~/.claude` / `~/.codex`) and its own port
  (`OTTO_E2E_PORT`, default 7831). It refuses port 7700, and refuses any port
  where something already answers.
- `claude` on the daemon's `PATH` is `fake-claude.mjs`: an agent emulator
  that streams ANSI output (2–20 KB/s plus occasional 200 KB bursts), appends
  real-shaped transcript JSONL under the temp `HOME` and posts hooks to the
  test daemon only. No model, no network. `codex`/`agy`/… are inert stubs.
- It drives the daemon's embedded UI in headless Chromium (CDP metrics).
- On exit — normal, error, safety abort or Ctrl-C — it kills only the
  processes it started (by pid, by its temp root, by the emulator path) and
  removes the temp root. `cleanup.sh` does the same for a run that was killed
  hard.
- Safety aborts: load average above `--max-load` (12), available memory below
  `--min-free-gb` (2), swap growth above `--max-swap-growth-mb` (500), or the
  `--deadline` (seconds).

## Running it

Run one load test at a time, in the foreground, when the machine is otherwise
quiet (it is a measurement). From `ui/`:

```sh
# The daemon must embed the UI (or pass --ui-url):
(cd ui && npm run build) && cargo build --release -p ottod --features embed-ui
# (defaults to target/release/ottod, else the installed app's ottod —
#  isolated either way; override with OTTO_E2E_BIN)

node scripts/loadtest/loadtest.mjs --mode scale --steps 1,3 --hold 180
node scripts/loadtest/loadtest.mjs --mode scale --steps 5,10 --hold 180
node scripts/loadtest/loadtest.mjs --mode leak --n 5 --leak-min 15

node scripts/loadtest/analyze.mjs "$TMPDIR/otto-loadtest/<run>"
bash scripts/loadtest/cleanup.sh   # only if a run was killed with -9
```

Runs land in `$OTTO_LOADTEST_RUNS` (default `$TMPDIR/otto-loadtest/<mode>-<ts>`,
or `--out <dir>`): `driver.log`, `samples.jsonl` (one sample per
`--sample` seconds), `ottod.log`, screenshots per UI state, `page-errors.json`.

| Mode | What it does |
|---|---|
| `scale` | baseline with no agents, then for each N in `--steps`: N emulated agents, `--hold` seconds per UI state, then close all and watch recovery. |
| `leak` | `--n` agents for `--leak-min` minutes, cycling UI states every `--state-s` seconds, sampling heap/RSS growth. |
| `churn` | `--n` agents, then a pop-out window and view switches: what extra windows and remounts cost. |
| `suspend` | manual vs delegated sessions killed and reopened from the UI, then `--watch-s` seconds of idle-suspend observation. |

`analyze.mjs` prints one row per (phase, N, UI state): mean CPU % / last RSS
per process group, JS heap, DOM size, layouts/s, style/script/task ms per
second, jank frames/s, terminal and events WebSocket KB/s, API requests/s and
thread counts, then the top API endpoints per phase (polling storms).

Useful flags: `--quiet-agents true` (emulators print nothing: isolates the
daemon/UI floor), `--keep-root true` (keep the temp root for inspection),
`--stacks false` (skip the 5 s `sample` stack captures of the test's own
processes), `--ui-url <url>`.
