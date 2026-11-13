// Insights ▸ "Generating the report" poll cadence (modules/insights/runPoll.ts):
// 3 s for the first ~5 minutes, then 30 s for as long as the run is active —
// a 2-hour run keeps its banner instead of timing out after 5 minutes.
import { test, mock } from 'node:test';
import assert from 'node:assert/strict';
import { FAST_POLLS, FAST_POLL_MS, SLOW_POLL_MS, runPollDelay } from '../src/modules/insights/runPoll.ts';

test('fast for the first 100 ticks, then every 30 s, never a stop', () => {
  assert.equal(runPollDelay(0), 3_000);
  assert.equal(runPollDelay(FAST_POLLS - 1), 3_000);
  assert.equal(runPollDelay(FAST_POLLS), 30_000);
  assert.equal(runPollDelay(10_000), 30_000, 'no cap: a long run keeps polling');
  assert.equal(FAST_POLLS * FAST_POLL_MS, 5 * 60_000, '≈ 5 minutes fast');
});

test('a 2-hour run is polled throughout and the loop ends only when the run is gone', () => {
  mock.timers.enable({ apis: ['setTimeout'] });
  try {
    const RUN_MS = 2 * 60 * 60_000;
    let elapsed = 0;
    let ticks = 0;
    let active = true;
    let ended = false;
    // The page's loop, reduced to its schedule: tick, then reschedule while active.
    const schedule = (): void => {
      if (!active) {
        ended = true;
        return;
      }
      const wait = runPollDelay(ticks);
      setTimeout(() => {
        elapsed += wait;
        ticks += 1;
        if (elapsed >= RUN_MS) active = false;
        schedule();
      }, wait);
    };
    // Step the clock a second at a time so each tick's reschedule is due in turn.
    const advance = (ms: number): void => {
      for (let t = 0; t < ms; t += 1_000) mock.timers.tick(1_000);
    };
    schedule();
    advance(5 * 60_000);
    assert.equal(ticks, FAST_POLLS, '100 fast ticks in the first 5 minutes');
    assert.equal(ended, false, 'still polling after 5 minutes');
    advance(60_000);
    assert.equal(ticks, FAST_POLLS + 2, 'then two ticks a minute');
    advance(RUN_MS);
    assert.equal(ended, true, 'the loop stops once the run is gone');
    const slowTicks = Math.ceil((RUN_MS - 5 * 60_000) / SLOW_POLL_MS);
    assert.equal(ticks, FAST_POLLS + slowTicks);
  } finally {
    mock.timers.reset();
  }
});
