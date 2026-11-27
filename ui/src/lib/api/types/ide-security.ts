export type SecurityCheck = 'vulnerability' | 'secret' | 'misconfiguration' | 'license';
export type SecuritySeverity = 'critical' | 'high' | 'medium' | 'low' | 'unknown';
export type SecurityScanStatus = 'queued' | 'running' | 'completed' | 'failed' | 'cancelled';

export interface ScanSettings {
  checks: SecurityCheck[];
  excludes: string[];
  parallelism: number;
  memory_mib: number;
  timeout_secs: number;
}

export interface SecurityFinding {
  fingerprint: string;
  engine: string;
  check: SecurityCheck;
  severity: SecuritySeverity;
  root: string;
  path: string;
  line: number | null;
  rule: string;
  title: string;
  description: string;
  fix: string;
}

export interface SecurityEngineVersion {
  version: string;
  updated_at: string;
  checked_at: string;
}

export interface SecurityScanSummary {
  id: string;
  status: SecurityScanStatus;
  created_at: string;
  started_at: string | null;
  finished_at: string | null;
  engine: SecurityEngineVersion | null;
  metrics: { wall_ms: number; peak_cpu_percent: number; peak_rss_bytes: number } | null;
  error: string | null;
  count: number;
}

export interface SecurityScanDetail {
  scan: SecurityScanSummary;
  findings: SecurityFinding[];
}

export interface ProjectScans {
  settings: ScanSettings;
  engine: SecurityEngineVersion | null;
  scans: SecurityScanSummary[];
}
