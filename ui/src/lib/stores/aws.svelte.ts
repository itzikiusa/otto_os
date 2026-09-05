// AWS console store: CLI install status, accounts (+ per-account permission
// probes), the active "Sign in" PTY sheet, and per-account/service list caches
// so switching services in the rail is instant. Like the scheduled-tasks store
// it does NOT import events.svelte.ts — the dispatcher calls `aws.applyEvent`.
//
// Svelte 5 gotchas honoured here (see sftp.svelte.ts): readers like `account()`
// / `perms()` never mutate `$state` (templates read them inside `$derived`);
// creation/mutation lives in loaders called from effects/handlers.

import { resourceAccess, type ResourceAccessChange } from './resource-access.svelte';
import { awsApi } from '../api/aws';
import { ApiError } from '../api/client';
import type {
  AthenaTable,
  AthenaWorkgroup,
  AwsAccount,
  AwsPermissions,
  AwsRegion,
  AwsService,
  AwsStatus,
  EksClusterSummary,
  Ec2Instance,
  Id,
  OttoEvent,
  RdsInstance,
  S3Bucket,
  SqsQueue,
  SqsQueueAttributesResp,
  UpsertAwsAccountReq,
} from '../api/types';

/** Athena catalog for one account (databases → tables → columns). */
export interface AthenaCatalog {
  workgroups: AthenaWorkgroup[];
  databases: string[];
  /** database → tables (loaded on expand). */
  tables: Record<string, AthenaTable[]>;
}

export const AWS_SERVICES: { id: AwsService; label: string; icon: string }[] = [
  { id: 's3', label: 'S3', icon: 'archive' },
  { id: 'sqs', label: 'SQS', icon: 'send' },
  { id: 'ec2', label: 'EC2', icon: 'box' },
  { id: 'athena', label: 'Athena', icon: 'db' },
  { id: 'eks', label: 'EKS', icon: 'helm' },
  { id: 'rds', label: 'RDS', icon: 'db' },
];

const INSTALL_POLL_MS = 1500;

class AwsStore {
  accessRevision = $state(0);
  onAccessChange(change: ResourceAccessChange): void {
    if (
      change.type === 'decision' &&
      (change.kind !== 'aws_account' ||
        !change.before ||
        !Object.keys(change.before.operations).some(
          (op) => change.before?.operations[op]?.allowed && !change.after?.operations[op]?.allowed,
        ))
    )
      return;
    this.accessRevision++;
    if (change.type === 'decision') {
      this.forgetAccount(change.id);
      this.accounts = this.accounts.filter((a) => a.id !== change.id);
      // Queue attributes are keyed by URL, so their account cannot be proven.
      this.sqsAttrs = {};
    } else {
      this.accounts = [];
      this.permissions = {};
      this.s3Buckets = {};
      this.sqsQueues = {};
      this.sqsAttrs = {};
      this.ec2 = {};
      this.eks = {};
      this.rds = {};
      this.athena = {};
      this.login = null;
    }
    this.accountsLoaded = false;
    this.accountsLoading = false;
    this.permLoading = {};
  }

  status: AwsStatus | null = $state(null);
  /** Set when `/aws/status` itself fails (404 = daemon predates the module). */
  statusError = $state('');
  statusLoaded = $state(false);
  installBusy = $state(false);

  accounts: AwsAccount[] = $state([]);
  accountsLoading = $state(false);
  accountsLoaded = $state(false);
  accountsError = $state('');

  regions: AwsRegion[] = $state([]);

  /** account id → last permission probe (seeded from the account row). */
  permissions: Record<Id, AwsPermissions> = $state({});
  permLoading: Record<Id, boolean> = $state({});

  /** The open "Sign in" sheet — the `aws sso login` PTY session for an account. */
  login: { accountId: Id; sessionId: Id } | null = $state(null);

  // --- per-service caches (keyed by account id, or `${account}:${region}`) ---
  s3Buckets: Record<Id, S3Bucket[]> = $state({});
  sqsQueues: Record<Id, SqsQueue[]> = $state({});
  /** queue url → attributes (approx counts). */
  sqsAttrs: Record<string, SqsQueueAttributesResp> = $state({});
  ec2: Record<string, Ec2Instance[]> = $state({});
  eks: Record<string, EksClusterSummary[]> = $state({});
  rds: Record<string, RdsInstance[]> = $state({});
  athena: Record<Id, AthenaCatalog> = $state({});

  private installTimer: ReturnType<typeof setTimeout> | null = null;

  // ── readers (pure) ───────────────────────────────────────────────────────

  account(id: Id | null | undefined): AwsAccount | null {
    if (!id) return null;
    return this.accounts.find((a) => a.id === id) ?? null;
  }

  perms(id: Id): AwsPermissions | null {
    return this.permissions[id] ?? this.account(id)?.permissions ?? null;
  }

  /** Whether an account's own IAM permits a service (`unknown` → optimistic). */
  serviceAllowed(id: Id, svc: AwsService): boolean {
    const p = this.perms(id);
    if (!p) return true;
    return p.services[svc] !== 'denied';
  }

  get installed(): boolean {
    return this.status?.installed === true;
  }

  get installing(): boolean {
    return this.status?.install.state === 'running';
  }

  // ── status / install ─────────────────────────────────────────────────────

  async loadStatus(): Promise<void> {
    try {
      this.status = await awsApi.status();
      this.statusError = '';
    } catch (e) {
      this.statusError =
        e instanceof ApiError && e.status === 404
          ? 'The running daemon has no AWS console routes (update Otto).'
          : e instanceof Error
            ? e.message
            : String(e);
    } finally {
      this.statusLoaded = true;
    }
    this.scheduleInstallPoll();
  }

  /** Kick off the CLI install and poll `/status` every 1.5 s until it settles. */
  async startInstall(): Promise<void> {
    this.installBusy = true;
    try {
      const job = await awsApi.install();
      if (this.status) this.status = { ...this.status, install: job };
    } finally {
      this.installBusy = false;
    }
    await this.loadStatus();
  }

  private scheduleInstallPoll(): void {
    if (this.installTimer) {
      clearTimeout(this.installTimer);
      this.installTimer = null;
    }
    if (!this.installing) return;
    this.installTimer = setTimeout(() => {
      this.installTimer = null;
      void this.loadStatus();
    }, INSTALL_POLL_MS);
  }

  // ── accounts ─────────────────────────────────────────────────────────────

  async loadAccounts(): Promise<void> {
    const revision=this.accessRevision;
    this.accountsLoading = true;
    try {
      const accounts = await awsApi.listAccounts();
      if(revision!==this.accessRevision)return;
      this.accounts=accounts;
      this.accountsError = '';
      // Seed the permission cache from the rows (server caches 10 min).
      const next = { ...this.permissions };
      for (const a of this.accounts) if (a.permissions && !next[a.id]) next[a.id] = a.permissions;
      this.permissions = next;
    } catch (e) {
      this.accountsError = e instanceof Error ? e.message : String(e);
    } finally {
      this.accountsLoading = false;
      this.accountsLoaded = true;
    }
  }

  async loadRegions(): Promise<void> {
    if (this.regions.length) return;
    try {
      this.regions = (await awsApi.regions()).regions;
    } catch {
      this.regions = [];
    }
  }

  async loadPermissions(id: Id, refresh = false): Promise<AwsPermissions | null> {
    const revision=this.accessRevision;
    if (this.permLoading[id]) return this.permissions[id] ?? null;
    this.permLoading = { ...this.permLoading, [id]: true };
    try {
      const p = await awsApi.permissions(id, refresh);
    if(revision!==this.accessRevision)return null;
      this.permissions = { ...this.permissions, [id]: p };
      return p;
    } catch {
      return this.permissions[id] ?? null;
    } finally {
      this.permLoading = { ...this.permLoading, [id]: false };
    }
  }

  async createAccount(body: UpsertAwsAccountReq): Promise<AwsAccount> {
    const a = await awsApi.createAccount(body);
    await this.loadAccounts();
    return a;
  }

  async updateAccount(id: Id, body: Partial<UpsertAwsAccountReq>): Promise<AwsAccount> {
    const a = await awsApi.updateAccount(id, body);
    await this.loadAccounts();
    return a;
  }

  async deleteAccount(id: Id): Promise<void> {
    await awsApi.deleteAccount(id);
    this.forgetAccount(id);
    await this.loadAccounts();
  }

  private forgetAccount(id: Id): void {
    const drop = <T>(rec: Record<string, T>): Record<string, T> =>
      Object.fromEntries(Object.entries(rec).filter(([k]) => k !== id && !k.startsWith(`${id}:`)));
    this.permissions = drop(this.permissions);
    this.s3Buckets = drop(this.s3Buckets);
    this.sqsQueues = drop(this.sqsQueues);
    this.ec2 = drop(this.ec2);
    this.eks = drop(this.eks);
    this.rds = drop(this.rds);
    this.athena = drop(this.athena);
    if (this.login?.accountId === id) this.login = null;
  }

  // ── sign in (aws sso login PTY) ──────────────────────────────────────────

  async beginLogin(accountId: Id, workspaceId: Id): Promise<void> {
    const revision=this.accessRevision;
    const s = await awsApi.login(accountId, workspaceId);
    if(revision!==this.accessRevision)return ;
    this.login = { accountId, sessionId: s.id };
  }

  endLogin(): void {
    this.login = null;
  }

  // ── service caches ───────────────────────────────────────────────────────

  async loadS3Buckets(id: Id): Promise<S3Bucket[]> {
    const revision=this.accessRevision;
    const r = await awsApi.s3Buckets(id);
    if(revision!==this.accessRevision)return [];
    this.s3Buckets = { ...this.s3Buckets, [id]: r.buckets };
    return r.buckets;
  }

  async loadSqsQueues(id: Id, prefix = ''): Promise<SqsQueue[]> {
    const revision=this.accessRevision;
    const r = await awsApi.sqsQueues(id, prefix);
    if(revision!==this.accessRevision)return [];
    this.sqsQueues = { ...this.sqsQueues, [id]: r.queues };
    return r.queues;
  }

  async loadSqsAttrs(id: Id, url: string): Promise<SqsQueueAttributesResp | null> {
    const revision=this.accessRevision;
    try {
      const a = await awsApi.sqsAttributes(id, url);
    if(revision!==this.accessRevision)return null;
      this.sqsAttrs = { ...this.sqsAttrs, [url]: a };
      return a;
    } catch {
      return null;
    }
  }

  async loadEc2(id: Id, region: string, state?: string, q?: string): Promise<Ec2Instance[]> {
    const revision=this.accessRevision;
    const r = await awsApi.ec2Instances(id, region || undefined, state, q);
    if(revision!==this.accessRevision)return [];
    this.ec2 = { ...this.ec2, [`${id}:${region}`]: r.instances };
    return r.instances;
  }

  async loadEks(id: Id, region: string): Promise<EksClusterSummary[]> {
    const revision=this.accessRevision;
    const r = await awsApi.eksClusters(id, region || undefined);
    if(revision!==this.accessRevision)return [];
    this.eks = { ...this.eks, [`${id}:${region}`]: r.clusters };
    return r.clusters;
  }

  async loadRds(id: Id, region: string): Promise<RdsInstance[]> {
    const revision=this.accessRevision;
    const r = await awsApi.rdsInstances(id, region || undefined);
    if(revision!==this.accessRevision)return [];
    this.rds = { ...this.rds, [`${id}:${region}`]: r.instances };
    return r.instances;
  }

  async loadAthenaCatalog(id: Id): Promise<AthenaCatalog> {
    const revision=this.accessRevision;
    const [wg, dbs] = await Promise.all([awsApi.athenaWorkgroups(id), awsApi.athenaDatabases(id)]);
    if(revision!==this.accessRevision)return {workgroups:[],databases:[],tables:{}};
    const cat: AthenaCatalog = {
      workgroups: wg.workgroups,
      databases: dbs.databases,
      tables: this.athena[id]?.tables ?? {},
    };
    this.athena = { ...this.athena, [id]: cat };
    return cat;
  }

  async loadAthenaTables(id: Id, database: string): Promise<AthenaTable[]> {
    const revision=this.accessRevision;
    const r = await awsApi.athenaTables(id, database);
    if(revision!==this.accessRevision)return [];
    const cur = this.athena[id] ?? { workgroups: [], databases: [], tables: {} };
    this.athena = {
      ...this.athena,
      [id]: { ...cur, tables: { ...cur.tables, [database]: r.tables } },
    };
    return r.tables;
  }

  // ── live events ──────────────────────────────────────────────────────────

  applyEvent(
    ev: Extract<OttoEvent, { type: 'aws_account_updated' | 'aws_install_updated' }>,
  ): void {
    if (ev.type === 'aws_account_updated') {
      if (ev.deleted) this.forgetAccount(ev.account_id);
      void this.loadAccounts();
    } else if (ev.tool === 'aws') {
      void this.loadStatus();
    }
  }
}

export const aws = new AwsStore();
resourceAccess.subscribe(change=>aws.onAccessChange(change));
