# Iteration 08 — MySQL ANSI_QUOTES and native result boundaries

A live diagnostic confirmed a distinct correctness defect:

```sql
SET SESSION sql_mode='ANSI_QUOTES';
SELECT 1 AS "ends\";
SELECT 2 AS last_result;
```

The native client returned two named rowsets. Otto's fixed double-quote lexical
assumption grouped the SELECTs into one chunk, and the reader merged rows 1 and 2
under the first schema. This was not a hypothetical finding.

Fixed the bounded defect: `exec_read_conn` finalizes each protocol completion,
resets columns/decoders, and preserves subsequent native rowsets and command
acknowledgements even if the client sees a single chunk. `finish_read_rows`
retains chunked background decoding. No metadata query or command replay added.
The permanent regression uses BIGINT 1 then VARCHAR 'second', with distinct column
names, so incorrect decoder/schema reuse fails visibly. Full 32-test live suite
passed after this change, including the prior capped-read performance tests.

Residual: scanner does not obtain an ANSI_QUOTES status bit (the protocol has
none). Therefore exotic mixtures of backslash-ending double-quoted identifiers
and quote-containing string literals are not a fully mode-aware client lexer.
A further script containing `SELECT 'x" ; y'` after that identifier also fails
in the stock mysql CLI with 1064 near `'x"`; this is not presented as an Otto-only
CLI parity defect. Full server-language lexical equivalence remains unclaimed.

## Lens assessment

|Lens|Score|Evidence|
|---|---:|---|
|L05 native forms|9.8|native ANSI_QUOTES SELECT form executes|
|L06 result fidelity|9.8|confirmed merged-rowset defect fixed and live-tested|
|L07 values/metadata|9.8|different result schemas and decoders verified|
|L08 SQL modes|9.8|NO_BACKSLASH fixed; ANSI_QUOTES lexer boundary explicitly qualified|

## Score interpretation and limits

Scores are review assessments, not percentages of the MySQL language or a claim
of universal version coverage. The live fixture is MySQL 9.7.1; MariaDB, Percona,
Aurora and older-server matrices were not run. Preview row/byte/cell caps remain
explicit product limits. LOCAL INFILE accepts regular files up to the existing
100 MiB import ceiling; stdin/FIFOs and unbounded streaming are not implemented.
Root MySQL query requests use a dedicated LOCAL_FILES-enabled connection, so
they pay connection/setup cost instead of reusing the warm shared pool. Shared
pools retain LOCAL_FILES disabled. No driver SQL or file transfer is replayed.
