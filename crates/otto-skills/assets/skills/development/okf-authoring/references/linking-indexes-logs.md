# Links, indexes, logs, and conversion

Use these rules whenever the concept neighborhood changes.

## Link discipline

- Use `[label](/datasets/orders.md)` for bundle-absolute links in tools or `[label](../datasets/orders.md)` for portable relative links.
- Link a concept once per section at its first useful mention.
- Do not link from headings, fenced code, schema field cells, or to the current concept itself.
- Express relationship kind in prose; links carry no hidden semantics.
- Preserve unresolved links as visible warnings when the target is planned knowledge.

## Index contract

Every directory containing concepts has `index.md`. Group entries under useful headings and format each concept as:

```markdown
* [Display title](concept.md) - One-sentence frontmatter description.
```

Nested indexes have no frontmatter. The root index may begin only with:

```yaml
---
okf_version: "0.1"
---
```

Regenerate indexes through `POST …/okf/indexes` when operating through Otto, or update the affected directory indexes directly when working offline.

## Log contract

Keep `log.md` newest-first, without frontmatter. Use ISO dates and link changed concepts:

```markdown
## 2026-07-12

* **Creation**: Added the [Orders data asset](datasets/orders.md).
* **Update**: Documented transaction behavior for [Create order](endpoints/create-order.md).
* **Deprecation**: Retired the [Legacy import](flows/legacy-import.md).
```

Allowed lead words are `**Initialization**`, `**Creation**`, `**Update**`, and `**Deprecation**`.

## Neighborhood maintenance

After create, move, rename, or deprecate work:

1. Fix inbound and outbound links.
2. Refresh old and new directory indexes.
3. Append one dated log entry describing the knowledge change.
4. Validate links and reserved files.

Deprecate durable history rather than deleting it: add a visible concept note and a `**Deprecation**` log entry. Remove a concept only when its content is truly wrong and no historical value remains.

## Converting notes

For Obsidian, convert `[[Note]]` to standard Markdown, aliases to link text, heading suffixes to anchors, inline tags to `tags`, embeds to links, callouts to blockquotes, and MOC notes to `index.md`.

For Notion, map Name to `title`, Tags to `tags`, URL to `resource`, and Last edited to `timestamp`; strip export UUID suffixes from filenames and links. In both conversions, add a factual `type`, preserve content, update links, and run conformance plus quality audits.
