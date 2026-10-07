// Scan-scoped cooperative cancellation. No process-wide state: unrelated scans,
// reports and interactive requests retain their own lifetime.
'use strict';
function check(signal) {
  if (signal) signal.throwIfAborted();
}
function wait(ms, signal, sleep) {
  check(signal);
  return new Promise((resolve, reject) => {
    let timer;
    const finish = (fn, value) => {
      clearTimeout(timer);
      if (signal) signal.removeEventListener('abort', abort);
      fn(value);
    };
    const abort = () => finish(reject, signal.reason);
    if (signal) signal.addEventListener('abort', abort, { once: true });
    if (sleep) Promise.resolve().then(() => sleep(ms)).then(() => finish(resolve), (e) => finish(reject, e));
    else timer = setTimeout(() => finish(resolve), Math.max(0, ms));
  });
}
// A cancelled queued task must not wait for another account's active request.
// Its original queue promise remains observed and checks cancellation before
// dispatch when it eventually reaches the head.
function abortable(promise, signal) {
  if (!signal) return promise;
  return new Promise((resolve, reject) => {
    const abort = () => reject(signal.reason);
    signal.addEventListener('abort', abort, { once: true });
    if (signal.aborted) abort();
    promise.then(resolve, reject).finally(() => signal.removeEventListener('abort', abort));
  });
}
module.exports = { check, wait, abortable };
