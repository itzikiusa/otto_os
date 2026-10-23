// Display labels for the automation editor's checks and run statuses.
import type { ApiAssertion, ApiAutomationRun } from '../../lib/api/types';

export const KIND_LABEL: Record<ApiAssertion['kind'], string> = {
  status: 'Status code',
  json_path: 'JSON value at',
  header: 'Response header',
  body_text: 'Response body text',
  duration_ms: 'Response time (ms)',
};
export const OP_LABEL: Record<ApiAssertion['op'], string> = {
  eq: 'equals',
  ne: 'does not equal',
  contains: 'contains',
  lt: 'is less than',
  gt: 'is greater than',
  lte: 'is at most',
  gte: 'is at least',
  exists: 'exists',
  not_exists: 'does not exist',
  matches: 'matches regex',
};
/** Operators that take no expected value. */
export const NO_VALUE_OPS: ReadonlySet<ApiAssertion['op']> = new Set(['exists', 'not_exists']);
export const RUN_STATUS: Record<ApiAutomationRun['status'], string> = {
  running: 'running', passed: 'passed', failed: 'failed', cancelled: 'canceled', interrupted: 'interrupted',
};
