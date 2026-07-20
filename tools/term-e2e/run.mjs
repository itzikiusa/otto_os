// Terminal-robustness E2E matrix. Usage:
//   node run.mjs                  # every scenario, incl. REAL claude/codex
//   node run.mjs shell-history …  # named scenarios only
//   node run.mjs --fast           # skip the real-agent scenarios (CI-friendly)
//
// Against a live daemon: OTTO_BASE=http://127.0.0.1:7700 OTTO_API_TOKEN=…
// Otherwise an ISOLATED throwaway ottod is spawned (temp data dir, port 7911);
// build it first: cargo build -p ottod
import {
  bootstrap, makeApi, ensureWorkspace, TermClient,
  sleep, count, waitFor, waitBufferStable, RESIZE_SETTLE_MS,
} from './client.mjs';

const argv = process.argv.slice(2);
const FAST = argv.includes('--fast');
const only = argv.filter((a) => !a.startsWith('--'));

const env = await bootstrap();
const api = makeApi(env.base, env.token);
const wsId = await ensureWorkspace(api);
console.log(`daemon: ${env.base} (${env.isolated ? 'isolated throwaway' : 'existing'}), workspace ${wsId}`);

const failures = [];
let currentScenario = '';
function check(cond, msg) {
  const line = `${cond ? 'ok ' : 'FAIL'}  [${currentScenario}] ${msg}`;
  console.log(line);
  if (!cond) failures.push(`[${currentScenario}] ${msg}`);
}

async function mkSession(provider, title, extra = {}) {
  return api('POST', `/api/v1/workspaces/${wsId}/sessions`, {
    kind: 'agent', provider, title, cwd: process.env.HOME, ...extra,
  });
}
async function rmSession(id) {
  await api('DELETE', `/api/v1/sessions/${id}`).catch(() => {});
}

// ── scenarios ────────────────────────────────────────────────────────────────

const scenarios = {
  /** 250 numbered shell lines survive attach + reconnect exactly once each. */
  async 'shell-history'() {
    const s = await mkSession('shell', 'e2e-shell-history');
    const c = new TermClient(env.base, env.token, s.id, { cols: 120, rows: 30 });
    try {
      await c.connect();
      c.type('for i in $(seq 1 250); do printf "LN_%04d marker line\\n" $i; done\n');
      await waitFor(() => count(c.bufferText(), 'LN_0250') >= 1, 20_000);
      await sleep(500);
      const text = c.bufferText();
      let dup = 0, missing = 0;
      for (let i = 1; i <= 250; i++) {
        const n = count(text, `LN_${String(i).padStart(4, '0')} `);
        if (n === 0) missing++;
        if (n > 1) dup++;
      }
      check(missing === 0, `live: all 250 lines present (missing=${missing})`);
      check(dup === 0, `live: no duplicated lines (dup=${dup})`);

      // Fresh reconnect: server rebuild must match.
      c.close();
      const c2 = new TermClient(env.base, env.token, s.id, { cols: 120, rows: 30 });
      await c2.connect();
      await sleep(500);
      const text2 = c2.bufferText();
      let dup2 = 0, missing2 = 0;
      for (let i = 1; i <= 250; i++) {
        const n = count(text2, `LN_${String(i).padStart(4, '0')} `);
        if (n === 0) missing2++;
        if (n > 1) dup2++;
      }
      check(missing2 === 0, `reconnect: all 250 lines present (missing=${missing2})`);
      check(dup2 === 0, `reconnect: no duplicated lines (dup=${dup2})`);
      c2.close();
    } finally {
      c.close();
      await rmSession(s.id);
    }
  },

  /** A resize storm neither loses nor duplicates shell content. */
  async 'shell-resize-storm'() {
    const s = await mkSession('shell', 'e2e-shell-storm');
    const c = new TermClient(env.base, env.token, s.id, { cols: 140, rows: 35 });
    try {
      await c.connect();
      c.type('for i in $(seq 1 120); do printf "SW_%04d storm marker with a reasonably long tail for wrapping\\n" $i; done\n');
      await waitFor(() => count(c.bufferText(), 'SW_0120') >= 1, 20_000);
      for (const [cols, rows] of [[100, 30], [80, 24], [120, 35], [60, 20], [150, 40]]) {
        await c.resizeSettled(cols, rows, 300);
      }
      const text = c.bufferText();
      let dup = 0, missing = 0;
      for (let i = 1; i <= 120; i++) {
        const n = count(text, `SW_${String(i).padStart(4, '0')} `);
        if (n === 0) missing++;
        if (n > 1) dup++;
      }
      check(missing === 0, `after 5 resizes: nothing lost (missing=${missing})`);
      check(dup === 0, `after 5 resizes: nothing duplicated (dup=${dup})`);

      c.close();
      const c2 = new TermClient(env.base, env.token, s.id, { cols: 150, rows: 40 });
      await c2.connect();
      await sleep(500);
      const t2 = c2.bufferText();
      let missing2 = 0, dup2 = 0;
      for (let i = 1; i <= 120; i++) {
        const n = count(t2, `SW_${String(i).padStart(4, '0')} `);
        if (n === 0) missing2++;
        if (n > 1) dup2++;
      }
      check(missing2 === 0, `reconnect after storm: nothing lost (missing=${missing2})`);
      check(dup2 === 0, `reconnect after storm: nothing duplicated (dup=${dup2})`);
      c2.close();
    } finally {
      c.close();
      await rmSession(s.id);
    }
  },

  /** narrow→wide rejoins soft-wrapped lines (native xterm reflow — the
   *  "widening doesn't redraw" case). */
  async 'reflow-rejoin'() {
    const s = await mkSession('shell', 'e2e-reflow');
    const c = new TermClient(env.base, env.token, s.id, { cols: 150, rows: 35 });
    try {
      await c.connect();
      const long = 'REJOIN_' + 'abcdefghij'.repeat(19) + '_END'; // 201 chars
      c.type(`printf '%s\\n' '${long}'\n`);
      await waitFor(() => c.bufferText().includes('_END'), 10_000);
      await c.resizeSettled(60, 24, 300);
      const narrowRows = c.bufferLines().filter((l) => l.includes('abcdefghij')).length;
      check(narrowRows >= 3, `at 60 cols the long line wraps (${narrowRows} rows)`);
      await c.resizeSettled(150, 35, 300);
      const prefix = long.slice(0, 150);
      const rejoined = c.bufferLines().some((l) => l.includes(prefix));
      check(rejoined, 'at 150 cols the line is contiguous again (150-char prefix on one row)');
    } finally {
      c.close();
      await rmSession(s.id);
    }
  },

  /** claude-like SIGWINCH-repainting TUI: drag-resizes leave every message
   *  exactly once, live and after reconnect. */
  async 'sim-tui'() {
    const s = await mkSession('shell', 'e2e-sim-tui');
    const c = new TermClient(env.base, env.token, s.id, { cols: 120, rows: 30 });
    const simPath = new URL('./tui_sim.py', import.meta.url).pathname;
    try {
      await c.connect();
      c.type(`SIM_MSGS=40 python3 ${simPath}\n`);
      await waitFor(() => count(c.bufferText(), 'MSG_039') >= 1, 15_000);
      await sleep(800);

      const analyze = (label, text) => {
        let dup = 0, missing = 0;
        for (let i = 0; i < 40; i++) {
          const n = count(text, `MSG_${String(i).padStart(3, '0')} `);
          if (n === 0) missing++;
          if (n > 1) dup++;
        }
        check(missing === 0, `${label}: all 40 msgs present (missing=${missing})`);
        check(dup === 0, `${label}: no duplicates (dup=${dup})`);
      };

      // Drag narrow (burst → ONE SIGWINCH at 80), then drag wide (ONE at 140).
      c.resize(112, 30);
      await sleep(60);
      await c.resizeSettled(80, 30, 800);
      analyze('after narrow drag 120→80', c.bufferText());
      c.resize(118, 30);
      await sleep(60);
      await c.resizeSettled(140, 30, 800);
      analyze('after widen drag 80→140', c.bufferText());

      c.close();
      const c2 = new TermClient(env.base, env.token, s.id, { cols: 140, rows: 30 });
      await c2.connect();
      await sleep(500);
      analyze('fresh reconnect @140', c2.bufferText());
      c2.close();
    } finally {
      c.close();
      await rmSession(s.id);
    }
  },

  /** Size authority: a passive viewer can't stomp the typing owner's grid —
   *  including when the passive viewer disconnects (pane-close case). */
  async 'authority-stomp'() {
    const s = await mkSession('shell', 'e2e-authority');
    const a = new TermClient(env.base, env.token, s.id, { cols: 150, rows: 40 });
    const b = new TermClient(env.base, env.token, s.id, { cols: 80, rows: 24 });
    try {
      await a.connect();
      a.type('echo authority-claimed\n'); // typing claims size authority
      await sleep(300);
      a.resize(150, 40);
      await sleep(RESIZE_SETTLE_MS + 200);
      await b.connect(); // passive viewer attaches at 80×24 (sends its resize on open)
      await sleep(400);
      let meta = (await api('GET', `/api/v1/sessions/${s.id}`)).meta ?? {};
      check(meta.pty_cols === 150 && meta.pty_rows === 40,
        `owner grid holds vs passive attach (pty=${meta.pty_cols}x${meta.pty_rows}, want 150x40)`);
      b.close(); // pane-close: detach must not disturb the survivor
      await sleep(400);
      meta = (await api('GET', `/api/v1/sessions/${s.id}`)).meta ?? {};
      check(meta.pty_cols === 150 && meta.pty_rows === 40,
        `owner grid holds after passive viewer closes (pty=${meta.pty_cols}x${meta.pty_rows})`);
    } finally {
      a.close();
      b.close();
      await rmSession(s.id);
    }
  },

  /** A PRIMARY pane (claims on attach) must not be shrunk by a passive viewer
   *  that attaches LATER, even when nobody has typed yet — the "pane renders
   *  at half size until I click into it" bug. */
  async 'authority-claim-primary'() {
    const s = await mkSession('shell', 'e2e-claim-primary');
    const a = new TermClient(env.base, env.token, s.id, { cols: 150, rows: 40, claimOnConnect: true });
    const b = new TermClient(env.base, env.token, s.id, { cols: 80, rows: 24 });
    try {
      await a.connect(); // primary pane: claim + resize, NO typing
      await sleep(300);
      await b.connect(); // passive preview attaches later at 80×24
      await sleep(400);
      let meta = (await api('GET', `/api/v1/sessions/${s.id}`)).meta ?? {};
      check(meta.pty_cols === 150 && meta.pty_rows === 40,
        `untyped primary pane holds its grid vs later passive attach (pty=${meta.pty_cols}x${meta.pty_rows}, want 150x40)`);
      // Authority is STICKY: the primary detaching must NOT let the passive
      // viewer re-pin the grid (agent output printed while the user is away
      // would be hard-wrapped narrow forever).
      a.close();
      await sleep(500);
      b.resize(80, 24);
      b.resize(100, 30);
      await sleep(RESIZE_SETTLE_MS + 400);
      meta = (await api('GET', `/api/v1/sessions/${s.id}`)).meta ?? {};
      check(meta.pty_cols === 150 && meta.pty_rows === 40,
        `authority sticks after primary detaches — passive resize denied (pty=${meta.pty_cols}x${meta.pty_rows}, want 150x40)`);
      // A NEW primary (claims on attach) takes the grid over.
      const a2 = new TermClient(env.base, env.token, s.id, { cols: 120, rows: 36, claimOnConnect: true });
      await a2.connect();
      await sleep(400);
      meta = (await api('GET', `/api/v1/sessions/${s.id}`)).meta ?? {};
      check(meta.pty_cols === 120 && meta.pty_rows === 36,
        `a re-attaching primary claims the grid back (pty=${meta.pty_cols}x${meta.pty_rows}, want 120x36)`);
      a2.close();
    } finally {
      a.close();
      b.close();
      await rmSession(s.id);
    }
  },

  /** Attach with a deep history must be fast (the "3–4s reload" complaint). */
  async 'attach-latency'() {
    const s = await mkSession('shell', 'e2e-latency');
    const c = new TermClient(env.base, env.token, s.id, { cols: 120, rows: 30 });
    const budget = Number(process.env.LATENCY_MS ?? 2500);
    try {
      await c.connect();
      c.type('for i in $(seq 1 4500); do echo "FILL_$i some scrollback ballast text"; done\n');
      await waitFor(() => c.bufferText().includes('FILL_4500'), 40_000);
      c.close();
      await sleep(300);
      const c2 = new TermClient(env.base, env.token, s.id, { cols: 120, rows: 30 });
      const t0 = Date.now();
      await c2.connect(); // resolves when the snapshot has been APPLIED client-side
      const ms = Date.now() - t0;
      check(ms < budget, `attach+rebuild with 4k-line history took ${ms}ms (budget ${budget}ms)`);
      check(c2.bufferText().includes('FILL_4500'), 'rebuilt buffer reaches the latest line');
      c2.close();
    } finally {
      c.close();
      await rmSession(s.id);
    }
  },

  /** REAL claude: prompt → response → resize matrix → reconnect. The response
   *  marker must never multiply. */
  async 'real-claude'() {
    await realAgent('claude', 'OTTO_E2E_MARK_CLA', {
      // claude erases its live region in place on SIGWINCH repaints — growth
      // budget 1 tolerates a single transitional frame, never multiplication.
      liveGrowthBudget: 1,
      reconnectSlack: 1,
    });
  },

  /** REAL codex: same flow. codex re-emits transcript lines on SIGWINCH (in
   *  every terminal — iTerm included), so scrollback growth up to ~2 lines per
   *  resize is upstream parity; MULTIPLICATION (the ×20 bug) must be gone and
   *  the visible screen must stay coherent. */
  async 'real-codex'() {
    await realAgent('codex', 'OTTO_E2E_MARK_COD', {
      liveGrowthBudget: 2 * 4, // parity bound: ≤2 copies per resize × 4 resizes
      reconnectSlack: 2,
    });
  },

  /** REAL codex with a LONG response (the user's exact ×20 report: an essay
   *  spanning screens, pushed through codex's scroll region, multiplied on
   *  every resize). Per-line copies must not multiply. */
  async 'real-codex-long'() {
    const s = await mkSession('codex', 'e2e-codex-long');
    const c = new TermClient(env.base, env.token, s.id, { cols: 120, rows: 32 });
    const NL = 40;
    const maxCount = (text) => {
      let m = 0;
      for (let i = 1; i <= NL; i++) m = Math.max(m, count(text, `CODL_${String(i).padStart(2, '0')}`));
      return m;
    };
    const missing = (text) => {
      let n = 0;
      for (let i = 1; i <= NL; i++) if (count(text, `CODL_${String(i).padStart(2, '0')}`) === 0) n++;
      return n;
    };
    try {
      await c.connect();
      await waitFor(() => c.bufferText().trim().length > 80, 45_000);
      await waitBufferStable(c, { quietMs: 3500, timeoutMs: 90_000 });
      let echoed = false;
      for (let attempt = 0; attempt < 4 && !echoed; attempt++) {
        c.type(`Output the lines CODL_01 through CODL_${NL}, one per line, no other text, no tools.`);
        echoed = !!(await waitFor(() => c.bufferText().includes('CODL_01'), 5000, 250));
        if (!echoed) await sleep(2000);
      }
      check(echoed, 'codex composer echoed the long prompt');
      await sleep(600);
      c.type('\r');
      await waitFor(() => count(c.bufferText(), `CODL_${NL}`) >= 1, 180_000, 500);
      await waitBufferStable(c, { quietMs: 5000, timeoutMs: 90_000 });
      const base = maxCount(c.bufferText());
      check(missing(c.bufferText()) === 0, `all ${NL} lines arrived`);
      check(base >= 1, `long response landed (max per-line ×${base})`);

      for (const [cols, rows] of [[100, 28], [78, 24], [140, 40], [120, 32]]) {
        await c.resizeSettled(cols, rows, 1200);
      }
      await waitBufferStable(c, { quietMs: 3000, timeoutMs: 30_000 });
      const after = maxCount(c.bufferText());
      const vpAfter = maxCount(c.viewportText());
      console.log(`     [codex-long] per-line copies: max ${base}→${after} after 4 resizes (viewport max ×${vpAfter})`);
      check(after - base <= 4, `no multiplication: per-line growth ${after - base} ≤ 4 (parity bound)`);
      check(vpAfter <= 2, `visible screen coherent (per-line max ×${vpAfter} on screen)`);
      check(missing(c.bufferText()) === 0, 'no lines lost through resizes');

      c.close();
      const c2 = new TermClient(env.base, env.token, s.id, { cols: 120, rows: 32 });
      await c2.connect();
      await sleep(800);
      const re = maxCount(c2.bufferText());
      console.log(`     [codex-long] reconnect: per-line max ×${re}`);
      check(re <= after + 1, `reconnect does not amplify (×${re} vs live ×${after})`);
      check(missing(c2.bufferText()) === 0, 'reconnect keeps the whole response');
      c2.close();
    } finally {
      c.close();
      await rmSession(s.id);
    }
  },
};

async function realAgent(provider, marker, { liveGrowthBudget, reconnectSlack }) {
  const s = await mkSession(provider, `e2e-${provider}`);
  const c = new TermClient(env.base, env.token, s.id, { cols: 120, rows: 32 });
  try {
    await c.connect();
    // Let the TUI paint its chrome and finish booting.
    const painted = await waitFor(() => c.bufferText().trim().length > 80, 45_000);
    check(!!painted, `${provider} TUI painted`);
    await waitBufferStable(c, { quietMs: 3500, timeoutMs: 90_000 });

    // Type the prompt and VERIFY the composer echoed it — a TUI that is still
    // initializing its input loop silently swallows early keystrokes.
    let echoed = false;
    for (let attempt = 0; attempt < 4 && !echoed; attempt++) {
      c.type(`Reply with exactly the single line ${marker} and nothing else.`);
      echoed = !!(await waitFor(() => c.bufferText().includes(marker), 5000, 250));
      if (!echoed) await sleep(2000);
    }
    check(echoed, `${provider} composer echoed the typed prompt`);
    await sleep(600);
    c.type('\r');
    // Wait until the response line landed (prompt echo + reply ⇒ ≥2) or the
    // TUI settles with at least one occurrence.
    await waitFor(() => count(c.bufferText(), marker) >= 2, 150_000, 500);
    await waitBufferStable(c, { quietMs: 4000, timeoutMs: 60_000 });
    const bufBase = count(c.bufferText(), marker);
    const vpBase = count(c.viewportText(), marker);
    check(bufBase >= 1, `${provider} responded (marker ×${bufBase} in buffer, ×${vpBase} on screen)`);

    // Four deliberate resizes, each fully settled = 4 real SIGWINCHes.
    const steps = [[100, 28], [78, 24], [140, 40], [120, 32]];
    for (const [cols, rows] of steps) {
      await c.resizeSettled(cols, rows, 1200);
    }
    await waitBufferStable(c, { quietMs: 3000, timeoutMs: 30_000 });
    const bufAfter = count(c.bufferText(), marker);
    const vpAfter = count(c.viewportText(), marker);
    console.log(`     [${provider}] marker count: buffer ${bufBase}→${bufAfter}, viewport ${vpBase}→${vpAfter} after ${steps.length} resizes`);
    check(bufAfter - bufBase <= liveGrowthBudget,
      `live view: no multiplication after ${steps.length} resizes (growth ${bufAfter - bufBase} ≤ ${liveGrowthBudget})`);
    check(vpAfter <= Math.max(vpBase, 2),
      `visible screen stays coherent (marker ×${vpAfter} on screen)`);

    // Fresh reconnect: the server rebuild must not amplify beyond the live view.
    c.close();
    const c2 = new TermClient(env.base, env.token, s.id, { cols: 120, rows: 32 });
    await c2.connect();
    await sleep(800);
    const bufRe = count(c2.bufferText(), marker);
    const vpRe = count(c2.viewportText(), marker);
    console.log(`     [${provider}] reconnect: marker ×${bufRe} in rebuilt buffer, ×${vpRe} on screen`);
    check(bufRe <= bufAfter + reconnectSlack,
      `reconnect does not amplify (×${bufRe} vs live ×${bufAfter} + slack ${reconnectSlack})`);
    check(bufRe >= 1, 'reconnect keeps the response in history');
    c2.close();
  } finally {
    c.close();
    await rmSession(s.id);
  }
}

// ── runner ───────────────────────────────────────────────────────────────────

const names = only.length
  ? only
  : Object.keys(scenarios).filter((n) => !FAST || !n.startsWith('real-'));

for (const name of names) {
  if (!scenarios[name]) {
    console.error(`unknown scenario: ${name} (have: ${Object.keys(scenarios).join(', ')})`);
    process.exit(2);
  }
}

for (const name of names) {
  currentScenario = name;
  console.log(`\n━━━ ${name} ━━━`);
  const t0 = Date.now();
  try {
    await scenarios[name]();
  } catch (e) {
    failures.push(`[${name}] threw: ${e.message}`);
    console.log(`FAIL  [${name}] threw: ${e.stack}`);
  }
  console.log(`━━━ ${name} done in ${((Date.now() - t0) / 1000).toFixed(1)}s`);
}

await env.stop();
console.log(`\n════ SUMMARY: ${failures.length === 0 ? 'ALL PASS' : `${failures.length} FAILURE(S)`} (${names.join(', ')}) ════`);
for (const f of failures) console.log('  ✗ ' + f);
process.exit(failures.length ? 1 : 0);
