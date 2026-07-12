# Maintain mode before and after

This example shows an additive edit after source evidence adds retry behavior. Existing identity fields, an unknown `owner` key, tags, and top-level headings survive.

## Before

```markdown
---
type: Service
title: Import service
description: Imports partner catalog records.
resource: "service://catalog-import"
tags: [catalog]
owner: commerce-platform
timestamp: 2026-06-01T08:00:00Z
---

# Overview

The service imports partner catalog records.

# Operations

The service runs from the partner upload endpoint.

# Citations

[1] [Import entry point](../../src/import/mod.rs#L10)
```

## After

```markdown
---
type: Service
title: Import service
description: Imports partner catalog records.
resource: "service://catalog-import"
tags: [catalog, retries]
owner: commerce-platform
timestamp: 2026-07-12T09:00:00Z
---

# Overview

The service imports partner catalog records. Failed transient partner calls are retried by the import worker.

# Operations

The service runs from the partner upload endpoint.

## Retry behavior

The worker retries timeout responses three times with the backoff defined in configuration. Validation failures are not retried.

# Citations

[1] [Import entry point](../../src/import/mod.rs#L10)
[2] [Worker retry loop](../../src/import/worker.rs#L48)
[3] [Retry configuration](../../config/import.yaml#L12)

# Failure handling

After the final transient failure, the worker records the import as failed and leaves the source file available for an operator retry.
```

Then refresh the local index description if needed and append:

```markdown
## 2026-07-12

* **Update**: Documented retry and failure behavior for the [Import service](services/import-service.md).
```
