// Pure context-packet builders for AWS CloudWatch Logs and SQS.
// Every packet runs client-side defense-in-depth scrubbing and strict size
// capping before the daemon's own `/context-packet/preview` redaction step.

/** Maximum character budget for text payloads sent to an agent (16 KiB). */
export const MAX_CONTEXT_CHARS = 16_384;

/** Scrub obvious inline credentials/tokens before leaving the view (the daemon
 *  also runs its full secret/PII redactor and shows the preview diff). */
export function scrubInlineSecrets(text: string): string {
  return text
    .replace(/-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z0-9 ]*PRIVATE KEY-----/g, '[REDACTED_PRIVATE_KEY]')
    .replace(/\b(Bearer\s+)[A-Za-z0-9._~+/=-]{8,}/gi, '$1[REDACTED]')
    .replace(/\b(?:AKIA|ASIA)[0-9A-Z]{16}\b/g, '[REDACTED_AWS_KEY]')
    .replace(/(https?:\/\/)([^/\s:@]+):([^/\s@]+)@/gi, '$1[REDACTED]@')
    .replace(
      /\b(password|passwd|secret|api[_-]?key|access[_-]?token|refresh[_-]?token)(["']?\s*[:=]\s*)(?:(")[^"]*"|(')[^']*'|[^\s,;}\]]+)/gi,
      (_m, key: string, sep: string, dq?: string, sq?: string) => `${key}${sep}${dq ? '"[REDACTED]"' : sq ? "'[REDACTED]'" : '[REDACTED]'}`,
    );
}

/** Cap `text` to `maxChars`, keeping either the start (`head`) or the end
 *  (`tail`, useful for logs where the crash sits at the bottom). */
export function capText(
  text: string,
  maxChars = MAX_CONTEXT_CHARS,
  keep: 'head' | 'tail' = 'head',
): { text: string; truncated: boolean; total_chars: number } {
  const total_chars = text.length;
  if (total_chars <= maxChars) return { text, truncated: false, total_chars };
  const clipped = keep === 'tail' ? text.slice(total_chars - maxChars) : text.slice(0, maxChars);
  return { text: clipped, truncated: true, total_chars };
}

export interface AwsLogEventPacketInput {
  account: string;
  environment: string;
  region: string;
  group: string;
  event: { timestamp: number; stream: string; message: string };
  maxChars?: number;
}

/** Build a scrubbed, size-capped context packet for one CloudWatch Logs event. */
export function buildAwsLogEventPacket(input: AwsLogEventPacketInput) {
  const scrubbed = scrubInlineSecrets(input.event.message);
  const capped = capText(scrubbed, input.maxChars ?? MAX_CONTEXT_CHARS, 'head');
  return {
    source: 'aws_cloudwatch_logs' as const,
    account: input.account,
    environment: input.environment,
    region: input.region,
    log_group: input.group,
    stream: input.event.stream,
    timestamp_iso: new Date(input.event.timestamp).toISOString(),
    message: capped.text,
    truncated: capped.truncated,
    total_chars: capped.total_chars,
  };
}

export interface AwsInsightsPacketInput {
  account: string;
  environment: string;
  region: string;
  groups: readonly string[];
  query: string;
  columns: readonly { name: string }[];
  rows: readonly (readonly unknown[])[];
  maxRows?: number;
  maxChars?: number;
}

/** Build a scrubbed, size-capped context packet for CloudWatch Logs Insights rows. */
export function buildAwsInsightsPacket(input: AwsInsightsPacketInput) {
  const maxRows = input.maxRows ?? 50;
  const maxChars = input.maxChars ?? MAX_CONTEXT_CHARS;
  const outRows: Record<string, unknown>[] = [];
  let usedChars = 0;
  let truncated = input.rows.length > maxRows;

  for (let i = 0; i < Math.min(input.rows.length, maxRows); i++) {
    const rawRow = input.rows[i];
    const obj: Record<string, unknown> = {};
    for (let c = 0; c < input.columns.length; c++) {
      const val = rawRow[c];
      obj[input.columns[c].name] = typeof val === 'string' ? scrubInlineSecrets(val) : val;
    }
    const serialized = JSON.stringify(obj);
    if (usedChars + serialized.length > maxChars && outRows.length > 0) {
      truncated = true;
      break;
    }
    if (serialized.length > maxChars && outRows.length === 0) {
      // Single oversized row: truncate string fields so the packet still fits.
      const clipped: Record<string, unknown> = {};
      const perCol = Math.max(128, Math.floor(maxChars / Math.max(1, input.columns.length)));
      for (const [k, v] of Object.entries(obj)) {
        clipped[k] = typeof v === 'string' && v.length > perCol ? `${v.slice(0, perCol)}…` : v;
      }
      outRows.push(clipped);
      truncated = true;
      break;
    }
    usedChars += serialized.length;
    outRows.push(obj);
  }

  return {
    source: 'aws_cloudwatch_insights' as const,
    account: input.account,
    environment: input.environment,
    region: input.region,
    groups: [...input.groups],
    query: input.query.trim(),
    total_rows: input.rows.length,
    included_rows: outRows.length,
    truncated,
    rows: outRows,
  };
}

export interface SqsMessagePacketInput {
  account: string;
  environment: string;
  region: string;
  queueName: string;
  queueUrl: string;
  message: {
    message_id: string;
    body: string;
    attributes: Record<string, string>;
    message_attributes?: Record<string, unknown>;
  };
  maxChars?: number;
}

/** Build a scrubbed, size-capped context packet for one peeked SQS message.
 *  Intentionally omits `receipt_handle` so an agent can never delete/extend the
 *  in-flight lease using the packet. */
export function buildSqsMessagePacket(input: SqsMessagePacketInput) {
  const scrubbed = scrubInlineSecrets(input.message.body);
  const capped = capText(scrubbed, input.maxChars ?? MAX_CONTEXT_CHARS, 'head');
  const sentMs = Number(input.message.attributes.SentTimestamp);
  return {
    source: 'aws_sqs' as const,
    account: input.account,
    environment: input.environment,
    region: input.region,
    queue: input.queueName,
    queue_url: input.queueUrl,
    message_id: input.message.message_id,
    sent_iso: Number.isFinite(sentMs) && sentMs > 0 ? new Date(sentMs).toISOString() : null,
    receive_count: input.message.attributes.ApproximateReceiveCount ?? null,
    message_attributes: input.message.message_attributes ?? {},
    body: capped.text,
    truncated: capped.truncated,
    total_chars: capped.total_chars,
  };
}
