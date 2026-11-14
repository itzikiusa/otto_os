import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { history, undo } from '@codemirror/commands';
import { EditorState } from '@codemirror/state';
import {
  clearIdeSensitiveStorage,
  IDE_CLIPBOARD_HISTORY_KEY,
  IDE_UNDO_DRAFT_INDEX_KEY,
  IDE_UNDO_DRAFT_PREFIX,
  IDE_UNDO_JOURNAL_KEY,
  isSecretIdePath,
  persistIdeUndoJournal,
  readIdeClipboardHistory,
  readIdeUndoDraft,
  recordIdeClipboardEntry,
  restoreIdeStateFromJournal,
  writeIdeUndoDraftImmediate,
} from '../src/lib/components/cm-ide-history.ts';
import {
  mergeBackendIdeProjects,
  normalizeProjectPath,
  parseNamedIdeProject,
} from '../src/lib/ideProjectsModel.ts';
import { diffLineArrays, diffTextLines } from '../src/lib/lineDiff.ts';
import {
  applyLeftHunkToRight,
  computeSideBySideComparison,
} from '../src/modules/ide/history/ideCompareEngine.ts';
import { buildLocalHistoryDiff } from '../src/modules/ide/history/ideLocalHistoryDiff.ts';

class MemoryStorage {
  private map = new Map<string, string>();
  get length(): number {
    return this.map.size;
  }
  key(index: number): string | null {
    return [...this.map.keys()][index] ?? null;
  }
  getItem(k: string): string | null {
    return this.map.get(k) ?? null;
  }
  setItem(k: string, v: string): void {
    this.map.set(k, String(v));
  }
  removeItem(k: string): void {
    this.map.delete(k);
  }
}

describe('cm-ide-history: secret-path filtering, per-path drafts, undo journal & sign-out wipe', () => {
  it('blocks secret/credential files from undo journal, draft storage, and clipboard history (SEC-10)', () => {
    for (const secret of [
      '/repo/.env',
      '/repo/.env.local',
      '/repo/.env.production',
      '/Users/alice/.ssh/id_rsa',
      '/Users/alice/.ssh/id_ed25519',
      '/etc/tls/server.pem',
      '/etc/tls/private.key',
      '/etc/tls/bundle.p12',
      '/repo/config/credentials.json',
    ]) {
      assert.equal(isSecretIdePath(secret), true, `expected secret: ${secret}`);
    }

    for (const safe of ['/repo/src/main.go', '/repo/README.md', '/repo/env.ts', '/repo/keys.go']) {
      assert.equal(isSecretIdePath(safe), false, `expected safe: ${safe}`);
    }

    const store = new MemoryStorage();
    const secretState = EditorState.create({
      doc: 'API_SECRET=hunter2\n',
      extensions: [history()],
    });
    persistIdeUndoJournal('/repo/.env', secretState, 'API_SECRET=old\n', 0, store);
    writeIdeUndoDraftImmediate('/repo/.env', 'API_SECRET=old\n', 'API_SECRET=hunter2\n', store);
    recordIdeClipboardEntry('API_SECRET=hunter2', '/repo/.env', store);

    assert.equal(store.getItem(IDE_UNDO_JOURNAL_KEY), null);
    assert.equal(store.getItem(`${IDE_UNDO_DRAFT_PREFIX}/repo/.env`), null);
    assert.deepEqual(readIdeClipboardHistory(store), []);
  });

  it('persists savedContent + draftContent across reload and wipes all IDE storage on sign-out (DATA-2, PERF-7)', () => {
    const store = new MemoryStorage();
    const filePath = '/repo/app/service.go';
    const diskBaseline = 'package app\n\nconst Version = 1\n';
    const unsavedDraft = 'package app\n\nconst Version = 2\n';

    let st = EditorState.create({ doc: diskBaseline, extensions: [history()] });
    st = st.update({
      changes: { from: diskBaseline.indexOf('1'), to: diskBaseline.indexOf('1') + 1, insert: '2' },
    }).state;

    // Immediate per-file draft write avoids rewriting the full multi-file undo journal
    writeIdeUndoDraftImmediate(filePath, diskBaseline, unsavedDraft, store);
    const fastDraft = readIdeUndoDraft(filePath, store);
    assert.ok(fastDraft);
    assert.equal(fastDraft.savedContent, diskBaseline);
    assert.equal(fastDraft.draftContent, unsavedDraft);

    // Full journal checkpoint stores both savedContent and draftContent + full undo stack
    persistIdeUndoJournal(filePath, st, diskBaseline, 128, store);
    const journalRaw = JSON.parse(store.getItem(IDE_UNDO_JOURNAL_KEY)!) as Record<
      string,
      { savedContent?: string; draftContent?: string; unsaved: boolean }
    >;
    assert.equal(journalRaw[filePath].savedContent, diskBaseline);
    assert.equal(journalRaw[filePath].draftContent, unsavedDraft);
    assert.equal(journalRaw[filePath].unsaved, true);

    const restored = restoreIdeStateFromJournal(filePath, diskBaseline, [history()], store);
    assert.ok(restored);
    assert.equal(restored.restoredDraft, unsavedDraft);
    assert.equal(restored.scrollTop, 128);
    assert.equal(restored.state.doc.toString(), unsavedDraft);

    // Undo on the restored state steps cleanly back to diskBaseline!
    let undoneState = restored.state;
    const didUndo = undo({
      state: undoneState,
      dispatch: (tr) => {
        undoneState = tr.state;
      },
    });
    assert.equal(didUndo, true);
    assert.equal(undoneState.doc.toString(), diskBaseline);

    // Sign-out cleanup wipes undo journal, per-path drafts, draft index, and clipboard history
    recordIdeClipboardEntry('const Version = 2', filePath, store);
    assert.equal(readIdeClipboardHistory(store).length, 1);
    clearIdeSensitiveStorage(store);
    assert.equal(store.getItem(IDE_UNDO_JOURNAL_KEY), null);
    assert.equal(store.getItem(IDE_UNDO_DRAFT_INDEX_KEY), null);
    assert.equal(store.getItem(`${IDE_UNDO_DRAFT_PREFIX}${filePath}`), null);
    assert.deepEqual(readIdeClipboardHistory(store), []);
  });
});

describe('lineDiff, ideCompareEngine & ideLocalHistoryDiff parity (DUP-7)', () => {
  it('computes identical edit counts across lineDiff, Side-by-Side compare, and Local History diff', () => {
    const left = ['header', 'alpha', 'shared-anchor', 'old-1', 'old-2', 'footer'].join('\n') + '\n';
    const right = ['header', 'alpha-v2', 'inserted', 'shared-anchor', 'new-1', 'footer'].join('\n') + '\n';

    const ops = diffTextLines(left, right);
    const addedCount = ops.filter((o) => o.op === 'insert').length;
    const deletedCount = ops.filter((o) => o.op === 'delete').length;
    assert.equal(addedCount, 3);
    assert.equal(deletedCount, 3);

    const sbs = computeSideBySideComparison(left, right);
    assert.equal(sbs.identical, false);
    assert.equal(sbs.hunks.length, 2);
    assert.equal(sbs.hunks[0].kind, 'modify');
    assert.equal(sbs.hunks[1].kind, 'modify');

    // Reverting both hunks from left -> right yields exact textual equality
    const afterFirst = applyLeftHunkToRight(left, right, 1);
    const sbs2 = computeSideBySideComparison(left, afterFirst);
    assert.equal(sbs2.hunks.length, 1);
    const fullyReverted = applyLeftHunkToRight(left, afterFirst, 0);
    assert.equal(fullyReverted, left);

    // Local history diff (currentContent = right, snapshotContent = left) agrees on +/- counts
    const lhDiff = buildLocalHistoryDiff('notes/doc.txt', left, right, true);
    assert.equal(lhDiff.total_added, deletedCount);
    assert.equal(lhDiff.total_deleted, addedCount);

    // Deleted file snapshot renders every line as recoverable addition
    const deletedDiff = buildLocalHistoryDiff('notes/doc.txt', left, null, false);
    assert.equal(deletedDiff.files[0].status, 'added');
    assert.equal(deletedDiff.total_added, 6);
    assert.equal(deletedDiff.total_deleted, 0);
  });

  it('handles large patience-anchored inputs beyond the exact DP cell threshold', () => {
    const oldArr: string[] = [];
    const newArr: string[] = [];
    for (let i = 0; i < 1400; i++) {
      const line = `unique-anchor-${i}`;
      oldArr.push(line);
      newArr.push(i === 700 ? 'mutated-middle-700' : line);
    }
    // Mutate first & last lines after prefix/suffix so mid slice exceeds 1.6M cells (1400*1400 = 1.96M)
    oldArr[0] = 'old-head';
    newArr[0] = 'new-head';
    oldArr[1399] = 'old-tail';
    newArr[1399] = 'new-tail';

    const ops = diffLineArrays(oldArr, newArr);
    const del = ops.filter((o) => o.op === 'delete').map((o) => o.line);
    const ins = ops.filter((o) => o.op === 'insert').map((o) => o.line);
    assert.deepEqual(del, ['old-head', 'unique-anchor-700', 'old-tail']);
    assert.deepEqual(ins, ['new-head', 'mutated-middle-700', 'new-tail']);
  });
});

describe('ideProjectsModel: canonical path, RFC3339 timestamps & pinned/empty reconciliation (DATA-5)', () => {
  it('normalizes macOS /private symlinks, RFC3339 timestamps, explicit pinned:false & empty remote list', () => {
    assert.equal(normalizeProjectPath('/private/tmp/otto-ws///'), '/tmp/otto-ws');
    assert.equal(normalizeProjectPath('/private/var/folders/ab/cd/'), '/var/folders/ab/cd');

    const parsed = parseNamedIdeProject({
      id: 'proj-1',
      name: 'Backend',
      primary_root: '/private/tmp/backend/',
      folders: [{ path: '/private/tmp/backend' }, { path: '/private/tmp/shared/' }],
      pinned: false,
      updated_at: '2026-11-14T12:00:00Z',
    });
    assert.ok(parsed);
    assert.equal(parsed.primaryRoot, '/tmp/backend');
    assert.deepEqual(parsed.folders, ['/tmp/backend', '/tmp/shared']);
    assert.equal(parsed.pinned, false);
    assert.equal(parsed.updatedAt, Date.parse('2026-11-14T12:00:00Z'));

    const local = [parsed];
    const merged = mergeBackendIdeProjects(
      local,
      [
        {
          id: 'proj-1',
          name: 'Backend Renamed',
          primary_root: '/tmp/backend',
          folders: ['/tmp/backend', '/tmp/shared'],
          updated_at: '2026-11-14T13:00:00Z',
        },
      ],
      true,
    );
    assert.equal(merged.length, 1);
    assert.equal(merged[0].name, 'Backend Renamed');
    // Preserves local explicit `pinned: false` when remote row omits `pinned`
    assert.equal(merged[0].pinned, false);
    assert.equal(merged[0].updatedAt, Date.parse('2026-11-14T13:00:00Z'));

    // Authoritative empty server list clears stale deleted local projects
    assert.deepEqual(mergeBackendIdeProjects(merged, [], true), []);
    assert.equal(mergeBackendIdeProjects(merged, [], false).length, 1);
  });
});
