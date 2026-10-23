import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { componentFunctions } from './componentFunctions.ts';
import { loadSource } from './sourceHarness.ts';

// accessibility-responsive-6w2-02 (+ the self-removing stop controls): the
// product chats' stop notice is spoken ONCE — by `speakStopNotice` (the
// component's `$effect`) through the shell announcer — and "Stop" / "Stop
// waiting" / "Reload …" hand focus to a named successor instead of <body>.
// The effects are driven by calling their functions after each state change,
// as Svelte re-runs them; repeated calls with unchanged state are re-renders.

const BACKGROUND_NOTICE = 'Stopped waiting — the agent turn may still finish in the background.';

/** The real announcer (`lib/announce.svelte.ts`) with frames run by hand. */
function announcer() {
  let frames: (() => void)[] = [];
  const mod = loadSource(new URL('../src/lib/announce.svelte.ts', import.meta.url), {}, {
    requestAnimationFrame: (fn: () => void) => frames.push(fn),
    cancelAnimationFrame: () => {},
  });
  const heard: [string, string][] = [];
  const flush = () => {
    const due = frames;
    frames = [];
    due.forEach((f) => f());
    for (const ch of ['polite', 'assertive'] as const) {
      if (mod.announcer[ch]) heard.push([ch, mod.announcer[ch]]);
      mod.announcer[ch] = '';
    }
  };
  return { mod, heard, flush };
}

for (const [file, idKey, load, stop, reloadLabel] of [
  ['DiscoveryChat', 'cid', 'loadChat', 'stopDiscoveryTurn', 'Reload chat'],
  ['RefineChat', 'tid', 'loadThread', 'stopRefinementTurn', 'Reload thread'],
] as const) {
  const url = new URL(`../src/modules/product/${file}.svelte`, import.meta.url);

  function fixture(stopResult: { stopped: boolean } = { stopped: true }) {
    const a = announcer();
    const focused: string[] = [];
    const body = { name: 'body' };
    const document = { body, activeElement: body as unknown };
    const loads: string[] = [];
    const state = componentFunctions(
      url,
      ['stopTurn', 'stopWaiting', 'reloadAfterStop', 'speakStopNotice', 'rescueStopFocus'],
      {
        [idKey]: 'A',
        loadSeq: 0,
        alive: true,
        stopRequestedTurns: new Set(),
        inFlightTurnId: 'turn-1',
        messages: [{ id: 'u1' }],
        sending: true,
        stopping: false,
        activeTurnId: 'turn-1',
        stopNotice: null,
        backgroundTurn: false,
        BACKGROUND_NOTICE,
        sayStop: a.mod.announceEdge(),
        document,
        reloadBtn: null,
        composerEl: { focus() { focused.push('composer'); document.activeElement = this; } },
        tick: async () => {},
        [load]: async (id: string) => { loads.push(id); },
        product: { [stop]: async () => stopResult },
        toastError() {},
      },
    );
    /** The after-stop bubble mounted: `bind:this` set the reload button. */
    const mountReload = () => {
      state.reloadBtn = {
        focus() {
          focused.push(reloadLabel);
          document.activeElement = this;
        },
      };
    };
    /** Svelte re-running both effects after a state change, twice (a re-render). */
    const effects = () => {
      for (let i = 0; i < 2; i++) {
        state.speakStopNotice();
        state.rescueStopFocus();
        a.flush();
      }
    };
    return { state, heard: a.heard, focused, document, loads, mountReload, effects };
  }

  test(`${file}: "Stop waiting" is announced once and focus lands on "${reloadLabel}"`, async () => {
    const f = fixture();
    f.effects(); // the turn in flight: nothing to say, nothing to move
    assert.deepEqual(f.heard, []);
    f.state.stopWaiting();
    f.mountReload();
    f.effects();
    assert.deepEqual(f.heard, [['polite', BACKGROUND_NOTICE]], 'spoken exactly once');
    assert.deepEqual(f.focused, [reloadLabel], 'focus moved off <body> to the reload button');
  });

  test(`${file}: "Reload …" focuses the composer after the reload, and a later stop speaks again`, async () => {
    const f = fixture();
    f.state.stopWaiting();
    f.mountReload();
    f.effects();
    f.focused.length = 0;
    const reloading = f.state.reloadAfterStop();
    f.state.reloadBtn = null; // the after-stop bubble unmounted: focus falls to <body>
    f.document.activeElement = f.document.body;
    f.effects();
    await reloading;
    assert.deepEqual(f.loads, ['A']);
    assert.deepEqual(f.focused, ['composer']);
    // The notice is gone, so the edge re-arms: the next stop is a new event.
    f.state.sending = true;
    f.state.activeTurnId = 'turn-2';
    f.state.stopWaiting();
    f.effects();
    assert.deepEqual(f.heard, [['polite', BACKGROUND_NOTICE], ['polite', BACKGROUND_NOTICE]]);
  });

  test(`${file}: a confirmed "Stop" says "Turn stopped." once and focuses "${reloadLabel}"`, async () => {
    const f = fixture({ stopped: true });
    await f.state.stopTurn();
    f.mountReload();
    f.effects();
    assert.equal(f.state.sending, false);
    assert.deepEqual(f.heard, [['polite', 'Turn stopped.']]);
    assert.deepEqual(f.focused, [reloadLabel]);
  });

  test(`${file}: focus the person moved elsewhere is not taken back`, () => {
    const f = fixture();
    f.document.activeElement = { name: 'search box' };
    f.state.stopWaiting();
    f.mountReload();
    f.effects();
    assert.deepEqual(f.focused, []);
    assert.equal(f.heard.length, 1);
  });

  test(`${file}: the bubbles that show the notice are not live regions (structure)`, () => {
    const markup = readFileSync(url, 'utf8').split('</script>')[1].split('<style>')[0];
    assert.doesNotMatch(markup, /role="status"|aria-live|use:announceShown/,
      'a status bubble around the notice would speak it a second time');
  });
}
