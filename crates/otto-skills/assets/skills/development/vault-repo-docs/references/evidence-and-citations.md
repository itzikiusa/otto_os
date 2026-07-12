# Evidence and citations

Use repository-relative `path:line` citations. Cite registration and contract
definitions, not only implementations. A load-bearing claim should be
independently recoverable from its citation.

Evidence hierarchy:

1. executable source and migrations;
2. tests and fixtures that exercise the current source;
3. generated contracts checked into the same commit;
4. configuration defaults and deployment manifests;
5. prose docs, which may be stale and must be cross-checked.

For APIs, cite route + DTO + handler. For data, cite schema + query + caller.
For runtime behavior, cite trigger registration + implementation + retry/error
branch. Include exact examples derived from fixtures or types, never plausible
guesses.

When sources disagree, record the disagreement as `uncertain` in coverage.md.
State what was checked and what additional evidence would resolve it. Never
convert uncertainty into an unqualified statement for a cleaner-looking scan.
