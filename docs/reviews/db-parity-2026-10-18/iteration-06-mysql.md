# Iteration 06 — MySQL Protocol completion and upload boundaries

The new LOCAL INFILE regression exercises 140,000 bytes in three 64 KiB-or-smaller
packets, a subsequent zero-byte file, exact affected counts 1 then 0, and the
140,000-character stored value. The proxy rewrites the server request to another
existing file and observes zero client upload bytes. These are actual live
checks, not inferred from the sender loop. Both ordinary and rejected requests
retain the existing timeout wrapper and dedicated session retirement.

The review rechecked `vendor/sqlx-mysql/src/connection/executor.rs`: grants are
taken before execution, consumed once, exact-byte compared, and cannot be reused
after server errors. Metadata snapshots remain independent of the upload path.
No additional defect found in this pass.

## Lens assessment

|Lens|Score|Evidence|
|---|---:|---|
|L05 native load/admin forms|9.8|FIELDS/LINES/IGNORE/SET native result checked|
|L06 completion/affected rows|9.8|multi-packet and empty-file acknowledgements checked|
|L07 values/metadata|9.8|140,000-character payload preserved; prior precise-value checks reused|
|L08 session/cancel|9.8|one batch/session and one-shot grants traced; prior timeout test reused|

## Score interpretation and limits

Scores are review assessments, not percentages of the MySQL language or a claim
of universal version coverage. The live fixture is MySQL 9.7.1; MariaDB, Percona,
Aurora and older-server matrices were not run. Preview row/byte/cell caps remain
explicit product limits. LOCAL INFILE accepts regular files up to the existing
100 MiB import ceiling; stdin/FIFOs and unbounded streaming are not implemented.
Root MySQL query requests use a dedicated LOCAL_FILES-enabled connection, so
they pay connection/setup cost instead of reusing the warm shared pool. Shared
pools retain LOCAL_FILES disabled. No driver SQL or file transfer is replayed.
