# Iteration 04 — MySQL LOCAL INFILE

Implementation authorized by root after review of `mysql-local-infile-proposal.md`.
This closes the remaining explicitly identified L05 native protocol gap; scores
remain pending until live verification, rather than assuming the new path works.

## Implemented behavior

- Recognize explicit LOAD DATA/XML [LOW_PRIORITY|CONCURRENT] LOCAL INFILE across
  submitted statement spans. Existing server-side LOAD DATA INFILE stays native.
- Service-owned trusted flag follows the existing root-only host-file policy;
  driver requires it before file access. Root integrated the shared service
  hooks; this worker changed no shared service file.
- Root-authorized MySQL query requests create a dedicated pool cloned from the
  normal connection options/setup callbacks, with LOCAL_FILES opt-in. One session
  remains pinned for the entire batch, preserving temporary tables and variables.
- Literal filenames decode using the server status NO_BACKSLASH_ESCAPES bit,
  without an observer SELECT that would alter ROW_COUNT()/FOUND_ROWS(). Native clauses
  (FIELDS, LINES, IGNORE, column variables and SET expressions) are sent unchanged
  for the server to execute.
- Read only the named regular file, with the existing import 100 MiB ceiling.
  SQLx receives bytes plus an exact filename grant, never a filesystem callback.
- Server0xFB must match the one-shot grant. Another filename or missing/reused
  grant closes the transport without sending file bytes. Upload uses 64 KiB chunks,
  protocol continuation sequence IDs and empty terminator, then native status.
- Dedicated sessions retire after the request and use the existing timeout/KILL
  handling. No query or file-transfer replay is introduced.

## Verification added (executed)

- Unit recognition tests distinguish local load from server load and quoted SQL;
  filename decoding covers NO_BACKSLASH_ESCAPES, escaped newline, unknown escape
  and doubled apostrophe.
- New live `mysql_local_infile` test exercises native FIELDS/LINES/IGNORE/SET,
  same-batch user variables and temporary table, escaped filenames with prior
  INSERT ROW_COUNT preservation, NO_BACKSLASH_ESCAPES, exact affected rows/data, and
  rejects a driver request without the trusted local-file authorization.
- A protocol proxy relays actual MySQL auth/setup but substitutes a different
  existing filename in server 0xFB. The test checks the replacement happened,
  response is an error, and zero client upload bytes followed the malicious
  request. Only the originally named file is opened by the driver.
- Fixture global local_infile setting is captured, enabled for the test, then
  restored before assertions. This only touches the isolated dev MySQL fixture.

`rustfmt` and scoped `git diff --check` passed. The completed live/test gates
are recorded below and in the final iteration report.

## Final verification update

LOCAL INFILE live scenario passed, including 140,000-byte chunked input, empty
file, escaped filename ROW_COUNT preservation, mixed-mode ending-backslash
SELECT before LOAD, NO_BACKSLASH_ESCAPES, and proxy mismatch with zero upload.
Final gate details and honest limits are in iteration-10-mysql.md. Root permission
now grants capability independent of eager SQL detection; every actual parsed
LOAD still needs the trusted flag and exact one-shot filename bytes.
