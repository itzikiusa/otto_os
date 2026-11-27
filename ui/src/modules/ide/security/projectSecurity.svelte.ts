import { api } from '../../../lib/api/client';
import type { ProjectScans, SecurityFinding, SecurityScanDetail } from '../../../lib/api/types';
import type { IdeDiagnostic } from '../store/ideTypes';

export const projectSecurity = $state<{ problems: IdeDiagnostic[] }>({ problems: [] });

export const projectSecurityPath = (id: string) => `/ide/projects/${encodeURIComponent(id)}/security`;

export function findingPath(finding: SecurityFinding): string {
  return `${finding.root}/${finding.path}`;
}

export function diagnostics(detail: SecurityScanDetail): IdeDiagnostic[] {
  return detail.findings.flatMap((finding) => {
    if (!finding.line || detail.annotations.suppressions[finding.fingerprint]) return [];
    const severity = finding.severity === 'critical' || finding.severity === 'high'
      ? 'error' : finding.severity === 'medium' ? 'warning' : 'info';
    return [{ path: findingPath(finding), line: finding.line, severity,
      message: finding.description, source: finding.engine, code: finding.rule } satisfies IdeDiagnostic];
  });
}

export async function latestSecurityDiagnostics(id: string, signal: AbortSignal): Promise<IdeDiagnostic[]> {
  const base = projectSecurityPath(id);
  const overview = await api.bg.get<ProjectScans>(base, signal);
  const scan = overview.scans.find((row) => row.status === 'completed');
  if (!scan) return [];
  return diagnostics(await api.bg.get<SecurityScanDetail>(`${base}/scans/${encodeURIComponent(scan.id)}`, signal));
}
