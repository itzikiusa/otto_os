# Iteration06 — independent Redis isolation review

Read-only review of latest operation-isolated run path and transport ownership. No Redis files edited and no tests/Cargo executed by this reviewer.

| Lens | Score | Evidence |
|---|---:|---|
| L13 command coverage | 9.7 | run accepts SELECT and transaction batches on dedicated connections; unknown/module commands pass through. Streaming pubsub/replication/MONITOR and transport credential/protocol changes remain explicit console limitations. Current owner gate pending. |
| L14 arguments/batches | 9.8 | run and read-only classifier share byte parser; quoted command names and binary arguments reach raw arg bytes. Batch stops first failure and keeps prior results. |
| L15 results/errors | 9.8 | Single command preserves top-level shape, batch shares ByteBudget, RESP3 nonfinite doubles remain inspectable strings, binary replies retain base64, server errors are not command retries. |
| L16 isolation/control | 9.7 | Fresh connection per run, initial DB handshake, SELECT and MULTI stay within batch, metadata cache separate. Owner Drop aborts transport driver; cancelled RequestGuard retires connection. Explicit long timeout raises transport allowance beyond30sec. No-timeout setting retains30sec transport default, so this is bounded console execution. New owner live gate pending. |

Traced `RedisDriver::run`, `connect_isolated`, `redis_connection_info`, `refuse_connection_state_command`, `statement_is_read_only`, transport Connection/Owner/RequestGuard, and reply conversion. No new isolation blocker found. Request-owned connection has no cached clone, so dropping the operation reaches the final Owner drop rather than leaving the socket held by a shared pool. RequestGuard owns Owner only while executing, and the driver task captures alive rather than Owner, avoiding an ownership cycle.

This review supports conditional9.8 isolation/coverage scores after the new live/session gates pass; it does not certify tests the reviewer did not execute or unlimited native-console compatibility.
