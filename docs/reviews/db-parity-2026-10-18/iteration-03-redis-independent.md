# Iteration 03 — independent Redis review (L13–L16)

Read-only review by the MySQL reviewer while Redis implementation proceeds elsewhere. No Redis code was edited and no Redis tests were claimed as executed. This report applies to the **pre-isolated-batch working snapshot**, not the pending implementation.

Base HEAD: `191824b35d695c3593bae06b31c20f86a5403ee0`.
Snapshot SHA-256:

- redis.rs: `3650e709d72263673578e7ba844837af2ccb5685e64404b8a9db93c245718dd4`
- redis/transport.rs: `51b2b9de4355e0be3f226f91ce62e4ad39229396963fa01536acff4fa17aae62`
- redis/framing.rs: `8110304ec00078472b6eefc85f9c9869582431f33f96d923ee7f5dd63d26df93`

| Lens | Snapshot score | Review outcome |
| --- | ---: | --- |
| L13 command/object/read classification | 9.5 | Native command dispatch and saved-read-only classification remain separate; stateful command admission is assessed under L16. |
| L14 argument/batch parsing | 8.5 | Confirmed argument byte mismatches versus redis-cli parser; batches preserve command order and partial-error results. |
| L15 response types/errors | 9.5 | Ordinary scalar/array/map conversion, large integers, binary and nonfinite doubles traced. Attributes are discarded intentionally by current grid shaping; nested maps become string-keyed JSON, not a lossless RESP envelope. |
| L16 state/blocking/cancel | 8.0 | Current snapshot refuses SELECT/MULTI/EXEC/WATCH etc; isolated batch connections are pending. Receive budgets/retirement trace clean; fixed transport30s needs reconciliation with longer requested deadlines. |

Scores reflect this snapshot and must not be used as the verdict on later code.

## L14 confirmed finding: redis-cli argument mismatch

**Major, source-traced:** `crates/otto-dbviewer/src/drivers/redis.rs:1189` (`try_split_args`) documents redis-cli/sdssplitargs compatibility but sends different bytes:

- `ECHO "a\q"`: native produces `aq`; Otto retains the backslash.
- Single-quoted `ECHO 'a\\b'`: native preserves both backslashes; Otto collapses them.
- `ECHO "\X41"`: native treats uppercase X as an ordinary escaped character and produces `X41`; Otto hex-decodes to `A`.
- `ECHO "foo"bar`: native rejects a closed quote followed by non-whitespace; Otto concatenates it to `foobar`.

Source: official [Redis sdssplitargs implementation](https://raw.githubusercontent.com/redis/redis/unstable/src/sds.c), lines1017–1064 inspected live. Double quotes recognize lowercase x hex and otherwise consume the escaped character; single quotes special-case only escaped apostrophe; both closing quote branches reject adjacent characters.

Fix direction: mirror those three branches and add exact-byte/error regressions. Do not remove the application's intentionally supported whole-line/inline comments merely to imitate every CLI affordance. This finding was sent to root for its Redis owner's implementation.

A native CLI reproduction was attempted against the dev Redis container but it was stopped; the Docker response was "container ... is not running". No live reproduction is claimed.

## L16 pending implementation check: timeout ownership

`redis/transport.rs:204` sets an unconditional30-second redis-rs response deadline. Once isolated blocking commands are admitted, `BLPOP key 45` with timeout_ms60000 would still fail at30seconds. This is not reachable through the current refusal guard, so it is a **pending-design check**, not a current confirmed runtime defect. Pass the effective command/request deadline into isolated transport construction or document the stronger fixed cap.

## Traced branches without additional defects

- The gate is acquired before resetting the shared receive budget; concurrent requests cannot reset one another's in-flight budget.
- Cancelling while waiting for the gate does not retire a connection being used by another request (request guard is constructed after admission).
- Dropping an admitted request retires/aborts the transport; unrecoverable errors/timeouts also retire it; no automatic command replay.
- Frames charge payload sizes before accepting bulk contents and cap aggregate nodes/depth. Nested completion accounting and pipeline cumulative budgets were traced.
- Arrays/maps truncate before JSON conversion; integer values use precision-preserving shared shaping; binary values are base64, finite/nonfinite doubles are explicit.
- Read/write classification remains independent of native command passthrough and should continue honoring saved read-only profiles.

## Explicit console limits

MONITOR/pubsub push streams require a separate streaming UI/protocol contract. They should remain explicitly unsupported by the bounded request/result console. Other ordinary stateful/blocking commands should use an isolated request connection rather than expanding the shared-connection refusal list.
