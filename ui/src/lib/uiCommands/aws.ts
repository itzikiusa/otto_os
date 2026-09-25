// Agent UI control — AWS handlers (`otto.ui_aws_*`). Navigation goes through
// the module's own routes (`#/aws/<account>/<service>`, S3 keeps bucket +
// prefix in the route) so the user watches the same view the agent reads;
// the reads use the same `awsApi` calls those views make (the daemon's AWS
// RBAC + IAM apply exactly as for the user's clicks).
//
// `aws_sqs_send` is OUTWARD (a message leaves this Mac into a real queue that
// other systems consume): it always asks with the where/what/who confirm and
// is never remembered.

import { registerUiCommands, registerUiState, UiCommandError, type UiCommandCtx } from '../uiCommands';
import { router } from '../router.svelte';
import { aws } from '../stores/aws.svelte';
import { awsApi } from '../api/aws';
import { confirmOutward } from '../confirmOutward';
import { toasts } from '../toast.svelte';
import type { AwsAccount, AwsService } from '../api/types';
import { agentLabel, asUiError, capList, dismissOnAbort, highlightWhenReady, resolveByIdOrName, tailText, waitFor } from './pagePort';

const SERVICES: readonly AwsService[] = ['s3', 'sqs', 'ec2', 'athena', 'eks', 'rds'];

async function accountFor(key: string, ctx: UiCommandCtx): Promise<AwsAccount> {
  if (aws.accounts.length === 0) await aws.loadAccounts();
  if (ctx.signal.aborted) throw new UiCommandError('cancelled_by_user', 'Cancelled');
  return resolveByIdOrName(aws.accounts, key, (a) => a.id, (a) => a.name, 'AWS account');
}

/** Route to an account's service view and wait for AwsPage to show it. */
async function showService(a: AwsAccount, svc: AwsService, ctx: UiCommandCtx, tail = ''): Promise<void> {
  if (!aws.serviceAllowed(a.id, svc)) {
    throw new UiCommandError('forbidden', `The account “${a.name}”'s IAM doesn't allow ${svc.toUpperCase()}.`);
  }
  router.go(`aws/${encodeURIComponent(a.id)}/${svc}${tail}`);
  await waitFor(() => router.parts[0] === 'aws' && router.parts[1] === a.id && router.parts[2] === svc, ctx.signal, 5000, 'the AWS view');
}

function s3Seg(bucket: string, prefix: string): string {
  return prefix ? `${encodeURIComponent(bucket)}?prefix=${encodeURIComponent(prefix)}` : encodeURIComponent(bucket);
}

const envNote = (a: AwsAccount): string => (a.environment === 'prod' ? ' (PRODUCTION)' : '');

registerUiCommands('aws', {
  async aws_list_accounts(_args, ctx) {
    router.go('aws');
    await aws.loadAccounts();
    void highlightWhenReady(ctx, '.aws-page, [data-testid="aws-page"]');
    return {
      accounts: aws.accounts.map((a) => ({
        id: a.id,
        name: a.name,
        region: a.region,
        environment: a.environment,
        auth_mode: a.auth_mode,
        services: aws.perms(a.id)?.services ?? null,
      })),
    };
  },

  async aws_open(args: { account: string; service?: AwsService }, ctx) {
    const a = await accountFor(args.account, ctx);
    if (args.service && !SERVICES.includes(args.service)) {
      throw new UiCommandError('invalid_args', `Unknown service “${args.service}” (one of ${SERVICES.join(', ')}).`);
    }
    if (args.service) await showService(a, args.service, ctx);
    else router.go(`aws/${encodeURIComponent(a.id)}`);
    return { account: { id: a.id, name: a.name, environment: a.environment }, service: args.service ?? null };
  },

  async aws_s3_browse(args: { account: string; bucket?: string; prefix?: string }, ctx) {
    const a = await accountFor(args.account, ctx);
    try {
      if (!args.bucket) {
        await showService(a, 's3', ctx);
        const buckets = await aws.loadS3Buckets(a.id);
        const { items, total, truncated } = capList(buckets);
        return { account: a.name, buckets: items, total, truncated };
      }
      const prefix = args.prefix ?? '';
      await showService(a, 's3', ctx, `/${s3Seg(args.bucket, prefix)}`);
      const r = await awsApi.s3Objects(a.id, args.bucket, prefix, null, 200);
      return {
        account: a.name,
        bucket: args.bucket,
        prefix,
        folders: r.prefixes,
        objects: r.objects,
        is_truncated: r.is_truncated,
      };
    } catch (e) {
      throw asUiError(e);
    }
  },

  async aws_s3_preview(args: { account: string; bucket: string; key: string }, ctx) {
    const a = await accountFor(args.account, ctx);
    const slash = args.key.lastIndexOf('/');
    const prefix = slash >= 0 ? args.key.slice(0, slash + 1) : '';
    await showService(a, 's3', ctx, `/${s3Seg(args.bucket, prefix)}`);
    try {
      const p = await awsApi.s3Preview(a.id, args.bucket, args.key, 256 * 1024);
      if (p.binary) return { bucket: args.bucket, key: args.key, binary: true, content_type: p.content_type ?? null };
      const t = tailText(p.text ?? '', 60_000);
      return {
        bucket: args.bucket,
        key: args.key,
        content_type: p.content_type ?? null,
        text: t.text,
        truncated: !!p.truncated || t.truncated,
      };
    } catch (e) {
      throw asUiError(e);
    }
  },

  async aws_sqs_list_queues(args: { account: string; prefix?: string }, ctx) {
    const a = await accountFor(args.account, ctx);
    await showService(a, 'sqs', ctx);
    try {
      const queues = await aws.loadSqsQueues(a.id, args.prefix ?? '');
      const { items, total, truncated } = capList(queues);
      // Approximate depth for the first few, like the view's own row badges.
      const attrs = await Promise.all(items.slice(0, 20).map((q) => aws.loadSqsAttrs(a.id, q.url)));
      return {
        account: a.name,
        total,
        truncated,
        queues: items.map((q, i) => ({
          ...q,
          approx_messages: attrs[i]?.approx_messages ?? null,
          approx_not_visible: attrs[i]?.approx_not_visible ?? null,
        })),
      };
    } catch (e) {
      throw asUiError(e);
    }
  },

  async aws_ec2_list(args: { account: string; region?: string; state?: string; query?: string }, ctx) {
    const a = await accountFor(args.account, ctx);
    await showService(a, 'ec2', ctx);
    try {
      const list = await aws.loadEc2(a.id, args.region ?? '', args.state, args.query);
      const { items, total, truncated } = capList(list);
      return { account: a.name, region: args.region || a.region, total, truncated, instances: items };
    } catch (e) {
      throw asUiError(e);
    }
  },

  async aws_sqs_send(
    args: { account: string; queue: string; body: string; delay_seconds?: number; group_id?: string; dedup_id?: string },
    ctx,
  ) {
    const a = await accountFor(args.account, ctx);
    await showService(a, 'sqs', ctx);
    const queues = aws.sqsQueues[a.id] ?? (await aws.loadSqsQueues(a.id));
    const q = resolveByIdOrName(queues, args.queue, (x) => x.url, (x) => x.name, 'SQS queue');
    if (q.fifo && !args.group_id) throw new UiCommandError('invalid_args', 'A FIFO queue needs `group_id`.');
    const who = agentLabel(ctx.agent);
    ctx.progress(`Waiting for you to confirm sending to ${q.name}`, true);
    const ok = await dismissOnAbort(ctx.signal, confirmOutward({
      verb: 'Send message',
      title: `${who} wants to send an SQS message`,
      where: `SQS queue “${q.name}” in ${a.name}${envNote(a)}`,
      what: args.body,
      who: 'Every consumer of this queue receives it.',
      danger: a.environment === 'prod',
    }));
    if (!ok) throw new UiCommandError('cancelled_by_user', 'The user declined sending the message.');
    try {
      const r = await awsApi.sqsSend(a.id, {
        url: q.url,
        body: args.body,
        delay_seconds: args.delay_seconds || undefined,
        group_id: q.fifo ? args.group_id : undefined,
        dedup_id: q.fifo ? args.dedup_id || undefined : undefined,
      });
      toasts.success('Message sent', `${q.name} · ${who}`);
      void aws.loadSqsAttrs(a.id, q.url);
      return { queue: q.name, message_id: r.message_id };
    } catch (e) {
      throw asUiError(e);
    }
  },
});

// `otto.ui_state` view: the account + service the route shows.
registerUiState('aws', () => {
  const a = aws.account(router.parts[1] ?? null);
  return {
    account: a ? { id: a.id, name: a.name, environment: a.environment } : null,
    service: router.parts[2] ?? null,
    s3: router.parts[2] === 's3' && router.parts[3] ? router.parts[3] : null,
  };
});
