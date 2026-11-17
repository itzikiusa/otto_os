import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import {
  parseCsvRecords,
  resolveCsvDelimiter,
  splitCsvRecords,
} from '../src/lib/components/cm-ide-csv.ts';
import {
  DEFAULT_IDE_PREFERENCES,
  IDE_PREFS_STORAGE_KEY,
  readIdePreferences,
  resolveEffectiveIdeScheme,
  writeIdePreferences,
} from '../src/lib/components/cm-ide-prefs.ts';
import {
  formatDbCellValue,
  IDE_DB_ENGINES,
  pickPreferredDbTarget,
  targetKeyOf,
} from '../src/modules/ide/dbwatch/ideDbWatchModel.ts';
import type {
  IdeEphemeralDbTarget,
  IdeSavedDbTarget,
} from '../src/lib/api/types/ide-db-watch.ts';

class MemoryStorage {
  private map = new Map<string, string>();

  get length(): number {
    return this.map.size;
  }

  clear(): void {
    this.map.clear();
  }

  getItem(key: string): string | null {
    return this.map.has(key) ? (this.map.get(key) ?? null) : null;
  }

  key(index: number): string | null {
    return Array.from(this.map.keys())[index] ?? null;
  }

  removeItem(key: string): void {
    this.map.delete(key);
  }

  setItem(key: string, value: string): void {
    this.map.set(key, String(value));
  }
}

function withMockStorage<T>(fn: (storage: MemoryStorage) => T): T {
  const prev = (globalThis as Record<string, unknown>).localStorage;
  const storage = new MemoryStorage();
  (globalThis as Record<string, unknown>).localStorage = storage;
  try {
    return fn(storage);
  } finally {
    if (prev === undefined) {
      delete (globalThis as Record<string, unknown>).localStorage;
    } else {
      (globalThis as Record<string, unknown>).localStorage = prev;
    }
  }
}

describe('RFC 4180 CSV/TSV quote-aware record splitting', () => {
  it('keeps multiline quoted LF and CRLF cells intact with escaped double quotes', () => {
    const csv = [
      'id,notes,amount',
      '1,"First line\nSecond ""quoted"" line",42',
      '2,"Alpha\r\nBeta",99',
      '',
    ].join('\r\n');

    const raw = splitCsvRecords(csv, ',');
    assert.equal(raw.length, 3);
    assert.equal(raw[0].startLine, 1);
    assert.equal(raw[1].startLine, 2);
    assert.equal(raw[2].startLine, 4);

    const parsed = parseCsvRecords(csv, ',');
    assert.deepEqual(parsed.map((r) => r.cells), [
      ['id', 'notes', 'amount'],
      ['1', 'First line\nSecond "quoted" line', '42'],
      ['2', 'Alpha\r\nBeta', '99'],
    ]);
  });

  it('honors explicit delimiter preference over auto-detection', () => {
    const sample = 'a,b,c\n1,2,3\n';
    assert.equal(resolveCsvDelimiter(sample, 'data.csv', 'auto'), ',');
    assert.equal(resolveCsvDelimiter(sample, 'data.csv', ';'), ';');
    assert.equal(resolveCsvDelimiter('a\tb\n1\t2\n', 'table.tsv', 'auto'), '\t');
  });
});

describe('IDE preferences migration, auto-save defaults, and tab cap clamping', () => {
  it('enables autoSave by default and clamps maxOpenTabs within 15..40', () => {
    assert.equal(DEFAULT_IDE_PREFERENCES.autoSave, true);
    assert.equal(DEFAULT_IDE_PREFERENCES.maxOpenTabs, 20);

    withMockStorage(() => {
      const low = writeIdePreferences({ maxOpenTabs: 3, autoSave: false });
      assert.equal(low.maxOpenTabs, 15);
      assert.equal(low.autoSave, false);

      const high = writeIdePreferences({ maxOpenTabs: 250, localHistoryRetentionDays: 365 });
      assert.equal(high.maxOpenTabs, 40);
      assert.equal(high.localHistoryRetentionDays, 90);
    });
  });

  it('migrates legacy otto.ide.keymap and removes stale key on update so keymap never drifts', () => {
    withMockStorage((storage) => {
      storage.setItem('otto.ide.keymap', 'standard');
      const migrated = readIdePreferences();
      assert.equal(migrated.keymapPreset, 'standard');

      const switched = writeIdePreferences({ keymapPreset: 'jetbrains' });
      assert.equal(switched.keymapPreset, 'jetbrains');
      assert.equal(storage.getItem('otto.ide.keymap'), null);
      assert.equal(storage.getItem('otto.ide.keymapPreset'), 'jetbrains');
      assert.equal(readIdePreferences().keymapPreset, 'jetbrains');

      const rawJson = JSON.parse(storage.getItem(IDE_PREFS_STORAGE_KEY) ?? '{}') as Record<
        string,
        unknown
      >;
      assert.equal('otto.ide.keymap' in rawJson, false);
    });
  });

  it('resolves effective color scheme for auto and explicit selections', () => {
    assert.equal(
      resolveEffectiveIdeScheme(
        {
          colorScheme: 'auto',
          preferredDarkScheme: 'jetbrains-darcula',
          preferredLightScheme: 'vscode-light-modern',
        },
        'dark',
      ),
      'jetbrains-darcula',
    );
    assert.equal(
      resolveEffectiveIdeScheme(
        {
          colorScheme: 'jetbrains-islands-light',
          preferredDarkScheme: 'jetbrains-darcula',
          preferredLightScheme: 'vscode-light-modern',
        },
        'dark',
      ),
      'jetbrains-islands-light',
    );
  });
});

describe('IDE Live DB Watch target prioritization and cell formatting', () => {
  const savedMysql: IdeSavedDbTarget = {
    target_kind: 'saved',
    connection_id: 'conn-dev-mysql',
    name: 'Dev MySQL',
    engine: 'mysql',
    host: '127.0.0.1',
    port: 3306,
    environment: 'dev',
    database: 'pr_bo_1',
  };
  const ephemeralMysql: IdeEphemeralDbTarget = {
    target_kind: 'ephemeral',
    ephemeral_id: 'docker:abc123def456:3306',
    name: 'testcontainers-mysql-1',
    engine: 'mysql',
    host: '127.0.0.1',
    port: 55124,
    database: 'pr_bo_test',
    source: 'testcontainers',
    container_image: 'mysql:8.0',
  };

  it('prefers live ephemeral Testcontainers over saved connections for matching engine', () => {
    assert.equal(IDE_DB_ENGINES.length, 5);
    const chosen = pickPreferredDbTarget('mysql', [savedMysql], [ephemeralMysql]);
    assert.ok(chosen);
    assert.equal(targetKeyOf(chosen), 'ephemeral:docker:abc123def456:3306');
  });

  it('prefers a saved connection over an unlabelled container from another project', () => {
    const other: IdeEphemeralDbTarget = { ...ephemeralMysql, ephemeral_id: 'tc:112233445566:13306', source: 'container' };
    const chosen = pickPreferredDbTarget('mysql', [savedMysql], [other]);
    assert.ok(chosen);
    assert.equal(targetKeyOf(chosen), 'saved:conn-dev-mysql');
    assert.equal(pickPreferredDbTarget('mysql', [], [other])?.target_kind, 'ephemeral', 'still offered with nothing saved');
  });

  it('preserves explicit user target selection when engine still matches', () => {
    const chosen = pickPreferredDbTarget(
      'mysql',
      [savedMysql],
      [ephemeralMysql],
      'saved:conn-dev-mysql',
    );
    assert.ok(chosen);
    assert.equal(targetKeyOf(chosen), 'saved:conn-dev-mysql');
  });

  it('formats null, undefined, scalar, and structured database cell values cleanly', () => {
    assert.equal(formatDbCellValue(null), 'NULL');
    assert.equal(formatDbCellValue(undefined), '—');
    assert.equal(formatDbCellValue(42), '42');
    assert.equal(formatDbCellValue(true), 'true');
    assert.equal(formatDbCellValue({ tier: 'vip' }), '{"tier":"vip"}');
  });
});
