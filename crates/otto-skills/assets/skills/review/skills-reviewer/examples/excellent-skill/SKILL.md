---
name: excellent-skill
description: Use this skill when reviewing small API migration pull requests for the Acme Payments service, especially changes touching idempotency keys, retry behavior, webhook signatures, or migration SQL.
license: MIT
metadata:
  version: "0.1.0"
---

# Acme Payments Migration Review

Use this workflow for Acme Payments API migration reviews.

## Workflow

1. Identify changed endpoints, SQL migrations, and webhook handlers.
2. Check that every write endpoint uses `Idempotency-Key` and returns the existing result on retry.
3. Verify SQL migrations are backward-compatible for one deploy cycle.
4. Check webhook signature verification before parsing the body.
5. Return findings in severity order with file/line evidence.

## Gotchas

- `payment_id` in API JSON maps to `payments.external_id`, not `payments.id`.
- Webhook retries may arrive out of order; never assume `created_at` order equals event order.
- `status=settled` can still be reversed for 24 hours.

## Examples

See `examples/review-idempotency.md` for a happy path and `examples/review-webhook-ordering.md` for an edge case.

## Validation

Before finalizing, confirm the review includes:
- one idempotency check;
- one migration compatibility check if SQL changed;
- webhook signature handling if webhooks changed.
