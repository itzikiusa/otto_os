// A small OKF markdown bundle ("Platform Docs") with enough cross-links for a
// lively knowledge graph. Entirely fictional.
import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';

const NOTES = {
  'index.md': ['Index', 'Reference', [], 'The platform handbook. Start with [[checkout-api]], [[payments]] and [[deploy-runbook]].'],
  'services/checkout-api.md': ['Checkout API', 'Service', ['checkout', 'api'], 'Takes orders and charges cards through [[payments]]. Rate limited — see [[rate-limits]]. Emits events to [[orders-events]]. Owned by [[team-payments]].'],
  'services/payments.md': ['Payments', 'Service', ['payments'], 'Wraps the card processor. Retries soft declines with [[retry-policy]]. Settlements land in [[payments-settled]].'],
  'services/catalog.md': ['Catalog', 'Service', ['catalog'], 'Products, prices and stock. Read by [[checkout-api]]; changes stream to [[inventory-changes]].'],
  'services/refunds.md': ['Refunds', 'Service', ['refunds'], 'Instant refunds to store credit. Calls [[payments]] and checks [[fraud-scoring]].'],
  'services/fraud-scoring.md': ['Fraud scoring', 'Service', ['risk'], 'Scores orders 0–1. Above 0.8 routes to a human. Used by [[refunds]] and [[checkout-api]].'],
  'data/orders-events.md': ['orders.events', 'Dataset', ['kafka'], 'Kafka topic with order lifecycle events. Produced by [[checkout-api]], consumed by [[analytics]].'],
  'data/payments-settled.md': ['payments.settled', 'Dataset', ['kafka'], 'Settlement events from [[payments]].'],
  'data/inventory-changes.md': ['inventory.changes', 'Dataset', ['kafka'], 'Stock deltas from [[catalog]].'],
  'data/analytics.md': ['Analytics warehouse', 'Dataset', ['clickhouse'], 'ClickHouse tables fed by [[orders-events]] and [[payments-settled]].'],
  'decisions/rate-limits.md': ['ADR-012 Rate limits', 'Decision', ['adr'], 'Token bucket, 60 requests per minute per client, on [[checkout-api]]. See [[deploy-runbook]] for rollout.'],
  'decisions/retry-policy.md': ['ADR-009 Retry policy', 'Decision', ['adr'], 'Exponential backoff, never retry hard declines. Applies to [[payments]] and webhooks.'],
  'runbooks/deploy-runbook.md': ['Deploy runbook', 'Runbook', ['oncall'], 'Ship behind a flag, watch [[checkout-api]] error rate, roll back with one command. Page [[team-payments]].'],
  'runbooks/incident-checklist.md': ['Incident checklist', 'Runbook', ['oncall'], 'Declare, assign roles, update status page. Link the [[deploy-runbook]].'],
  'teams/team-payments.md': ['Payments squad', 'Team', ['team'], 'Noah, Iris, Maya. Owns [[checkout-api]], [[payments]] and [[refunds]].'],
  'teams/team-catalog.md': ['Catalog squad', 'Team', ['team'], 'Owns [[catalog]] and [[inventory-changes]].'],
};

export function writeVault(root) {
  mkdirSync(root, { recursive: true });
  for (const [rel, [title, type, tags, body]] of Object.entries(NOTES)) {
    const f = join(root, rel);
    mkdirSync(dirname(f), { recursive: true });
    const front = rel === 'index.md' ? '---\nokf_version: "0.1"\n---\n' : `---\ntype: ${type}\ntitle: ${title}\ndescription: ${body.split('.')[0]}.\ntags: [${tags.join(', ')}]\ntimestamp: 2026-09-20T10:00:00Z\n---\n`;
    writeFileSync(f, `${front}\n# ${title}\n\n${body}\n`);
  }
  return root;
}
