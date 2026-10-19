# Iteration 07 — MySQL Incremental session-mode parsing

Finding fixed: eager splitting under the original MySQL backslash rules could
merge later statements after SET/EXECUTE changed NO_BACKSLASH_ESCAPES. The driver
now acquires its session first and calls `StatementScanner::next` with the
server's current status flag after every executed statement. The scanner keeps
one character-index buffer, offset and DELIMITER state; its one initial
single-statement lookahead adds at most a linear scan, not repeated suffix
rescanning. Single-query pagination, shared response budget and batch error-stop
behavior remain intact.

`mysql_batch_uses_current_mode_after_set_and_execute` exercises direct SET and
prepared EXECUTE changing the mode, a literal ending in backslash, switching
back, and a final SELECT 77. All expected result rows survive. The incremental
unit checks mode changes, byte offsets, DELIMITER changes and nonconsuming
lookahead; all 26 preceding splitter tests still pass.

LOCAL policy no longer predicts all future statement boundaries: the service
grants actual root host-file authority independently of text, the root request
negotiates capability on a dedicated session, and the driver checks actual
parsed LOAD before reading a file. Non-root never receives that authority. The
live infile case includes SET NO_BACKSLASH_ESCAPES, an ending-backslash SELECT,
and a later LOAD, which passes.

## Lens assessment

|Lens|Score|Evidence|
|---|---:|---|
|L05 native commands|9.8|SET/PREPARE/EXECUTE plus delayed LOCAL verified|
|L06 batch fidelity|9.8|later statements and boundaries survive mode toggles|
|L07 values/metadata|9.8|literal backslashes preserved; earlier type tests reused|
|L08 session state|9.8|mode comes from current server status without observer SQL|

## Score interpretation and limits

Scores are review assessments, not percentages of the MySQL language or a claim
of universal version coverage. The live fixture is MySQL 9.7.1; MariaDB, Percona,
Aurora and older-server matrices were not run. Preview row/byte/cell caps remain
explicit product limits. LOCAL INFILE accepts regular files up to the existing
100 MiB import ceiling; stdin/FIFOs and unbounded streaming are not implemented.
Root MySQL query requests use a dedicated LOCAL_FILES-enabled connection, so
they pay connection/setup cost instead of reusing the warm shared pool. Shared
pools retain LOCAL_FILES disabled. No driver SQL or file transfer is replayed.
