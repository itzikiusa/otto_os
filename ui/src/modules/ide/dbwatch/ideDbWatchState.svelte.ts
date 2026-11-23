// Reactive state controller for the IDE Live DB Watch & Component-Test Watcher panel.

import { api } from '../../../lib/api/client';
import { loadErrorText } from '../../../lib/loadError';
import { onIdeTargetReset } from '../store/ideStateSlices.svelte';
import type {
  IdeDbEngine,
  IdeDbWatchDetectReq,
  IdeDbWatchDetectResp,
  IdeDbWatchSnapshotItem,
  IdeDbWatchSnapshotReq,
  IdeDbWatchSnapshotResp,
  IdeDbWatchTarget,
  IdeDetectedDbAccess,
  IdeEphemeralDbTarget,
  IdeSavedDbTarget,
} from '../../../lib/api/types/ide-db-watch';
import {
  formatDbCellValue,
  IDE_DB_ENGINES,
  pickPreferredDbTarget,
  targetKeyOf,
} from './ideDbWatchModel';

export { formatDbCellValue, IDE_DB_ENGINES, pickPreferredDbTarget, targetKeyOf };

class IdeDbWatchController {
  detectedObjects = $state<IdeDetectedDbAccess[]>([]);
  savedConnections = $state<IdeSavedDbTarget[]>([]);
  ephemeralTargets = $state<IdeEphemeralDbTarget[]>([]);
  detectLoading = $state(false);
  detectError = $state<string | null>(null);

  selectedTargetKey = $state('');
  selectedEngine = $state<IdeDbEngine>('mysql');
  selectedDatabase = $state('');
  selectedObjectName = $state('');
  customQuery = $state('');
  limit = $state(100);

  baselineSnapshot = $state<IdeDbWatchSnapshotItem | null>(null);
  latestResp = $state<IdeDbWatchSnapshotResp | null>(null);
  snapshotLoading = $state(false);
  snapshotError = $state<string | null>(null);

  autoRefreshOnExec = $state(true);
  liveWatch = $state(false);
  viewMode = $state<'changes' | 'rows'>('changes');

  private gen = 0;
  private detectSeq = 0;
  private snapshotSeq = 0;

  constructor() {
    onIdeTargetReset(() => this.reset());
  }

  reset(): void {
    this.gen += 1;
    this.detectSeq += 1;
    this.snapshotSeq += 1;
    this.detectedObjects = [];
    this.ephemeralTargets = [];
    this.detectLoading = false;
    this.detectError = null;
    this.selectedTargetKey = '';
    this.selectedDatabase = '';
    this.selectedObjectName = '';
    this.customQuery = '';
    this.baselineSnapshot = null;
    this.latestResp = null;
    this.snapshotLoading = false;
    this.snapshotError = null;
    this.liveWatch = false;
  }

  get allTargets(): IdeDbWatchTarget[] {
    return [...this.ephemeralTargets, ...this.savedConnections];
  }

  get selectedTarget(): IdeDbWatchTarget | null {
    return this.allTargets.find((t) => targetKeyOf(t) === this.selectedTargetKey) ?? null;
  }

  get canCapture(): boolean {
    return Boolean(
      this.selectedTarget && (this.selectedObjectName.trim() || this.customQuery.trim()),
    );
  }

  get changeBadgeCount(): number | undefined {
    const d = this.latestResp?.diff;
    if (!d) return undefined;
    const total = d.added_count + d.modified_count + d.removed_count;
    return total > 0 ? total : undefined;
  }

  selectDetectedObject(obj: IdeDetectedDbAccess): void {
    const changedObject =
      this.selectedEngine !== obj.engine || this.selectedObjectName !== obj.object_name;
    this.selectedEngine = obj.engine;
    this.selectedObjectName = obj.object_name;
    if (obj.database_hint) {
      this.selectedDatabase = obj.database_hint;
    }
    const picked = pickPreferredDbTarget(
      obj.engine,
      this.savedConnections,
      this.ephemeralTargets,
      this.selectedTargetKey,
    );
    if (picked) {
      this.selectedTargetKey = targetKeyOf(picked);
      if (!this.selectedDatabase && picked.database) {
        this.selectedDatabase = picked.database;
      }
    }
    if (changedObject) {
      this.baselineSnapshot = null;
      this.latestResp = null;
      this.snapshotError = null;
    }
  }

  selectTargetByKey(key: string): void {
    if (this.selectedTargetKey === key) return;
    this.selectedTargetKey = key;
    const target = this.selectedTarget;
    if (target) {
      this.selectedEngine = target.engine;
      if (!this.selectedDatabase && target.database) {
        this.selectedDatabase = target.database;
      }
    }
    this.baselineSnapshot = null;
    this.latestResp = null;
    this.snapshotError = null;
  }

  async detect(opts: {
    root: string;
    activeFile?: string | null;
    connectionId?: string | null;
    additionalRoots?: string[];
  }): Promise<void> {
    if (!opts.root) return;
    const gen = this.gen;
    const seq = ++this.detectSeq;
    this.detectLoading = true;
    this.detectError = null;
    try {
      const req: IdeDbWatchDetectReq = {
        root: opts.root,
        connection_id: opts.connectionId ?? null,
        active_file: opts.activeFile ?? null,
        additional_roots: opts.additionalRoots ?? [],
      };
      const resp = await api.post<IdeDbWatchDetectResp>('/ide/db-watch/detect', req);
      if (gen !== this.gen || seq !== this.detectSeq) return;
      this.detectedObjects = resp.detected_objects ?? [];
      this.savedConnections = resp.saved_connections ?? [];
      this.ephemeralTargets = resp.ephemeral_targets ?? [];

      if (!this.selectedObjectName && this.detectedObjects.length > 0) {
        this.selectDetectedObject(this.detectedObjects[0]);
      } else {
        const picked = pickPreferredDbTarget(
          this.selectedEngine,
          this.savedConnections,
          this.ephemeralTargets,
          this.selectedTargetKey,
        );
        if (picked) {
          this.selectedTargetKey = targetKeyOf(picked);
          if (!this.selectedDatabase && picked.database) {
            this.selectedDatabase = picked.database;
          }
        } else {
          this.selectedTargetKey = '';
        }
      }
    } catch (e) {
      if (gen !== this.gen || seq !== this.detectSeq) return;
      this.detectError = loadErrorText(e);
    } finally {
      if (gen === this.gen && seq === this.detectSeq) this.detectLoading = false;
    }
  }

  async captureSnapshot(root: string, mode: 'baseline' | 'diff' = 'diff'): Promise<boolean> {
    const target = this.selectedTarget;
    const objectName = this.selectedObjectName.trim();
    const custom = this.customQuery.trim();
    if (!root || !target || (!objectName && !custom)) return false;

    const gen = this.gen;
    const seq = ++this.snapshotSeq;
    this.snapshotLoading = true;
    this.snapshotError = null;
    try {
      const req: IdeDbWatchSnapshotReq = {
        root,
        target_kind: target.target_kind,
        connection_id: target.target_kind === 'saved' ? target.connection_id : null,
        ephemeral_id: target.target_kind === 'ephemeral' ? target.ephemeral_id : null,
        engine: this.selectedEngine,
        database: this.selectedDatabase.trim() || target.database || null,
        object_name: objectName || 'custom',
        custom_query: custom || null,
        limit: Math.max(1, Math.min(500, this.limit || 100)),
        previous: mode === 'diff' ? this.baselineSnapshot : null,
      };
      const resp = await api.post<IdeDbWatchSnapshotResp>('/ide/db-watch/snapshot', req);
      if (gen !== this.gen || seq !== this.snapshotSeq) return false;
      this.latestResp = resp;
      if (mode === 'baseline' || !this.baselineSnapshot) {
        this.baselineSnapshot = resp.snapshot;
      }
      return true;
    } catch (e) {
      if (gen !== this.gen || seq !== this.snapshotSeq) return false;
      this.snapshotError = loadErrorText(e);
      return false;
    } finally {
      if (gen === this.gen && seq === this.snapshotSeq) this.snapshotLoading = false;
    }
  }

  promoteLatestToBaseline(): void {
    if (!this.latestResp) return;
    this.baselineSnapshot = this.latestResp.snapshot;
    this.latestResp = { ...this.latestResp, diff: null };
  }
}

export const ideDbWatch = new IdeDbWatchController();
