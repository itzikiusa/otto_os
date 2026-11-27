import { api } from '../../../lib/api/client';
import type { ProjectScans, SecurityScanDetail, SecurityScanSummary } from '../../../lib/api/types';
import type { IdeDiagnostic } from '../store/ideTypes';
import { findingPath } from './projectSecurity.svelte';

export const projectSecurityPath = (id: string) => `/ide/projects/${encodeURIComponent(id)}/security`;

export function diagnostics(detail: SecurityScanDetail): IdeDiagnostic[] {
  return detail.findings.flatMap((finding) => {
    if (!finding.line || detail.annotations.suppressions[finding.fingerprint]) return [];
    const severity = finding.severity === 'critical' || finding.severity === 'high'
      ? 'error' : finding.severity === 'medium' ? 'warning' : 'info';
    return [{ path: findingPath(finding), line: finding.line, severity,
      message: finding.description, source: finding.engine, code: finding.rule } satisfies IdeDiagnostic];
  });
}

export function latestFileScan(scans: SecurityScanSummary[]): SecurityScanSummary | undefined {
  return scans.find((scan) => scan.status === 'completed'
    && (scan.engines.length === 0 || scan.engines.some((engine) => engine.name !== 'otto-passive-http')));
}

export function scanDetailChanged(previous: SecurityScanSummary | undefined, next: SecurityScanSummary | undefined): boolean {
  return previous?.id !== next?.id || previous?.status !== next?.status || previous?.finished_at !== next?.finished_at;
}

export async function latestSecurityDiagnostics(id: string, signal: AbortSignal): Promise<IdeDiagnostic[]> {
  const base = projectSecurityPath(id);
  const overview = await api.bg.get<ProjectScans>(base, signal);
  const scan = latestFileScan(overview.scans);
  if (!scan) return [];
  return diagnostics(await api.bg.get<SecurityScanDetail>(`${base}/scans/${encodeURIComponent(scan.id)}`, signal));
}
