// Serial request pacer for upstream (forge) calls proxied through the daemon.
// One queue: every task waits until `minIntervalMs` has elapsed since the
// previous ATTEMPT started (retries count as attempts, so a 429 storm never
// tightens the spacing). 429 honours Retry-After (seconds or HTTP date),
// otherwise backs off exponentially (base·2^n, capped). 5xx and network
// errors get a smaller retry budget. `sleep`/`now` are injectable so tests
// run on a fake clock.
'use strict';

const { check, wait, abortable } = require('./cancellation.js');

function headerOf(res, name) {
  const h = res && res.headers;
  if (!h) return null;
  if (typeof h.get === 'function') return h.get(name);
  const k = Object.keys(h).find((x) => x.toLowerCase() === name.toLowerCase());
  return k ? h[k] : null;
}

/** Retry-After → ms, or null when absent/unparseable. */
function parseRetryAfter(value, nowMs) {
  if (value == null || value === '') return null;
  const n = Number(value);
  if (Number.isFinite(n)) return Math.max(0, n * 1000);
  const t = Date.parse(value);
  return Number.isFinite(t) ? Math.max(0, t - nowMs) : null;
}

function createPacer(opts = {}) {
  const minIntervalMs = opts.minIntervalMs ?? 2000;
  const maxRetries = opts.maxRetries ?? 5;
  const max5xxRetries = Math.min(opts.max5xxRetries ?? 2, maxRetries);
  const baseBackoffMs = opts.baseBackoffMs ?? 2000;
  const capMs = opts.capMs ?? 60000;
  const sleep = opts.sleep;
  const now = opts.now || Date.now;
  const onWait = opts.onWait || (() => {});

  let lastStart = null;
  let tail = Promise.resolve();
  const stats = { calls: 0, retries: 0, throttled: 0, last_backoff_ms: 0 };

  const backoff = (n) => Math.min(capMs, baseBackoffMs * 2 ** n);

  async function spaced(signal) {
    check(signal);
    if (lastStart != null) {
      const wait = lastStart + minIntervalMs - now();
      if (wait > 0) await pause(wait, signal);
    }
    check(signal);
    lastStart = now();
    stats.calls++;
  }

  /** Earliest time the next call may start, relative to now (ms). */
  function nextCallEtaMs() {
    return lastStart == null ? 0 : Math.max(0, lastStart + minIntervalMs - now());
  }

  const pause = (ms, signal) => wait(ms, signal, sleep);

  async function runTask(task, signal) {
    let n429 = 0;
    let n5xx = 0;
    for (;;) {
      await spaced(signal);
      let res;
      let err = null;
      try {
        res = await task();
      } catch (e) {
        err = e;
      }
      check(signal);
      const status = err ? 0 : Number(res && res.status) || 0;
      if (status === 429) {
        stats.throttled++;
        if (n429 >= maxRetries) return res;
        const ra = parseRetryAfter(headerOf(res, 'retry-after'), now());
        const wait = ra != null ? Math.min(ra, capMs) : backoff(n429);
        n429++;
        stats.retries++;
        stats.last_backoff_ms = wait;
        onWait({ reason: 'throttled', backoff_ms: wait, attempt: n429 });
        await pause(wait, signal);
        continue;
      }
      if (err || status >= 500) {
        if (n5xx >= max5xxRetries) {
          if (err) throw err;
          return res;
        }
        const wait = backoff(n5xx);
        n5xx++;
        stats.retries++;
        stats.last_backoff_ms = wait;
        onWait({ reason: err ? 'network' : 'server', backoff_ms: wait, attempt: n5xx });
        await pause(wait, signal);
        continue;
      }
      return res;
    }
  }

  /** Enqueue `task` (→ Promise<Response-like>); resolves with its final response. */
  function schedule(task, { signal } = {}) {
    const p = tail.then(() => runTask(task, signal));
    tail = p.catch(() => {});
    return abortable(p, signal);
  }

  return { schedule, nextCallEtaMs, stats };
}

module.exports = { createPacer, parseRetryAfter };
