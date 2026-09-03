// AWS console API client — thin typed wrappers over the generic `api` helper.
// Mirrors docs/design/aws-k8s-consoles.md §2 (`/aws/*`, crates/otto-aws). Every
// service call takes an account id; `region` overrides the account's default.

import { api, baseUrl, getToken, ApiError } from './client';
import type {
  AthenaExecution,
  AthenaQueryReq,
  AthenaQueryStatus,
  AthenaTable,
  AthenaWorkgroup,
  AwsAccount,
  AwsPermissions,
  AwsRegion,
  AwsStatus,
  AwsTestResp,
  DiscoveredProfile,
  Ec2Action,
  Ec2ActionResp,
  Ec2Instance,
  Ec2InstanceDetail,
  EksClusterDetail,
  EksClusterSummary,
  EksImportReq,
  EksImportResp,
  InstallJob,
  MetricsNamespace,
  MetricsRange,
  MetricsResp,
  Problem,
  RdsInstance,
  RdsInstanceDetail,
  S3Bucket,
  S3ListObjectsResp,
  S3ObjectHead,
  S3PreviewResp,
  Session,
  SqsMessage,
  SqsPeekReq,
  SqsQueue,
  SqsQueueAttributesResp,
  SqsRedriveReq,
  SqsSendReq,
  UpsertAwsAccountReq,
} from './types';

/** Build `?a=b&c=d` from defined, non-empty values (empty string → omitted). */
function qs(params: Record<string, string | number | boolean | null | undefined>): string {
  const u = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) {
    if (v === undefined || v === null || v === '') continue;
    u.set(k, String(v));
  }
  const s = u.toString();
  return s ? `?${s}` : '';
}

const acct = (id: string) => `/aws/accounts/${encodeURIComponent(id)}`;

/** True when a daemon error means "credentials expired / missing — run `aws sso
 *  login`" (the contract's `login required:` message prefix). */
export function isLoginRequired(e: unknown): boolean {
  return e instanceof Error && /^login required/i.test(e.message);
}

/** True when the daemon reports the `aws` binary is missing. */
export function isNotInstalled(e: unknown): boolean {
  return e instanceof Error && /not installed/i.test(e.message);
}

export const awsApi = {
  // --- plumbing ---
  status: () => api.get<AwsStatus>('/aws/status'),
  install: () => api.post<InstallJob>('/aws/install', {}),
  discover: () => api.get<{ profiles: DiscoveredProfile[] }>('/aws/discover'),
  regions: () => api.get<{ regions: AwsRegion[] }>('/aws/regions'),

  // --- accounts ---
  listAccounts: () => api.get<AwsAccount[]>('/aws/accounts'),
  createAccount: (body: UpsertAwsAccountReq) => api.post<AwsAccount>('/aws/accounts', body),
  getAccount: (id: string) => api.get<AwsAccount>(acct(id)),
  updateAccount: (id: string, body: Partial<UpsertAwsAccountReq>) =>
    api.patch<AwsAccount>(acct(id), body),
  deleteAccount: (id: string) => api.del<void>(acct(id)),
  test: (id: string) => api.post<AwsTestResp>(`${acct(id)}/test`, {}),
  permissions: (id: string, refresh = false) =>
    api.get<AwsPermissions>(`${acct(id)}/permissions${qs({ refresh: refresh ? 'true' : '' })}`),
  /** Spawn `aws sso login` in a PTY session (profile accounts only). */
  login: (id: string, workspaceId: string) =>
    api.post<Session>(`${acct(id)}/login`, { workspace_id: workspaceId }),

  // --- S3 ---
  s3Buckets: (id: string) => api.get<{ buckets: S3Bucket[] }>(`${acct(id)}/s3/buckets`),
  s3Objects: (id: string, bucket: string, prefix = '', token?: string | null, max?: number) =>
    api.get<S3ListObjectsResp>(
      `${acct(id)}/s3/buckets/${encodeURIComponent(bucket)}/objects${qs({ prefix, token, max })}`,
    ),
  s3Head: (id: string, bucket: string, key: string) =>
    api.get<S3ObjectHead>(
      `${acct(id)}/s3/buckets/${encodeURIComponent(bucket)}/object${qs({ key })}`,
    ),
  s3Preview: (id: string, bucket: string, key: string, maxBytes?: number) =>
    api.get<S3PreviewResp>(
      `${acct(id)}/s3/buckets/${encodeURIComponent(bucket)}/preview${qs({ key, max_bytes: maxBytes })}`,
    ),
  /** Handler-relative path of the streamed download (feed to `awsDownloadBlob`). */
  s3DownloadPath: (id: string, bucket: string, key: string) =>
    `${acct(id)}/s3/buckets/${encodeURIComponent(bucket)}/download${qs({ key })}`,

  // --- SQS ---
  sqsQueues: (id: string, prefix = '') =>
    api.get<{ queues: SqsQueue[] }>(`${acct(id)}/sqs/queues${qs({ prefix })}`),
  sqsAttributes: (id: string, url: string) =>
    api.get<SqsQueueAttributesResp>(`${acct(id)}/sqs/queues/attributes${qs({ url })}`),
  sqsPeek: (id: string, body: SqsPeekReq) =>
    api.post<{ messages: SqsMessage[] }>(`${acct(id)}/sqs/queues/peek`, body),
  sqsSend: (id: string, body: SqsSendReq) =>
    api.post<{ message_id: string }>(`${acct(id)}/sqs/queues/send`, body),
  sqsDeleteMessage: (id: string, url: string, receipt_handle: string) =>
    api.post<void>(`${acct(id)}/sqs/queues/delete-message`, { url, receipt_handle }),
  sqsPurge: (id: string, url: string, confirm_name: string) =>
    api.post<void>(`${acct(id)}/sqs/queues/purge`, { url, confirm_name }),
  sqsRedrive: (id: string, body: SqsRedriveReq) =>
    api.post<{ task_handle: string }>(`${acct(id)}/sqs/queues/redrive`, body),

  // --- EC2 ---
  ec2Instances: (id: string, region?: string, state?: string, q?: string) =>
    api.get<{ instances: Ec2Instance[] }>(`${acct(id)}/ec2/instances${qs({ region, state, q })}`),
  ec2Instance: (id: string, instanceId: string, region?: string) =>
    api.get<Ec2InstanceDetail>(
      `${acct(id)}/ec2/instances/${encodeURIComponent(instanceId)}${qs({ region })}`,
    ),
  ec2Action: (id: string, instanceId: string, action: Ec2Action, region?: string) =>
    api.post<Ec2ActionResp>(
      `${acct(id)}/ec2/instances/${encodeURIComponent(instanceId)}/${action}${qs({ region })}`,
      { confirm_id: instanceId },
    ),

  // --- Athena ---
  athenaWorkgroups: (id: string) =>
    api.get<{ workgroups: AthenaWorkgroup[] }>(`${acct(id)}/athena/workgroups`),
  athenaDatabases: (id: string, catalog = 'AwsDataCatalog') =>
    api.get<{ databases: string[] }>(`${acct(id)}/athena/databases${qs({ catalog })}`),
  athenaTables: (id: string, database: string, catalog = 'AwsDataCatalog') =>
    api.get<{ tables: AthenaTable[] }>(`${acct(id)}/athena/tables${qs({ database, catalog })}`),
  athenaHistory: (id: string, workgroup?: string, max?: number) =>
    api.get<{ executions: AthenaExecution[] }>(
      `${acct(id)}/athena/history${qs({ workgroup, max })}`,
    ),
  athenaQuery: (id: string, body: AthenaQueryReq) =>
    api.post<{ query_execution_id: string }>(`${acct(id)}/athena/query`, body),
  athenaStatus: (id: string, qid: string, token?: string | null, max?: number) =>
    api.get<AthenaQueryStatus>(
      `${acct(id)}/athena/query/${encodeURIComponent(qid)}${qs({ token, max })}`,
    ),
  athenaCancel: (id: string, qid: string) =>
    api.post<void>(`${acct(id)}/athena/query/${encodeURIComponent(qid)}/cancel`, {}),

  // --- EKS ---
  eksClusters: (id: string, region?: string) =>
    api.get<{ clusters: EksClusterSummary[] }>(`${acct(id)}/eks/clusters${qs({ region })}`),
  eksCluster: (id: string, name: string, region?: string) =>
    api.get<EksClusterDetail>(
      `${acct(id)}/eks/clusters/${encodeURIComponent(name)}${qs({ region })}`,
    ),
  eksImport: (id: string, name: string, body: EksImportReq, region?: string) =>
    api.post<EksImportResp>(
      `${acct(id)}/eks/clusters/${encodeURIComponent(name)}/import-kubeconfig${qs({ region })}`,
      body,
    ),

  // --- RDS (read-only) ---
  rdsInstances: (id: string, region?: string, q?: string) =>
    api.get<{ instances: RdsInstance[] }>(`${acct(id)}/rds/instances${qs({ region, q })}`),
  rdsInstance: (id: string, identifier: string, region?: string) =>
    api.get<RdsInstanceDetail>(
      `${acct(id)}/rds/instances/${encodeURIComponent(identifier)}${qs({ region })}`,
    ),

  // --- CloudWatch metrics ---
  /** One `get-metric-data` per call (server-cached 30 s). `instanceType` lets
   *  the daemon drop the CPU-credit series for non-burstable EC2 families. */
  metrics: (
    id: string,
    namespace: MetricsNamespace,
    dimValue: string,
    range: MetricsRange,
    opts: { region?: string; instanceType?: string | null; signal?: AbortSignal } = {},
  ) =>
    api.get<MetricsResp>(
      `${acct(id)}/metrics${qs({
        namespace,
        dim_value: dimValue,
        range,
        region: opts.region,
        instance_type: opts.instanceType,
      })}`,
      opts.signal,
    ),
};

/**
 * Stream an authenticated binary download (`…/s3/…/download`) into a Blob,
 * reporting received bytes so the caller can drive a progress bar. Mirrors
 * `authedBlobUrl`'s auth + error handling but reads the body incrementally
 * (objects can be large) and honours an AbortSignal. Returns the Blob + the
 * server's suggested filename (from `Content-Disposition`, if any).
 */
export async function awsDownloadBlob(
  path: string,
  onProgress?: (received: number, total: number | null) => void,
  signal?: AbortSignal,
): Promise<{ blob: Blob; filename: string | null; contentType: string }> {
  const token = getToken();
  const headers: Record<string, string> = token ? { Authorization: `Bearer ${token}` } : {};
  const resp = await fetch(`${baseUrl()}/api/v1${path}`, { headers, signal });
  if (!resp.ok) {
    let problem: Problem = { code: 'internal', message: resp.statusText };
    try {
      problem = await resp.json();
    } catch {
      // non-JSON error body — keep statusText
    }
    throw new ApiError(resp.status, problem);
  }
  const contentType = resp.headers.get('content-type') ?? 'application/octet-stream';
  const lenHeader = resp.headers.get('content-length');
  const total = lenHeader ? Number(lenHeader) : null;
  const disp = resp.headers.get('content-disposition') ?? '';
  const fnMatch = /filename\*?=(?:UTF-8'')?"?([^";]+)"?/i.exec(disp);
  const filename = fnMatch ? decodeURIComponent(fnMatch[1]) : null;
  if (!resp.body) {
    const blob = await resp.blob();
    onProgress?.(blob.size, blob.size);
    return { blob, filename, contentType };
  }
  const reader = resp.body.getReader();
  const chunks: BlobPart[] = [];
  let received = 0;
  for (;;) {
    const { value, done } = await reader.read();
    if (done) break;
    chunks.push(value);
    received += value.byteLength;
    onProgress?.(received, total);
  }
  return { blob: new Blob(chunks, { type: contentType }), filename, contentType };
}

/** Hand a Blob to the browser as a file download (`<a download>`). Works in the
 *  Tauri WKWebView too (the shell routes the download to ~/Downloads). */
export function saveBlob(blob: Blob, filename: string): void {
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}
