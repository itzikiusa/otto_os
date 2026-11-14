import assert from 'node:assert/strict';
import { test } from 'node:test';
import type { Session } from '../src/lib/api/types';
import {
  buildIdePathAliases,
  canonicalIdePath,
  discoverIdeProjects,
  groupThreadsByIdeProject,
  matchSessionToIdeProject,
  parseNamedIdeProjects,
  parseSavedAdditionalRoots,
} from '../src/modules/threads/ideProjectModel.ts';
import type { AskOption } from '../src/modules/threads/runtime/askAgents.ts';
import {
  addComposerContext,
  assembleComposerMessage,
  fileMentionContext,
} from '../src/modules/threads/runtime/composerContext.ts';
import {
  atMentionTokenAt,
  buildUnifiedMentionResults,
  computeRelativeDisplay,
  parseUserMessageWithBadges,
  rankFileMentionItems,
  replaceAtMentionToken,
} from '../src/modules/threads/runtime/mentionMatcher.ts';

function makeSession(overrides: Partial<Session> & { id: string }): Session {
  return {
    id: overrides.id,
    workspace_id: overrides.workspace_id ?? 'wsp_1',
    kind: overrides.kind ?? 'agent',
    title: overrides.title ?? overrides.id,
    provider: overrides.provider ?? 'claude',
    status: overrides.status ?? 'idle',
    cwd: overrides.cwd ?? '',
    provider_session_id: overrides.provider_session_id ?? null,
    connection_id: overrides.connection_id ?? null,
    created_by: overrides.created_by ?? 'usr_1',
    created_at: overrides.created_at ?? '2026-11-14T04:00:00Z',
    last_active_at: overrides.last_active_at ?? '2026-11-14T04:05:00Z',
    archived: overrides.archived ?? false,
    meta: overrides.meta ?? {},
  };
}

test('atMentionTokenAt parses empty @, fuzzy files, ~/, ./, ../, /abs, @alias/, quoted paths, and rejects emails', () => {
  assert.deepEqual(atMentionTokenAt('@'), {
    query: '',
    start: 0,
    end: 1,
    isPathLike: false,
  });

  assert.deepEqual(atMentionTokenAt('inspect @main.go'), {
    query: 'main.go',
    start: 8,
    end: 16,
    isPathLike: true,
  });

  // Home, parent, relative, absolute, and project-alias navigation queries:
  for (const nav of [
    '~/go_admission/promotions/',
    '../go_utilities/logger',
    './cmd/server',
    '/Users/dev/work/pom.xml',
    'promotions/internal/service.go',
  ]) {
    const text = `check @${nav}`;
    const tok = atMentionTokenAt(text);
    assert.ok(tok, `expected token for ${nav}`);
    assert.equal(tok.query, nav);
    assert.equal(tok.isPathLike, true);
  }

  // Mid-sentence caret editing `@promo` before ` and fix tests`:
  const mid = 'review @promo and fix tests';
  const caret = 'review @promo'.length;
  const midTok = atMentionTokenAt(mid, caret);
  assert.deepEqual(midTok, {
    query: 'promo',
    start: 7,
    end: 13,
    isPathLike: false,
  });

  // Quoted path with spaces stays active until closing quote:
  const openQuoted = 'open @"docs/my design';
  assert.deepEqual(atMentionTokenAt(openQuoted), {
    query: 'docs/my design',
    start: 5,
    end: openQuoted.length,
    isPathLike: true,
  });
  assert.equal(atMentionTokenAt('open @"docs/my design"'), null);

  // Rejects email addresses and trailing whitespace:
  assert.equal(atMentionTokenAt('contact dev@otto.local'), null);
  assert.equal(atMentionTokenAt('check @codex now'), null);
});

test('replaceAtMentionToken handles inline file reference, agent removal, and keepOpen folder drill-down', () => {
  // 1. Agent pick removes `@cod` cleanly (preserving `check this ` exactly):
  const agentText = 'check this @cod';
  const agentTok = atMentionTokenAt(agentText)!;
  const afterAgent = replaceAtMentionToken(agentText, agentTok, '');
  assert.equal(afterAgent.text, 'check this ');
  assert.equal(afterAgent.caret, 'check this '.length);

  // 2. Folder drill-down keeps `@~/go_admission/` open without trailing space:
  const dirText = 'inspect @~/go_ad';
  const dirTok = atMentionTokenAt(dirText)!;
  const drilled = replaceAtMentionToken(dirText, dirTok, '@~/go_admission/', { keepOpen: true });
  assert.equal(drilled.text, 'inspect @~/go_admission/');
  assert.equal(drilled.caret, drilled.text.length);
  assert.deepEqual(atMentionTokenAt(drilled.text, drilled.caret), {
    query: '~/go_admission/',
    start: 8,
    end: drilled.text.length,
    isPathLike: true,
  });

  // 3. Picking a file inside a sentence splices `@promotions/cmd/main.go ` cleanly:
  const midSentence = 'compare @main with production';
  const midTok = atMentionTokenAt(midSentence, 'compare @main'.length)!;
  const picked = replaceAtMentionToken(midSentence, midTok, '@promotions/cmd/main.go');
  assert.equal(picked.text, 'compare @promotions/cmd/main.go with production');
  assert.equal(picked.caret, 'compare @promotions/cmd/main.go '.length);

  // 4. Quoted paths with spaces get wrapped safely by computeRelativeDisplay:
  const relSpaced = computeRelativeDisplay('/ws/my folder/spec.md', '/ws');
  assert.equal(relSpaced.display, 'my folder/spec.md');
  assert.equal(relSpaced.insertToken, '@"my folder/spec.md"');
});

test('rankFileMentionItems & buildUnifiedMentionResults rank open tabs, exact files, and strong @agent matches accurately', () => {
  const rawItems = [
    { path: '/ws/promotions/pkg/helper_main.go', name: 'helper_main.go', kind: 'file' },
    { path: '/ws/promotions/cmd/main.go', name: 'main.go', kind: 'file' },
    { path: '/ws/promotions/internal/main_service.go', name: 'main_service.go', kind: 'file' },
    { path: '/ws/promotions/README.md', name: 'README.md', kind: 'file', openTab: true, activeTab: true },
  ];

  // Empty query `@` surfaces active/open IDE tabs at the very top:
  const emptyRanked = rankFileMentionItems(rawItems, '', { cwd: '/ws/promotions' });
  assert.equal(emptyRanked[0].name, 'README.md');
  assert.equal(emptyRanked[0].openTab, true);

  // `@main` ranks exact prefix `main.go` / `main_service.go` ahead of substring `helper_main.go`:
  const mainRanked = rankFileMentionItems(rawItems, 'main', { cwd: '/ws/promotions' });
  assert.equal(mainRanked[0].name, 'main.go');
  assert.equal(mainRanked[1].name, 'main_service.go');
  assert.equal(mainRanked[2].name, 'helper_main.go');

  const agents: AskOption[] = [
    { provider: 'codex', model: null, label: 'Codex', detail: 'Codex' },
    { provider: 'claude', model: null, label: 'Claude Code', detail: 'Claude Code' },
  ];

  // `@cod` strongly matches `Codex`, placing `Codex` FIRST so Enter delegates to Codex immediately:
  const codUnified = buildUnifiedMentionResults({
    query: 'cod',
    filter: 'all',
    files: rankFileMentionItems(
      [{ path: '/ws/promotions/pkg/unicode_helper.go', name: 'unicode_helper.go', kind: 'file' }],
      'cod',
      { cwd: '/ws/promotions' },
    ),
    agents: agents.filter((a) => a.provider.startsWith('cod')),
    allowAgents: true,
  });
  assert.equal(codUnified.agentsFirst, true);
  assert.equal(codUnified.flat[0]?.kind, 'agent');

  // Path query `@cmd/main` hides agent noise and shows files first:
  const pathUnified = buildUnifiedMentionResults({
    query: 'cmd/main',
    filter: 'all',
    files: mainRanked,
    agents,
    allowAgents: true,
  });
  assert.equal(pathUnified.agents.length, 0);
  assert.equal(pathUnified.flat[0]?.kind, 'file');

  // Filter pills `files` vs `agents` isolate each section cleanly:
  assert.equal(
    buildUnifiedMentionResults({ query: '', filter: 'files', files: emptyRanked, agents }).agents.length,
    0,
  );
  assert.equal(
    buildUnifiedMentionResults({ query: '', filter: 'agents', files: emptyRanked, agents }).files.length,
    0,
  );
});

test('fileMentionContext + assembleComposerMessage + parseUserMessageWithBadges round-trip IDE & file context cleanly', () => {
  const fileChip = fileMentionContext({
    path: '/ws/promotions/cmd/main.go',
    label: '@cmd/main.go',
  });
  const dirChip = fileMentionContext({
    path: '/ws/promotions/internal',
    label: '@internal/',
    isDir: true,
  });
  assert.equal(fileChip.kind, 'diff');
  assert.equal(fileChip.id, 'file:/ws/promotions/cmd/main.go');
  assert.equal(dirChip.id, 'dir:/ws/promotions/internal');

  const hyphenChip = fileMentionContext({
    path: '/ws/promotions/internal/my-service.go',
    label: '@internal/my-service.go',
  });
  const list = addComposerContext(addComposerContext(addComposerContext([], fileChip), dirChip), hyphenChip);
  const assembled = assembleComposerMessage('Check initialization flow', list);
  assert.ok(assembled.includes('--- File context: @cmd/main.go ---'));
  assert.ok(assembled.includes('--- Folder context: @internal/ ---'));
  assert.ok(assembled.includes('--- File context: @internal/my-service.go ---'));

  const wrappedWithIde = [
    '<ide_context>',
    '<item index="1" kind="selection" label="@cmd/main.go:L10-L24" path="/ws/promotions/cmd/main.go" lines="10-24" language="go">',
    'func main() {}',
    '</item>',
    '</ide_context>',
    '',
    assembled,
  ].join('\n');

  const parsed = parseUserMessageWithBadges(wrappedWithIde);
  assert.equal(parsed.prose, 'Check initialization flow');
  assert.equal(parsed.badges.length, 4);
  assert.equal(parsed.badges[0].kind, 'selection');
  assert.equal(parsed.badges[0].label, '@cmd/main.go:L10-L24');
  assert.equal(parsed.badges[0].line, 10);
  assert.equal(parsed.badges[1].label, '@cmd/main.go');
  assert.equal(parsed.badges[1].path, '/ws/promotions/cmd/main.go');
  assert.equal(parsed.badges[2].kind, 'folder');
  assert.equal(parsed.badges[2].path, '/ws/promotions/internal');
  assert.equal(parsed.badges[3].label, '@internal/my-service.go');
  assert.equal(parsed.badges[3].path, '/ws/promotions/internal/my-service.go');
});

test('discoverIdeProjects & groupThreadsByIdeProject unify single-repo and multi-repo IDE projects in Threads sidebar', () => {
  const namedJson = JSON.stringify([
    {
      id: 'proj-casino-go',
      name: 'Casino Go',
      primary_root: '/Users/dev/go_admission/promotions',
      folders: [
        '/Users/dev/go_admission/promotions',
        '/Users/dev/go_utilities',
        '/Users/dev/go_casino_kit',
      ],
      pinned: true,
    },
    {
      id: 'proj-bo-java',
      name: 'BO Java',
      primary_root: '/var/folders/xy/bo-parent',
      folders: ['/var/folders/xy/bo-parent', '/var/folders/xy/bo-service'],
      pinned: false,
    },
  ]);

  const named = parseNamedIdeProjects(namedJson);
  assert.equal(named.length, 2);
  assert.deepEqual(parseSavedAdditionalRoots('["/Users/dev/extra-a/", "/Users/dev/extra-a"]'), [
    '/Users/dev/extra-a',
  ]);
  assert.equal(canonicalIdePath('/private/var/folders/xy/bo-parent/'), '/var/folders/xy/bo-parent');

  const projects = discoverIdeProjects({
    namedProjects: named,
    lastRoot: '/Users/dev/solo-rust-ide',
  });
  assert.equal(projects.length, 3);
  assert.equal(projects[0].name, 'Casino Go');
  assert.equal(projects[0].metaLabel, '3 repos');
  assert.equal(projects[1].name, 'BO Java');
  assert.equal(projects[1].metaLabel, '2 repos');
  assert.equal(projects[2].name, 'solo-rust-ide');
  assert.equal(projects[2].metaLabel, null);

  const sPromo = makeSession({ id: 'rt_promo', cwd: '/Users/dev/go_admission/promotions/internal/dao' });
  const sKit = makeSession({ id: 'rt_kit', cwd: '/Users/dev/go_casino_kit' });
  const sJavaSymlink = makeSession({ id: 'rt_java', cwd: '/private/var/folders/xy/bo-service/src/main' });
  const sOutside = makeSession({ id: 'rt_outside', cwd: '/Users/dev/unrelated-notes' });

  assert.equal(matchSessionToIdeProject(sKit, projects)?.id, 'proj-casino-go');
  assert.equal(matchSessionToIdeProject(sJavaSymlink, projects)?.id, 'proj-bo-java');
  assert.equal(matchSessionToIdeProject(sOutside, projects), null);

  const grouped = groupThreadsByIdeProject(projects, [sPromo, sKit, sJavaSymlink, sOutside]);
  assert.equal(grouped.matchedSessionIds.has('rt_promo'), true);
  assert.equal(grouped.matchedSessionIds.has('rt_kit'), true);
  assert.equal(grouped.matchedSessionIds.has('rt_java'), true);
  assert.equal(grouped.matchedSessionIds.has('rt_outside'), false);

  const casinoGroup = grouped.groups.find((g) => g.project.id === 'proj-casino-go')!;
  assert.deepEqual(
    casinoGroup.items.map((s) => s.id),
    ['rt_promo', 'rt_kit'],
  );

  const aliases = buildIdePathAliases({ projects });
  assert.equal(aliases['promotions'], '/Users/dev/go_admission/promotions');
  assert.equal(aliases['go_utilities'], '/Users/dev/go_utilities');
  assert.equal(aliases['solo-rust-ide'], '/Users/dev/solo-rust-ide');
});
