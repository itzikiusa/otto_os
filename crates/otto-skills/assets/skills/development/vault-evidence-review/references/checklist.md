# Evidence reviewer checklist and output

Check exact `path:line`; registration plus contract plus implementation where
needed; schema plus query plus caller for data; trigger plus implementation plus
error branch for runtime; realistic examples derived from fixtures/types;
generated-source provenance; stale README/config contradictions; supported
`irrelevant`/`generated` statuses; and explicit uncertainty where evidence is
missing. A nearby file or symbol name is not proof.

Return only `[]` or objects with `severity`, `category`, `summary`, `doc`,
`source`, `evidence`, `repair`. Category is `evidence` or `coverage`; source is
the location that disproves or fails to support the claim. Do not report a
finding based only on preference, lack of external access, or a moved line when
the cited symbol remains unambiguous and correct.
