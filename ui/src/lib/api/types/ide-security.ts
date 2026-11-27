export type SecurityCheck = 'vulnerability' | 'secret' | 'misconfiguration' | 'license' | 'sast';
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

export interface SecurityEngineRun {
  name: string;
  version: SecurityEngineVersion;
  metrics: { wall_ms: number; peak_cpu_percent: number; peak_rss_bytes: number };
  mode: 'cached' | 'incremental' | 'full';
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
  engines: SecurityEngineRun[];
}

export interface SecurityScanDetail {
  scan: SecurityScanSummary;
  findings: SecurityFinding[];
  annotations: {
    existing: string[];
    baseline: string[];
    suppressions: Record<string, string>;
  };
}

export interface ProjectScans {
  settings: ScanSettings;
  engine: SecurityEngineVersion | null;
  scans: SecurityScanSummary[];
  opengrep: SecurityEngineVersion | null;
  schedule: SecuritySchedule | null;
  baseline: string | null;
}

export interface SecurityScheduleInput {
  cron: string;
  timezone: string;
}

export interface SecuritySchedule extends SecurityScheduleInput {
  next_runs: string[];
}

export type SecurityManageAction =
  | { action: 'preview_schedule'; schedule: SecurityScheduleInput }
  | { action: 'set_schedule'; schedule: SecurityScheduleInput | null }
  | { action: 'set_suppression'; fingerprint: string; reason: string | null }
  | { action: 'set_baseline'; scan_id: string | null }
  | { action: 'import_sarif'; sarif: unknown };
