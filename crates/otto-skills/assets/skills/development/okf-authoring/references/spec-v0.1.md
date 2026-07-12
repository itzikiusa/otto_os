# OKF v0.1 format and conformance

Use this reference for exact bundle rules. Otto follows OKF v0.1 from `GoogleCloudPlatform/knowledge-catalog/okf/SPEC.md`; Otto's deterministic E1–E3/W1–W5 taxonomy is implemented by its Vault validator.

## Bundle and concept identity

- An OKF bundle is a directory tree of Markdown files.
- A concept is any `.md` file other than reserved `index.md` and `log.md`.
- A concept ID is its bundle-relative path without `.md`: `services/auth-api.md` becomes `services/auth-api`.
- Paths are the identity. Do not add a second ID scheme.

## Concept frontmatter

Start every concept with a YAML mapping between `---` delimiters.

| Field | Requirement | Rule |
|---|---|---|
| `type` | Required | Non-empty, free-form concept kind |
| `title` | Recommended | Display name; filename is fallback |
| `description` | Recommended | Exactly one sentence used by search and indexes |
| `resource` | Conditional | Canonical URI for a real asset; omit for abstract concepts |
| `tags` | Optional | Short YAML string list |
| `timestamp` | Recommended | ISO 8601 time of last meaningful change |

Extra keys are legal. Preserve every unknown key during maintenance. Put documentation URLs in `# Citations`, not `resource`.

## Reserved files

- `index.md` is not a concept. Nested indexes have no frontmatter. Only the bundle-root `index.md` may have frontmatter, and it may contain only `okf_version: "0.1"`.
- `log.md` is not a concept and has no frontmatter. Its level-two headings use `YYYY-MM-DD`.

See [linking-indexes-logs.md](linking-indexes-logs.md) for their body formats.

## Links

Use standard Markdown links. Bundle-absolute paths begin at `/`; file-relative paths begin at the current concept. Do not emit wikilinks. A broken internal link is legal knowledge debt and therefore a warning, not a conformance error.

## Deterministic rules

| Rule | Class | Meaning |
|---|---|---|
| E1 | Error | Concept has no frontmatter or unparseable frontmatter |
| E2 | Error | Concept has missing or empty `type` |
| E3 | Error | Reserved-file frontmatter violates the rules above |
| W1 | Warning | Concept lacks `title` or `description` |
| W2 | Warning | Internal Markdown link does not resolve |
| W3 | Warning | Concept lacks `timestamp` |
| W4 | Warning | Directory containing concepts lacks `index.md` |
| W5 | Warning | `log.md` level-two heading is not `YYYY-MM-DD` |

A bundle is conformant only when it has no E1–E3 findings. Warnings never make conformance validation fail.

## Offline validator

Run `scripts/validate_okf.py ROOT --format json|text`. It walks `.md` paths in lexical order, reads UTF-8 without writing, parses a conservative top-level YAML mapping, and returns nonzero only for conformance errors. Its JSON result contains `conformant`, `errors`, `warnings`, and `checked_notes`.

For Vault-hosted bundles, the authoritative runtime equivalents are `otto_vault_okf_validate` or `POST /api/v1/workspaces/{ws}/vault/vaults/{id}/okf/validate`.
