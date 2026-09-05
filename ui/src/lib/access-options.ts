import type { ResourceKind } from './api/types';
// Mirrors the closed operation catalogue in otto-core::access.
export const accessOperations: Record<ResourceKind, string[]> = {
  connection: [
    'discover',
    'db_browse',
    'db_query',
    'db_export',
    'db_data',
    'db_schema',
    'configure',
    'manage_access',
    'change_submit',
    'change_approve',
    'change_execute',
    'shell',
    'sftp_read',
    'sftp_write',
  ],
  mcp_server: ['discover', 'invoke', 'configure', 'manage_access', 'approve'],
  aws_account: [
    'discover',
    'configure',
    'manage_access',
    's3_list',
    's3_read',
    's3_write',
    's3_delete',
    's3_buckets',
    'ec2_view',
    'ec2_start',
    'ec2_stop',
    'ec2_reboot',
    'ec2_terminate',
    'sqs_view',
    'sqs_send',
    'sqs_receive',
    'sqs_delete',
    'sqs_purge',
    'sqs_redrive',
    'athena_view',
    'athena_query',
    'eks_view',
    'eks_import',
    'rds_view',
    'metrics',
  ],
  k8s_cluster: [
    'discover',
    'configure',
    'manage_access',
    'workloads_view',
    'resources_view',
    'secrets_view',
    'logs',
    'metrics',
    'exec',
    'k9s',
    'apply',
    'scale',
    'restart',
    'delete',
  ],
};
const labels: Record<string, string> = {
  discover: 'See resource',
  db_browse: 'Browse database schema',
  db_query: 'Query data',
  db_export: 'Export data',
  db_data: 'Modify data',
  db_schema: 'Modify schema',
  configure: 'Configure resource',
  manage_access: 'Manage access',
  change_submit: 'Submit database changes',
  change_approve: 'Approve database changes',
  change_execute: 'Execute approved changes',
  shell: 'Open terminal',
  sftp_read: 'Browse and download files',
  sftp_write: 'Write and delete files',
  invoke: 'Invoke tools',
  approve: 'Approve invocations',
  secrets_view: 'View secret values',
  k9s: 'Open k9s',
  exec: 'Exec into containers',
  s3_buckets: 'Create and delete buckets',
};
export const operationLabel = (operation: string) =>
  labels[operation] ??
  operation
    .split('_')
    .map((word, i) =>
      ['s3', 'ec2', 'sqs', 'eks', 'rds'].includes(word)
        ? word.toUpperCase()
        : i === 0
          ? word[0].toUpperCase() + word.slice(1)
          : word,
    )
    .join(' ');
export const resourceLabels: Record<ResourceKind, string> = {
  connection: 'Connection',
  mcp_server: 'MCP server',
  aws_account: 'AWS account',
  k8s_cluster: 'Kubernetes cluster',
};
export const scopeHint: Record<ResourceKind, string> = {
  connection: 'Exact database names, one per line.',
  mcp_server: 'Exact tool names, one per line.',
  aws_account: 'Exact bucket scopes, one per line: bucket:my-bucket',
  k8s_cluster: 'Exact namespace scopes, one per line: namespace:production',
};
/** Database Explorer tree IDs wrap the native name in db:/kdb: segments. */
export function databaseAccessChild(node: string | null | undefined): string | undefined {
  if (!node) return undefined;
  const segment = node.split('/').find((part) => part.startsWith('db:') || part.startsWith('kdb:'));
  return segment ? segment.slice(segment.indexOf(':') + 1) : node;
}
