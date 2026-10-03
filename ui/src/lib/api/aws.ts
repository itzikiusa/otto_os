// AWS console API client — thin typed wrappers over the generic `api` helper.
// Mirrors docs/design/aws-k8s-consoles.md §2 (`/aws/*`, crates/otto-aws). Every
// service call takes an account id; `region` overrides the account's default.

import { api, getToken, ApiError, laneFetch } from './client';
import type {
  AthenaExecution,
  AthenaQueryReq,
  AthenaQueryStatus,
  AthenaTable,
  AthenaWorkgroup,
  AwsAccount,
  AwsLogEvent,
  AwsLogEventsQuery,
  AwsLogGroup,
  AwsLogStream,
  AwsLogsInsightsReq,
  AwsLogsInsightsResults,
  AwsPermissions,
  AwsRegionError,
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
  S3DownloadJob,
  S3ListObjectsResp,
  S3ObjectHead,
  S3PresignResp,
  S3PreviewResp,
  S3UploadResp,
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

/** `region` value that fans EC2 / EKS / RDS lists out over every enabled region. */
export const ALL_REGIONS = 'all';

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
  s3Objects: (
    id: string,
    bucket: string,
    prefix = '',
    token?: string | null,
    max?: number,
    signal?: AbortSignal,
  ) =>
    api.get<S3ListObjectsResp>(
      `${acct(id)}/s3/buckets/${encodeURIComponent(bucket)}/objects${qs({ prefix, token, max })}`,
      signal,
    ),
  s3Head: (id: string, bucket: string, key: string) =>
    api.get<S3ObjectHead>(
      `${acct(id)}/s3/buckets/${encodeURIComponent(bucket)}/object${qs({ key })}`,
    ),
  s3Preview: (id: string, bucket: string, key: string, maxBytes?: number) =>
    api.get<S3PreviewResp>(
      `${acct(id)}/s3/buckets/${encodeURIComponent(bucket)}/preview${qs({ key, max_bytes: maxBytes })}`,
    ),
  /** Handler-relative path of the streamed download (feed to `awsDownloadBlob`).
   *  `inline` = in-app image/PDF preview (served inline, capped at 25 MB). */
  s3DownloadPath: (id: string, bucket: string, key: string, inline = false) =>
    `${acct(id)}/s3/buckets/${encodeURIComponent(bucket)}/download${qs({ key, inline: inline ? 'true' : '' })}`,
  /** Presigned GET link (`expiresIn` seconds, ≤ 7 days). Audited. */
  s3Presign: (id: string, bucket: string, key: string, expiresIn?: number) =>
    api.post<S3PresignResp>(`${acct(id)}/s3/buckets/${encodeURIComponent(bucket)}/presign`, {
      key,
      expires_in: expiresIn,
    }),
  /** Delete one object (Edit). Prod accounts need `confirm` === key. */
  s3Delete: (id: string, bucket: string, key: string, confirm?: string) =>
    api.del<void>(
      `${acct(id)}/s3/buckets/${encodeURIComponent(bucket)}/object${qs({ key, confirm })}`,
    ),
  /** Large objects: the daemon writes the object into `localDir` (a directory
   *  on the daemon host) — nothing is buffered in the webview. Poll the job. */
  s3DownloadTo: (id: string, bucket: string, key: string, localDir: string) =>
    api.post<S3DownloadJob>(`${acct(id)}/s3/buckets/${encodeURIComponent(bucket)}/download-to`, {
      key,
      local_dir: localDir,
    }),
  s3DownloadJob: (id: string, job: string, signal?: AbortSignal) =>
    api.get<S3DownloadJob>(`${acct(id)}/s3/download-jobs/${encodeURIComponent(job)}`, signal),
  s3DownloadCancel: (id: string, job: string) =>
    api.post<S3DownloadJob>(`${acct(id)}/s3/download-jobs/${encodeURIComponent(job)}/cancel`, {}),

  // --- SQS ---
  sqsQueues: (id: string, prefix = '', region?: string) =>
    api.get<{ queues: SqsQueue[] }>(`${acct(id)}/sqs/queues${qs({ prefix, region })}`),
  sqsAttributes: (id: string, url: string, region?: string) =>
    api.get<SqsQueueAttributesResp>(`${acct(id)}/sqs/queues/attributes${qs({ url, region })}`),
  sqsPeek: (id: string, body: SqsPeekReq, region?: string) =>
    api.post<{ messages: SqsMessage[] }>(`${acct(id)}/sqs/queues/peek${qs({ region })}`, body),
  sqsSend: (id: string, body: SqsSendReq, region?: string) =>
    api.post<{ message_id: string }>(`${acct(id)}/sqs/queues/send${qs({ region })}`, body),
  sqsDeleteMessage: (id: string, url: string, receipt_handle: string, region?: string) =>
    api.post<void>(`${acct(id)}/sqs/queues/delete-message${qs({ region })}`, {
      url,
      receipt_handle,
    }),
  sqsPurge: (id: string, url: string, confirm_name: string, region?: string) =>
    api.post<void>(`${acct(id)}/sqs/queues/purge${qs({ region })}`, { url, confirm_name }),
  sqsRedrive: (id: string, body: SqsRedriveReq, region?: string) =>
    api.post<{ task_handle: string }>(`${acct(id)}/sqs/queues/redrive${qs({ region })}`, body),

  // --- EC2 ---
  /** `region = ALL_REGIONS` fans out over every enabled region (rows carry
   *  `region`; failed regions come back in `region_errors`). */
  ec2Instances: (id: string, region?: string, state?: string, q?: string) =>
    api.get<{ instances: Ec2Instance[]; region_errors?: AwsRegionError[] }>(
      `${acct(id)}/ec2/instances${qs({ region, state, q })}`,
    ),
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
  athenaWorkgroups: (id: string, region?: string) =>
    api.get<{ workgroups: AthenaWorkgroup[] }>(`${acct(id)}/athena/workgroups${qs({ region })}`),
  athenaDatabases: (id: string, catalog = 'AwsDataCatalog', region?: string) =>
    api.get<{ databases: string[] }>(`${acct(id)}/athena/databases${qs({ catalog, region })}`),
  athenaTables: (id: string, database: string, catalog = 'AwsDataCatalog', region?: string) =>
    api.get<{ tables: AthenaTable[] }>(
      `${acct(id)}/athena/tables${qs({ database, catalog, region })}`,
    ),
  athenaHistory: (id: string, workgroup?: string, max?: number, region?: string) =>
    api.get<{ executions: AthenaExecution[] }>(
      `${acct(id)}/athena/history${qs({ workgroup, max, region })}`,
    ),
  athenaQuery: (id: string, body: AthenaQueryReq, region?: string) =>
    api.post<{ query_execution_id: string }>(`${acct(id)}/athena/query${qs({ region })}`, body),
  athenaStatus: (id: string, qid: string, token?: string | null, max?: number, region?: string) =>
    api.get<AthenaQueryStatus>(
      `${acct(id)}/athena/query/${encodeURIComponent(qid)}${qs({ token, max, region })}`,
    ),
  athenaCancel: (id: string, qid: string, region?: string) =>
    api.post<void>(
      `${acct(id)}/athena/query/${encodeURIComponent(qid)}/cancel${qs({ region })}`,
      {},
    ),

  // --- EKS ---
  eksClusters: (id: string, region?: string) =>
    api.get<{ clusters: EksClusterSummary[]; region_errors?: AwsRegionError[] }>(
      `${acct(id)}/eks/clusters${qs({ region })}`,
    ),
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
    api.get<{ instances: RdsInstance[]; region_errors?: AwsRegionError[] }>(
      `${acct(id)}/rds/instances${qs({ region, q })}`,
    ),
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

  // --- CloudWatch Logs (read-only; account `metrics` operation, Aws:View) ---
  logGroups: (id: string, prefix = '', token?: string | null, region?: string) =>
    api.get<{ groups: AwsLogGroup[]; next_token?: string | null }>(
      `${acct(id)}/logs/groups${qs({ prefix, token, region })}`,
    ),
  logStreams: (id: string, group: string, prefix = '', token?: string | null, region?: string) =>
    api.get<{ streams: AwsLogStream[]; next_token?: string | null }>(
      `${acct(id)}/logs/streams${qs({ group, prefix, token, region })}`,
    ),
  logEvents: (id: string, q: AwsLogEventsQuery, signal?: AbortSignal) =>
    api.get<{ events: AwsLogEvent[]; next_token?: string | null }>(
      `${acct(id)}/logs/events${qs({
        group: q.group,
        streams: q.streams?.length ? q.streams.join(',') : '',
        pattern: q.pattern,
        start: q.start,
        end: q.end,
        token: q.token,
        max: q.max,
        region: q.region,
      })}`,
      signal,
    ),
  logsInsightsStart: (id: string, body: AwsLogsInsightsReq, region?: string) =>
    api.post<{ query_id: string }>(`${acct(id)}/logs/insights${qs({ region })}`, body),
  logsInsightsResults: (id: string, qid: string, region?: string) =>
    api.get<AwsLogsInsightsResults>(
      `${acct(id)}/logs/insights/${encodeURIComponent(qid)}${qs({ region })}`,
    ),
  logsInsightsStop: (id: string, qid: string, region?: string) =>
    api.post<void>(`${acct(id)}/logs/insights/${encodeURIComponent(qid)}/stop${qs({ region })}`, {}),
};

/** Upload a File to `s3://bucket/key` (raw body PUT, Edit-gated). `overwrite`
 *  replaces an existing object; without it the daemon answers 409. */
export async function awsS3Upload(
  id: string,
  bucket: string,
  key: string,
  file: Blob,
  opts: { overwrite?: boolean; region?: string; signal?: AbortSignal } = {},
): Promise<S3UploadResp> {
  const token = getToken();
  const headers: Record<string, string> = {
    'Content-Type': file.type || 'application/octet-stream',
  };
  if (token) headers.Authorization = `Bearer ${token}`;
  const path = `${acct(id)}/s3/buckets/${encodeURIComponent(bucket)}/object${qs({
    key,
    overwrite: opts.overwrite ? 'true' : '',
    region: opts.region,
  })}`;
  const resp = await laneFetch('long', path, {
    method: 'PUT',
    headers,
    body: file,
    signal: opts.signal,
  });
  if (!resp.ok) {
    let problem: Problem = { code: 'internal', message: resp.statusText };
    try {
      problem = await resp.json();
    } catch {
      // non-JSON error body — keep statusText
    }
    throw new ApiError(resp.status, problem);
  }
  return (await resp.json()) as S3UploadResp;
}

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
  // Large objects stream for a long time: the long lane (alias host).
  const resp = await laneFetch('long', path, { headers, signal });
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
