# Iteration 09 — MySQL Types, limits and session observation

Rechecked typed decoding and bounded output after reader refactoring. Each
completion now resets the decoder array, which matters when one native chunk
returns an integer set then a text set. Empty metadata stays distinct from
command acknowledgements. The preexisting precise-value fixture covers unsigned
integers, binary, decimal, JSON, null and extended/negative TIME; export uses
native text values and empty-schema headers. No fabricated broad type matrix is
claimed beyond those tests.

The LOCAL filename parser previously queried @@session.sql_mode for escaped
paths, changing ROW_COUNT()/FOUND_ROWS() before LOAD SET expressions. That
observer query is removed: `MySqlConnection::no_backslash_escapes()` reads the
already tracked server status. Live INSERT 2 rows then escaped-path LOAD records
ROW_COUNT 2. A separate NO_BACKSLASH filename and same-batch session variables
pass. The payload 100 MiB bound and response caps are explicit limits rather than
silent claims of unlimited values.

The full 32 live tests were already rerun for the reader change; this independent
source sweep did not trigger redundant test execution.

## Lens assessment

|Lens|Score|Evidence|
|---|---:|---|
|L05 native expressions|9.8|LOAD SET receives unmodified native session counters|
|L06 rows/acknowledgements|9.8|metadata completion logic re-traced|
|L07 values and limits|9.8|existing exact-value/export proof plus cross-schema regression|
|L08 state observation|9.8|observer SELECT eliminated; ROW_COUNT live regression passes|

## Score interpretation and limits

Scores are review assessments, not percentages of the MySQL language or a claim
of universal version coverage. The live fixture is MySQL 9.7.1; MariaDB, Percona,
Aurora and older-server matrices were not run. Preview row/byte/cell caps remain
explicit product limits. LOCAL INFILE accepts regular files up to the existing
100 MiB import ceiling; stdin/FIFOs and unbounded streaming are not implemented.
Root MySQL query requests use a dedicated LOCAL_FILES-enabled connection, so
they pay connection/setup cost instead of reusing the warm shared pool. Shared
pools retain LOCAL_FILES disabled. No driver SQL or file transfer is replayed.
