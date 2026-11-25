import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { ensureSyntaxTree, StreamLanguage } from '@codemirror/language';
import { EditorState, type Extension } from '@codemirror/state';
import { makefileParser, type MakeState } from '../src/lib/components/cm-lang-makefile.ts';
import { cmLangNow, loadCmLang } from '../src/lib/components/cm-langs.ts';

// Highlighting budget (owner, 2026-11-24: "the formatting of makefile is very slow … i
// scroll, and i see the colors slowly added"). Every language the editor loads tokenizes a
// realistic 2,000-line file start to end in under 30 ms, and an edit re-tokenizes from the
// changed line, not the whole file. The scroll side (no line painted before its colours)
// is e2e/desktop-ide-highlight-perf.spec.ts. Every StreamLanguage of ours is in the table.

const BUDGET_MS = 30 * Number(process.env.OTTO_PERF_BUDGET_SCALE ?? (process.env.CI ? 2 : 1));
const MAKEFILE = readFileSync(new URL('./fixtures/Makefile.sample', import.meta.url), 'utf8');
const LONG_COMMENT = Array.from({ length: 12 }, () => '# ' + 'This target builds the acceptance suite with every flag the CI pipeline needs, see docs/ci.md. '.repeat(3)).join('\n');

/** `lines` lines made of `block` repeated (each copy numbered so no two are equal). */
export function repeatTo(lines: number, block: (i: number) => string): string {
  const out: string[] = [];
  for (let i = 0; out.length < lines; i++) out.push(...block(i).split('\n'));
  return out.slice(0, lines).join('\n') + '\n';
}

const SAMPLES: Record<string, string> = {
  makefile: repeatTo(2000, (i) => `${MAKEFILE.replace(/^(\w[\w-]*):/gm, `$1_${i}:`)}\n${LONG_COMMENT}\nVERSION_${i} := $(shell git describe --tags --always --dirty 2>/dev/null | sed -e 's/^v//' -e 's/-/./g')`),
  sh: repeatTo(2000, (i) => `#!/usr/bin/env bash\nset -euo pipefail\nNAME_${i}="app-${i}"\nif [[ -n "${'$'}{CI:-}" ]]; then\n  echo "building ${'$'}NAME_${i}" | tee -a build.log\n  go build -o "bin/${'$'}NAME_${i}" ./cmd/... && ls -la bin/\nfi\n# ${LONG_COMMENT.slice(2, 200)}`),
  toml: repeatTo(2000, (i) => `[package_${i}]\nname = "otto-${i}"\nversion = "0.${i}.0"\nedition = "2021"\n# comment ${i}\n[dependencies_${i}]\nserde = { version = "1", features = ["derive"] }\nenabled = true\nretries = ${i}`),
  properties: repeatTo(2000, (i) => 'spring.datasource' + i + '.url=jdbc:mysql://' + '$' + '{DB_HOST:localhost}:3306/games?useSSL=false\n# comment ' + i + '\nfeature' + i + '.flags={"enabled": true, "rollout": [1, 2, 3], "name": "x' + i + '", "nested": {"a": [{"b": null}]}}\nlong' + i + '=' + 'value with \\u0020 escapes and words '.repeat(20) + '\\\n    continued'),
  yaml: repeatTo(2000, (i) => 'service' + i + ':\n  image: "nginx:1.' + i + '"\n  ports: [80, 443]\n  env: &env' + i + '\n    A: 1\n    B: true\n  other: *env' + i + '\n  script: |\n    echo ' + i + '\n    exit 0\n# comment\n---'),
  conf: repeatTo(2000, (i) => '[section' + i + ']\nhost = db' + i + '.internal ; comment\nport=5432\nenabled = yes\n127.0.0.1 host' + i),
  dotenv: repeatTo(2000, (i) => 'export DB_URL_' + i + '=postgres://u@h/db\nAPI_KEY_' + i + '="abc"\n# c'),
  ignore: repeatTo(2000, (i) => '# c\n/dist' + i + '/\n**/*.log\n!keep' + i + '.log\n*.png binary -diff'),
  requirements: repeatTo(2000, (i) => 'requests' + i + '[socks]>=2.31,<3 ; python_version < "3.12"\n# pinned\ndjango==5.0.' + i),
  dockerfile: repeatTo(2000, (i) => 'FROM golang:1.23 AS build' + i + '\nARG VERSION=dev\nENV CGO_ENABLED=0 GOOS=linux\nCOPY --from=build /out/app /app\nRUN go build -o /out/app ./cmd/... && \\\n    echo "built $VERSION"\nENTRYPOINT ["/app", "--port", "8080"]'),
  nginx: repeatTo(2000, (i) => 'server {\n  listen 44' + i + ' ssl;\n  location ~* \\.php$ {\n    proxy_pass http://$upstream;\n  }\n}'),
  proto: repeatTo(2000, (i) => 'message Bet' + i + ' {\n  string id = 1; // the id\n  repeated Leg legs = 2;\n}\n/* block\n comment */'),
  hcl: repeatTo(2000, (i) => 'resource "aws_s3_bucket" "logs' + i + '" {\n  bucket = "logs-' + '$' + '{var.env}"\n  tags   = merge(local.tags, { a = true })\n}'),
  diff: repeatTo(2000, (i) => 'diff --git a/x' + i + ' b/x\n--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@ func main\n-old\n+new\n same'),
  powershell: repeatTo(2000, (i) => 'param([string]$Name' + i + ' = "x")\nif ($Name -eq $null) { Write-Host "hi" -ForegroundColor Green }\n<# c #>'),
  batch: repeatTo(2000, (i) => '@echo off\nREM build\n:loop' + i + '\nset NAME=%1\nif "%NAME%"=="" goto loop'),
  mod: repeatTo(2000, (i) => `require (\n\tgithub.com/example/pkg${i} v1.${i}.0 // indirect\n\tgolang.org/x/sys v0.${i}.0\n)\nreplace example.com/a${i} => ../a${i}`),
};

async function parseMs(ext: string, text: string): Promise<number> {
  await loadCmLang(ext);
  const lang = cmLangNow(ext, 'mysql');
  assert.ok(lang, ext);
  const times: number[] = [];
  for (let k = 0; k < 5; k++) {
    const st = EditorState.create({ doc: text, extensions: [lang as Extension] });
    const t0 = performance.now();
    const tree = ensureSyntaxTree(st, st.doc.length, 5_000);
    times.push(performance.now() - t0);
    assert.equal(tree?.length, st.doc.length, `${ext}: parsed to the end`);
  }
  return times.sort((a, b) => a - b)[2];
}

for (const [ext, text] of Object.entries(SAMPLES)) {
  test(`${ext}: 2,000 lines tokenize start to end within ${BUDGET_MS} ms`, async () => {
    assert.equal(text.split('\n').length - 1, 2000);
    const ms = await parseMs(ext, text);
    console.log(`${ext}: ${ms.toFixed(1)} ms for 2,000 lines`);
    assert.ok(ms < BUDGET_MS, `${ext}: ${ms.toFixed(1)} ms`);
  });
}

test('an edit re-tokenizes from the changed line, not the whole Makefile', () => {
  let lines = 0;
  const counting = StreamLanguage.define<MakeState>({
    ...makefileParser,
    token: (s, st) => {
      if (s.sol()) lines++;
      return makefileParser.token(s, st);
    },
  });
  const text = SAMPLES.makefile;
  const st = EditorState.create({ doc: text, extensions: [counting] });
  // The editor parses up to the end of what is on screen: say lines 1–1,040.
  const shown = st.doc.line(1040).to;
  ensureSyntaxTree(st, shown, 5_000);
  lines = 0;
  const at = st.doc.line(1000).from;
  const next = st.update({ changes: { from: at, insert: 'X' } }).state;
  // …and after the edit, parses (as the editor's eager-parse plugin does) to the screen's end.
  ensureSyntaxTree(next, next.doc.line(1040).to, 5_000);
  console.log(`one keystroke at line 1,000 re-tokenized ${lines} lines`);
  assert.ok(lines > 0 && lines <= 60, `re-tokenized ${lines} lines for one keystroke at line 1,000`);
});
