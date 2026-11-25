import assert from 'node:assert/strict';
import { test } from 'node:test';
import { cmLangNow, loadCmLang } from '../src/lib/components/cm-langs.ts';
import { EXT_ALIAS, LANG_KEYS, NAMES, langGutters, langKeyFor, sniffLangKey } from '../src/lib/components/langForPath.ts';

// Which language a file opens with (owner, 2026-11-24, of a .properties file in white
// text: "can we add support for .properties? … .yaml .yml … .toml … or any weird lang like
// this"). Every name and extension in the table, then real-world paths.

test('every key the table can return has a language pack that loads', async () => {
  for (const key of LANG_KEYS) {
    await loadCmLang(key);
    assert.ok(cmLangNow(key, 'mysql'), key);
  }
});

test('every extension alias maps to its key', () => {
  for (const [ext, key] of Object.entries(EXT_ALIAS)) {
    assert.ok(LANG_KEYS.has(key), ext + ' → ' + key + ' has no pack');
    assert.equal(langKeyFor('/repo/some/file.' + ext), key, ext);
    assert.equal(langKeyFor('/repo/some/FILE.' + ext.toUpperCase()), key, ext + ' (upper case)');
  }
});

test('every file name maps to its key, whatever its case', () => {
  for (const [name, key] of Object.entries(NAMES)) {
    assert.ok(LANG_KEYS.has(key), name + ' → ' + key + ' has no pack');
    assert.equal(langKeyFor('/repo/' + name), key, name);
  }
  assert.equal(langKeyFor('/repo/Dockerfile'), 'dockerfile');
  assert.equal(langKeyFor('/repo/CODEOWNERS'), 'ignore');
});

const PATHS: [string, string][] = [
  ['/svc/src/main/resources/application.properties', 'properties'],
  ['/svc/gradle.properties', 'properties'],
  ['/svc/application-prod.yml', 'yaml'],
  ['/svc/docker-compose.yml', 'yaml'],
  ['/svc/docker-compose.override.yaml', 'yaml'],
  ['/svc/.github/workflows/ci.yml', 'yaml'],
  ['/svc/pnpm-lock.yaml', 'yaml'],
  ['/svc/yarn.lock', 'yaml'],
  ['/svc/Cargo.toml', 'toml'], ['/svc/pyproject.toml', 'toml'], ['/svc/Cargo.lock', 'toml'], ['/svc/poetry.lock', 'toml'],
  ['/svc/uv.lock', 'toml'], ['/svc/Pipfile', 'toml'], ['/home/u/.cargo/config', 'toml'],
  ['/svc/setup.cfg', 'conf'], ['/svc/tox.ini', 'conf'], ['/etc/mysql/my.cnf', 'conf'], ['/svc/app.conf', 'conf'],
  ['/svc/.editorconfig', 'conf'], ['/svc/.npmrc', 'conf'], ['/etc/hosts', 'conf'], ['/home/u/.ssh/config', 'conf'],
  ['/home/u/.gitconfig', 'conf'], ['/svc/.gitmodules', 'conf'], ['/etc/systemd/system/otto.service', 'conf'],
  ['/svc/.env', 'dotenv'], ['/svc/.env.local', 'dotenv'], ['/svc/.env.production', 'dotenv'], ['/svc/prod.env', 'dotenv'],
  ['/svc/.gitignore', 'ignore'], ['/svc/.dockerignore', 'ignore'], ['/svc/.gitattributes', 'ignore'], ['/svc/.prettierignore', 'ignore'],
  ['/svc/Dockerfile', 'dockerfile'], ['/svc/Dockerfile.dev', 'dockerfile'], ['/svc/dev.Dockerfile', 'dockerfile'],
  ['/svc/Containerfile', 'dockerfile'], ['/svc/build/api.dockerfile', 'dockerfile'],
  ['/etc/nginx/nginx.conf', 'nginx'], ['/etc/nginx/sites-available/default', 'nginx'], ['/etc/nginx/conf.d/api.conf', 'nginx'],
  ['/svc/api/v1/games.proto', 'proto'],
  ['/infra/main.tf', 'hcl'], ['/infra/prod.tfvars', 'hcl'], ['/infra/config.hcl', 'hcl'],
  ['/svc/build.gradle', 'gradle'], ['/svc/build.gradle.kts', 'kts'], ['/svc/Jenkinsfile', 'groovy'], ['/svc/x.groovy', 'groovy'],
  ['/svc/pom.xml', 'xml'], ['/svc/games.iml', 'xml'], ['/media/talk.en.vtt', 'vtt'], ['/svc/application-dev.yml', 'yaml'], ['/svc/Info.plist', 'xml'], ['/svc/App.csproj', 'xml'],
  ['/svc/db/V1__init.sql', 'sql'], ['/svc/fix.diff', 'diff'], ['/svc/0001-fix.patch', 'diff'],
  ['/svc/run.sh', 'sh'], ['/svc/run.bash', 'bash'], ['/svc/run.zsh', 'zsh'], ['/svc/config.fish', 'sh'], ['/home/u/.zshrc', 'sh'],
  ['/svc/build.bat', 'batch'], ['/svc/build.cmd', 'batch'], ['/svc/deploy.ps1', 'powershell'],
  ['/svc/go.mod', 'mod'], ['/svc/go.sum', 'sum'], ['/svc/go.work', 'work'],
  ['/svc/requirements.txt', 'requirements'], ['/svc/requirements-dev.txt', 'requirements'], ['/svc/constraints.txt', 'requirements'],
  ['/svc/package-lock.json', 'json'], ['/svc/composer.lock', 'json'], ['/svc/flake.lock', 'json'],
  ['/svc/Makefile', 'makefile'], ['/svc/App.svelte', 'svelte'], ['/svc/main.go', 'go'], ['/svc/a.ts', 'ts'],
];

test('real-world paths', () => {
  for (const [p, key] of PATHS) assert.equal(langKeyFor(p), key, p);
});

test('a file nothing names takes the language the IDE gave it, else keeps its extension', () => {
  assert.equal(langKeyFor('/x/notes.txt'), 'txt');
  assert.equal(langKeyFor('/x/notes.txt', 'conf'), 'conf');
  assert.equal(langKeyFor('/x/hosts.local', 'conf'), 'conf');
  assert.equal(langKeyFor('/x/strange', 'yaml'), 'yaml');
  assert.equal(langKeyFor('/x/main.c'), 'c');
  assert.equal(langKeyFor('', 'json'), 'json');
  assert.equal(langKeyFor('', 'sql'), 'sql');
  assert.equal(langKeyFor('/x/Dockerfile.ci', 'dockerfile'), 'dockerfile');
  assert.equal(langKeyFor('/x/y.proto', 'protobuf'), 'proto');
});

test('gutters: code keeps them all, a Makefile a ▶ column, everything else only the folds it uses', () => {
  const g = (p: string, hint?: string) => langGutters(p, hint);
  for (const p of ['/x/main.go', '/x/a.ts', '/x/a.py', '/x/Main.java', '/x/lib.rs', '/x/run.sh']) {
    assert.deepEqual(g(p), { breakpoints: true, runOnly: false, lint: true, fold: true }, p);
  }
  assert.deepEqual(g('/x/Makefile'), { breakpoints: false, runOnly: true, lint: false, fold: false });
  for (const p of ['/x/application.properties', '/x/.env', '/x/.gitignore', '/x/Dockerfile', '/x/requirements.txt', '/x/notes.txt', '/x/fix.ps1']) {
    assert.deepEqual(g(p), { breakpoints: false, runOnly: false, lint: false, fold: false }, p);
  }
  for (const p of ['/x/a.yaml', '/x/Cargo.toml', '/x/setup.cfg', '/x/main.tf', '/x/a.proto', '/x/README.md', '/x/pom.xml', '/x/hosts']) {
    assert.deepEqual(g(p), { breakpoints: false, runOnly: false, lint: false, fold: true }, p);
  }
  assert.deepEqual(g('/x/package.json'), { breakpoints: false, runOnly: false, lint: true, fold: true });
  assert.deepEqual(g('/x/odd', 'conf'), { breakpoints: false, runOnly: false, lint: false, fold: true });
});

test('a file with no known name is sniffed from its first lines', () => {
  const cases: [string, string][] = [
    ['127.0.0.1 localhost\n255.255.255.255 broadcasthost\n::1 localhost\n# added by Docker\n', 'conf'],
    ['registry=https://registry.npmjs.org/\n//registry.npmjs.org/:_authToken=x\nsave-exact=true\n', 'conf'],
    ['[core]\n\trepositoryformatversion = 0\n\tbare = false\n[remote "origin"]\n\turl = git@x:y.git\n', 'conf'],
    ['; php.ini\n[PHP]\nmemory_limit = 512M\n', 'conf'],
    ['name: app\nversion: 1\nitems:\n  - a\n  - b\n', 'yaml'],
    ['{\n  "a": 1\n}\n', 'json'],
    ['[\n  {"a": 1}\n]\n', 'json'],
    ['#!/usr/bin/env bash\nset -e\necho hi\n', 'shell'],
    ['#!/usr/bin/python3\nprint(1)\n', 'python'],
    ['This is a README with prose.\nIt has sentences, and no keys at all.\n', ''],
    ['', ''],
  ];
  for (const [text, want] of cases) assert.equal(sniffLangKey(text), want, JSON.stringify(text.slice(0, 30)));
  // …and the key it gives is a language here.
  assert.equal(langKeyFor('/etc/hosts.local', sniffLangKey(cases[0][0])), 'conf');
  assert.equal(langKeyFor('/x/odd.lock', sniffLangKey('{"a":1}')), 'json');
  assert.equal(langKeyFor('/x/deploy', sniffLangKey(cases[7][0])), 'sh');
});
