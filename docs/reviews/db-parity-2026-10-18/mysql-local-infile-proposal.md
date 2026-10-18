# Remaining MySQL LOCAL INFILE capability

Investigation only; no local-file protocol implementation has been added.

## Current evidence

- `vendor/sqlx-mysql/src/connection/stream.rs` does not negotiate LOCAL_FILES.
- `vendor/sqlx-mysql/src/connection/executor.rs` explicitly identifies LocalInfileRequest as unhandled.
- Existing `/db/import` (`service.rs` import_from_path) parses a local file and emits INSERT batches. It is useful but does not preserve native LOAD DATA FIELDS/LINES/IGNORE/SET semantics.
- `service.rs::require_local_path_access` already reserves daemon-host file access for root users. This existing rule is the correct authorization boundary; no new permission ceremony is proposed.

## Feasible implementation

1. In the authenticated service boundary, identify explicit LOAD DATA LOCAL INFILE requests and apply require_local_path_access, preserving saved-read-only and write-confirmation checks.
2. Decode the explicitly named literal filename using MySQL lexical rules. Authorize only that local file for this command. Upload endpoints could similarly supply explicitly selected bytes without permitting arbitrary path reads.
3. Negotiate LOCAL_FILES only on a connection/pool configured for these authorized requests. An isolated request connection must remain shared across the complete submitted batch so temporary tables, variables and transactions still work.
4. Expose a one-shot SQLx command-local payload grant keyed by the expected filename, preferably bytes/stream supplied by Otto rather than a SQLx filesystem-opening callback.
5. On server packet0xFB, require an exact filename match against the grant, transfer the supplied bytes with correct packet sequencing and empty terminator, then consume the native OK/error result. A malicious/misconfigured server cannot choose another host path.
6. Clear the grant after one use or cancellation, and discard interrupted sessions. Retain existing timeout and response bounds. Test FIELDS/LINES behavior, affected rows, native failure, cancellation and a deliberately mismatched server filename.

This expands the vendor patch by roughly4–6 files (connection option/state, capability negotiation, executor packet branch and payload encoding), plus the service authorization bridge and driver request setup. It is implementable, but materially broader than the approved metadata-only patch. Root has been asked to review this direction before it proceeds.

Alternative: retain the explicit unsupported-native-command limitation and existing Import UI, without calling that universal native parity. Merely enabling LOCAL_FILES globally or allowing a server-requested path callback is not a faithful implementation of the user's explicitly submitted local-file command.
