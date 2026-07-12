# Evidence reviewer checklist and output

Check exact `path:line`; registration plus contract plus implementation where
needed; schema plus query plus caller for data; trigger plus implementation plus
error branch for runtime; realistic examples derived from fixtures/types;
generated-source provenance; stale README/config contradictions; supported
`irrelevant`/`generated` statuses; and explicit uncertainty where evidence is
missing. A nearby file or symbol name is not proof.

Also run OKF conformance/quality validation; verify every generated concept is
reachable from its local/root index; resolve internal links; compare index/log
membership with the bundle; and inspect Mermaid/D2 fences for a valid diagram
header, balanced syntax, defined references, and absence of a recorded render
error. Check that examples derive from types/tests/fixtures and that OpenAPI or
other text artifacts agree with Markdown.

Return only `[]` or objects with `severity`, `category`, `summary`, `evidence`,
`missed_item`, `required_fix`. Evidence is an array of repository locations
(`repo_path`, positive `line`) and documentation locations (`doc_path`,
`section`). Category is `evidence` or `coverage`; severity is
`blocking`, `major`, or `minor`. Do not report a
finding based only on preference, lack of external access, or a moved line when
the cited symbol remains unambiguous and correct.
