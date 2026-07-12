# Quality gates

Conformance proves parseability, not usefulness. Apply every applicable gate before reporting produce or maintain work complete.

## Evidence gate

- Trace claims to source files, code symbols, schemas, queries, configuration, runtime evidence, or authoritative documents.
- Put numbered Markdown links under `# Citations`; cite source locations as precisely as the medium permits.
- Never invent a URL, field, type, enum, join, index, behavior, owner, or relationship.
- Label unresolved facts `Unknown` or `N/A`, state what evidence was checked, and cite that evidence in the same section. A bare marker does not satisfy a depth gate.

## Depth gate

- Service concepts explain boundaries, integrations, configuration, failure behavior, and operations.
- Endpoint concepts satisfy every item in the API endpoint contract in [concept-patterns.md](concept-patterns.md), including complete bodies and examples rather than DTO names alone.
- Datastore concepts satisfy every item in the datastore contract, including field-level impact paths.
- Flow concepts cross API, data, messaging, worker, and external side-effect boundaries where source evidence does.

## Example and diagram gate

Use examples to prove shape and behavior: realistic request/response/error bodies, SQL, key patterns, payloads, or commands. Mark redacted or illustrative values. Never fabricate a “real” sample.

Add a Mermaid or D2 diagram only when it materially clarifies a multi-stage flow, relationship graph, or data impact path. Verify node and edge labels against citations. A diagram supplements prose; it does not replace contracts or evidence.

## Maintenance gate

Augment rather than rewrite:

- Preserve every existing top-level heading in its original order and wording.
- Copy `type`, `title`, and `resource` verbatim unless the underlying identity changed.
- Preserve unknown frontmatter, union-merge tags, and refresh `timestamp` after a meaningful change.
- Extend prose, add bullets or subsections, and append new top-level headings after existing headings.
- Update links, local indexes, and `log.md` in the same pass.

## Deterministic gate

Run both tools from the skill directory:

```bash
python3 scripts/validate_okf.py ROOT --format text
python3 scripts/audit_bundle.py ROOT --format text
```

`validate_okf.py` fails only for E1–E3 conformance errors. `audit_bundle.py` emits findings shaped as `{rule,path,message,severity}`; conformance findings use `error`, depth and warning findings use `warning`. A warning is not permission to invent a fix.

The audit requires substantive prose, populated schemas, and realistic structured examples. A heading, a keyword, a table header without rows, or an arbitrary fenced sentence does not satisfy a gate. When source evidence is incomplete, use the evidence-backed `Unknown`/`N/A` form above instead of filler.

## Completion evidence

Finish with:

- Mode and changed concept/index/log paths.
- Sources consulted and material unknowns.
- Conformance result and checked-note count.
- Quality findings resolved or explicitly blocked by unavailable evidence.
- Confirmation that no existing headings/frontmatter were lost during maintenance.
