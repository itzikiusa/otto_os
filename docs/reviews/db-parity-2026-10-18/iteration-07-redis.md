# Redis Parity — Iteration 07: Database State, `SELECT` & Per-Connection Modes

Focused review pass on logical database selection (`kdb:<n>`), intra-batch `SELECT`, negative database index guards, and per-connection `CLIENT` state across `crates/otto-dbviewer/src/drivers/redis.rs` and `crates/otto-dbviewer/src/drivers/redis/transport.rs`.

## Concrete Cases Traced & Verified

1. **Initial Database Scope (`keyspace_for` → `connect_isolated` Handshake)**:
   - Every entry point (`schema_children`, `object_detail`, `run`) resolves the target database index via `parse_db_index` / `keyspace_for(cfg, req)` (`redis.rs:437–492`), accepting `kdb:<n>`, `kdb:<n>/...`, `db:<n>`, `db<n>`, or `<n>`, falling back to `cfg.database` or `0`.
   - `transport::from_stream` (`transport.rs:212–216`) sends `SELECT <db>` during connection establishment whenever `info.db != 0`, before any user command runs.
2. **Intra-Batch `SELECT <db>` Without Cross-Request or Metadata Bleed**:
   - Within a single `RedisDriver::run` batch (`SELECT 7\nSET parity:db7:only yes\nGET parity:db7:only`), `SELECT 7` mutates the isolated connection's active database for the remainder of that batch.
   - Cached metadata connections in `self.clients` (keyed by `<cache_key>|db=<n>`) are never used by `run()`, so schema tree browsing and `object_detail` previews on `kdb:0` or `kdb:1` are never switched to `db7`.
   - The next `run()` call opens a new connection pinned to `keyspace_for(cfg, req)` (e.g., `db0`), where `GET parity:db7:only` returns `(nil)`.
3. **Negative & Malformed Database Index Rejection**:
   - `parse_db_index` (`redis.rs:440–446`) rejects any negative integer (`kdb:-1`, `db:-2`, `-1`, or `cfg.database = Some("-1")`) with `Error::Invalid("redis: invalid database index ...")` before touching `self.clients` or opening a socket.
4. **Per-Connection `CLIENT` Modes vs Framing Desynchronizers**:
   - Safe per-connection modes (`CLIENT SETNAME`, `CLIENT CACHING`, `CLIENT NO-EVICT`, `CLIENT NO-TOUCH`, `CLIENT SETINFO`, `READONLY`, `READWRITE`) are permitted on the isolated `run()` connection and die with the socket at the end of the batch.
   - Commands that suppress replies or switch wire framing (`CLIENT REPLY OFF|SKIP`, `CLIENT TRACKING ON`, `HELLO`, `AUTH`, `RESET`, `QUIT`) remain refused by `refuse_connection_state_command_bytes` (`redis.rs:598–655`) because `MultiplexedConnection` expects one RESP2 reply frame per command in the batch.

## Scores (Iteration 07)

| Lens | Score | Evidence Boundary |
|------|------:|-------------------|
| **L13** | 9.8 | `SELECT` and per-connection `CLIENT` verbs classified and exposed in completions; `CLIENT` introspection (`LIST`, `INFO`, `ID`, `GETNAME`, `GETREDIR`, `TRACKINGINFO`) allowed in read-only mode. |
| **L14** | 9.8 | Multi-command batches starting with `SELECT <db>` preserve per-statement previews and results. |
| **L15** | 9.8 | `SELECT` returns `QueryResult::message("OK")`; invalid DB indices (`SELECT 999999`) surface the server `ERR DB index is out of range` cleanly. |
| **L16** | 9.8 | Zero database state bleed between `run()` calls or cached metadata connections; negative DB indices rejected at the boundary. |
