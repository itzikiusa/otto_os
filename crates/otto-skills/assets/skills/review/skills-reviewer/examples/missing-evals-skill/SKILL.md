---
name: missing-evals-skill
description: Use this skill when converting legacy CSV exports from the Northstar billing system into normalized JSON invoices.
---

# Northstar CSV to JSON

1. Read the CSV.
2. Map `cust_id` to `customer.id`.
3. Map `gross_cents` to `invoice.gross.amount_cents`.
4. Emit one JSON file per invoice.

## Gotchas

- `gross_cents` can be negative for refunds.
- Empty `tax_code` means exempt, not unknown.

## Example

Input row:

```csv
cust_id,invoice_id,gross_cents,tax_code
C-1,INV-9,-1200,
```

Output shape:

```json
{"customer":{"id":"C-1"},"invoice":{"id":"INV-9","gross":{"amount_cents":-1200},"tax":{"status":"exempt"}}}
```
