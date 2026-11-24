import assert from 'node:assert/strict';
import { test } from 'node:test';
import { StringStream } from '@codemirror/language';
import { confParserConf, confParserDotenv, ignoreParser, propertiesParser, requirementsParser, tomlParser, yamlParser } from '../src/lib/components/cm-langs-config.ts';
import { batchParser, diffParser, dockerfileParser, hclParser, nginxParser, powershellParser, protoParser, vttParser } from '../src/lib/components/cm-langs-infra.ts';

// The config / infrastructure languages (owner, 2026-11-24: "no ordinary config file should
// open as a wall of white text"), tokenized line by line as CodeMirror's StreamLanguage
// does: the style of each token, by its text.

interface Parser<S> { startState: (unit?: number) => S; token: (s: StringStream, st: S) => string | null; blankLine?: (st: S) => void }

/** Every line's tokens as [text, style]. */
export function tokenize<S>(p: Parser<S>, text: string): [string, string | null][][] {
  const st = p.startState(2);
  return text.split('\n').map((line) => {
    if (!line) { p.blankLine?.(st); return []; }
    const s = new StringStream(line, 4, 2);
    const out: [string, string | null][] = [];
    while (!s.eol()) {
      const style = p.token(s, st);
      assert.ok(s.pos > s.start, 'the tokenizer advances on: ' + line);
      out.push([s.current(), style]);
      s.start = s.pos;
    }
    return out;
  });
}

/** The style of the first token "tok" on line n (0-based). */
function styleOf(lines: [string, string | null][][], n: number, tok: string): string | null {
  const hit = lines[n].find(([t]) => t === tok);
  assert.ok(hit, 'no token ' + JSON.stringify(tok) + ' on line ' + n + ': ' + JSON.stringify(lines[n]));
  return hit[1];
}

const PROPS = [
  '# Kafka consumer, the shape of the owner\'s application.properties',
  '! legacy comment',
  'spring.datasource.url=jdbc:mysql://${DB_HOST:localhost}:3306/games?useSSL=false',
  'server.port : 8080',
  'app.name   Otto Games',
  'app.welcome=Hello\\u0020World\\n\\',
  '    second line of the value',
  'feature.flags={"enabled": true, "rollout": [1, 2, 3], "name": "x\\"y"}',
  'kafka.config=[{"topic": "bets", \\',
  '   "partitions": 12}]',
  'key\\=with\\:escapes=v',
].join('\n');

test('.properties: keys, both separators, # and ! comments, escapes, continuations, placeholders', () => {
  const l = tokenize(propertiesParser, PROPS);
  assert.equal(l[0][0][1], 'comment');
  assert.equal(l[1][0][1], 'comment');
  assert.equal(styleOf(l, 2, 'spring.datasource.url'), 'propertyName');
  assert.equal(styleOf(l, 2, '='), 'operator');
  assert.equal(styleOf(l, 2, '${DB_HOST:localhost}'), 'labelName');
  assert.equal(styleOf(l, 2, 'jdbc:mysql://'), 'string');
  assert.equal(styleOf(l, 3, 'server.port'), 'propertyName');
  assert.equal(styleOf(l, 3, ':'), 'operator');
  assert.equal(styleOf(l, 4, 'app.name'), 'propertyName');
  assert.equal(styleOf(l, 4, 'Otto Games'), 'string');
  assert.equal(styleOf(l, 5, '\\u0020'), 'escape');
  assert.equal(l[5][l[5].length - 1][1], 'escape', 'the continuation backslash');
  assert.equal(l[6].find(([t]) => t.trim())?.[1], 'string', 'a continued line is value, not a key');
  assert.equal(styleOf(l, 10, 'key\\=with\\:escapes'), 'propertyName');
});

test('.properties: a value that is JSON is highlighted as JSON, also across a continuation', () => {
  const l = tokenize(propertiesParser, PROPS);
  assert.equal(styleOf(l, 7, '"enabled"'), 'propertyName');
  assert.equal(styleOf(l, 7, 'true'), 'atom');
  assert.equal(styleOf(l, 7, '1'), 'number');
  assert.equal(styleOf(l, 7, '"x\\"y"'), 'string');
  assert.equal(styleOf(l, 7, '{'), 'punctuation');
  assert.equal(styleOf(l, 8, '"topic"'), 'propertyName');
  assert.equal(styleOf(l, 8, '"bets"'), 'string');
  assert.equal(styleOf(l, 9, '"partitions"'), 'propertyName');
  assert.equal(styleOf(l, 9, '12'), 'number');
  assert.equal(styleOf(l, 10, 'key\\=with\\:escapes'), 'propertyName', 'the JSON value closed: the next line is a key again');
});

test('YAML: keys, scalars, anchors and aliases, tags, block scalars, documents, flow collections', () => {
  const l = tokenize(yamlParser, [
    '%YAML 1.2',
    '---',
    'defaults: &defaults # shared',
    '  image: "nginx:1.27"',
    '  replicas: 3',
    '  enabled: true',
    'prod:',
    '  <<: *defaults',
    '  tags: !!set {a, b}',
    '  ports: [80, 443]',
    '  script: |',
    '    echo: not a key',
    '    exit 0',
    '  url: http://example.com:8080/x',
    '- name: first',
    '...',
  ].join('\n'));
  assert.equal(l[0][0][1], 'meta');
  assert.equal(l[1][0][1], 'meta');
  assert.equal(styleOf(l, 2, 'defaults'), 'propertyName');
  assert.equal(styleOf(l, 2, '&defaults'), 'labelName');
  assert.equal(styleOf(l, 2, '# shared'), 'comment');
  assert.equal(styleOf(l, 3, '"nginx:1.27"'), 'string');
  assert.equal(styleOf(l, 4, '3'), 'number');
  assert.equal(styleOf(l, 5, 'true'), 'atom');
  assert.equal(styleOf(l, 7, '*defaults'), 'labelName');
  assert.equal(styleOf(l, 8, '!!set'), 'meta');
  assert.equal(styleOf(l, 9, '80'), 'number');
  assert.equal(styleOf(l, 9, '['), 'punctuation');
  assert.equal(styleOf(l, 10, '|'), 'operator');
  assert.equal(l[11][0][1], 'string', 'a block scalar line is text, even with a colon');
  assert.equal(l[12][0][1], 'string');
  assert.equal(styleOf(l, 13, 'url'), 'propertyName', 'the block ended');
  assert.equal(styleOf(l, 13, 'http://example.com:8080/x'), 'string');
  assert.equal(styleOf(l, 14, '-'), 'punctuation');
  assert.equal(styleOf(l, 14, 'name'), 'propertyName');
});

test('TOML: tables, dotted and quoted keys, inline tables, arrays over lines, multi-line strings, dates', () => {
  const l = tokenize(tomlParser, [
    '[package]',
    'name = "otto"',
    'site."google.com" = true',
    'serde = { version = "1", features = ["derive"] }',
    'list = [',
    '  "a", # one',
    '  "b",',
    ']',
    'desc = """',
    'name = not a key',
    '"""',
    'when = 1979-05-27T07:32:00Z',
    '[[bin]]',
  ].join('\n'));
  assert.equal(styleOf(l, 0, '[package]'), 'typeName');
  assert.equal(styleOf(l, 1, 'name'), 'propertyName');
  assert.equal(styleOf(l, 1, '"otto"'), 'string');
  assert.equal(styleOf(l, 2, 'site."google.com"'), 'propertyName');
  assert.equal(styleOf(l, 3, 'version'), 'propertyName');
  assert.equal(styleOf(l, 3, 'features'), 'propertyName');
  assert.equal(styleOf(l, 5, '# one'), 'comment');
  assert.equal(l[9][0][1], 'string', 'inside a multi-line string');
  assert.equal(styleOf(l, 11, '1979-05-27T07:32:00Z'), 'number');
  assert.equal(styleOf(l, 12, '[[bin]]'), 'typeName');
});

test('INI / conf / .env and the key = value fallback', () => {
  const l = tokenize(confParserConf, [
    '; php.ini',
    '[database]',
    'host = db.internal',
    'port=5432',
    'enabled = yes',
    '127.0.0.1 localhost',
    '//registry.npmjs.org/:_authToken=${NPM_TOKEN}',
    'Port 22',
    'path = "/var/lib/x" # where',
  ].join('\n'));
  assert.equal(l[0][0][1], 'comment');
  assert.equal(l[1][0][1], 'typeName');
  assert.equal(styleOf(l, 2, 'host'), 'propertyName');
  assert.equal(styleOf(l, 2, '='), 'operator');
  assert.equal(styleOf(l, 2, 'db.internal'), 'string');
  assert.equal(styleOf(l, 3, '5432'), 'number');
  assert.equal(styleOf(l, 4, 'yes'), 'atom');
  assert.equal(styleOf(l, 5, '127.0.0.1'), 'number');
  assert.equal(styleOf(l, 6, '//registry.npmjs.org/:_authToken'), 'propertyName');
  assert.equal(styleOf(l, 6, '${NPM_TOKEN}'), 'labelName');
  assert.equal(styleOf(l, 7, 'Port'), 'propertyName');
  assert.equal(styleOf(l, 8, '"/var/lib/x"'), 'string');
  assert.equal(styleOf(l, 8, '# where'), 'comment');
  const e = tokenize(confParserDotenv, 'export DB_URL=postgres://u@h/db\nAPI_KEY="abc"');
  assert.equal(styleOf(e, 0, 'export'), 'keyword');
  assert.equal(styleOf(e, 0, 'DB_URL'), 'propertyName');
  assert.equal(styleOf(e, 1, '"abc"'), 'string');
});

test('ignore files, .gitattributes, CODEOWNERS and requirements.txt', () => {
  const g = tokenize(ignoreParser, '# build output\n/dist/\n**/*.log\n!keep.log\n*.png binary -diff\n/src/ @team/core');
  assert.equal(g[0][0][1], 'comment');
  assert.equal(styleOf(g, 1, 'dist'), 'string');
  assert.equal(styleOf(g, 2, '**'), 'operator');
  assert.equal(styleOf(g, 3, '!'), 'keyword');
  assert.equal(styleOf(g, 4, 'binary'), 'attributeName');
  assert.equal(styleOf(g, 5, '@team/core'), 'labelName');
  const r = tokenize(requirementsParser, '-r base.txt\nrequests[socks]>=2.31,<3 ; python_version < "3.12"\n# pinned\ndjango==5.0.1');
  assert.equal(styleOf(r, 0, '-r'), 'attributeName');
  assert.equal(styleOf(r, 1, 'requests'), 'propertyName');
  assert.equal(styleOf(r, 1, '>='), 'operator');
  assert.equal(styleOf(r, 1, '2.31'), 'number');
  assert.equal(styleOf(r, 1, '"3.12"'), 'string');
  assert.equal(r[2][0][1], 'comment');
  assert.equal(styleOf(r, 3, '5.0.1'), 'number');
});

test('Dockerfile: instructions, RUN as shell, FROM … AS stage, ENV keys, flags, comments', () => {
  const l = tokenize(dockerfileParser, [
    '# syntax=docker/dockerfile:1',
    'FROM golang:1.23 AS build',
    'ARG VERSION=dev',
    'ENV CGO_ENABLED=0 GOOS=linux',
    'COPY --from=build /out/app /app',
    'RUN go build -o /out/app ./cmd/... && \\',
    '    echo "built $VERSION"',
    '# a comment',
    'EXPOSE 8080/tcp',
    'ENTRYPOINT ["/app", "--port", "8080"]',
  ].join('\n'));
  assert.equal(l[0][0][1], 'meta');
  assert.equal(styleOf(l, 1, 'FROM'), 'keyword');
  assert.equal(styleOf(l, 1, 'AS'), 'keyword');
  assert.equal(styleOf(l, 1, 'build'), 'labelName');
  assert.equal(styleOf(l, 2, 'VERSION'), 'propertyName');
  assert.equal(styleOf(l, 3, 'GOOS'), 'propertyName');
  assert.equal(styleOf(l, 4, '--from=build'), 'attributeName');
  assert.equal(styleOf(l, 5, 'go'), 'variableName.function');
  assert.equal(styleOf(l, 5, '-o'), 'attributeName');
  assert.equal(styleOf(l, 6, 'echo'), 'builtin', 'the continued RUN line is still shell');
  assert.equal(l[7][0][1], 'comment');
  assert.equal(styleOf(l, 8, '8080/tcp'), 'number');
  assert.equal(styleOf(l, 9, '/app'), 'string');
});

test('nginx, proto, HCL, diff, PowerShell and batch get their own tokens', () => {
  const n = tokenize(nginxParser, 'server {\n  listen 443 ssl;\n  location ~* \\.php$ {\n    proxy_pass http://$upstream;\n  }\n}');
  assert.equal(styleOf(n, 0, 'server'), 'keyword');
  assert.equal(styleOf(n, 1, 'listen'), 'keyword');
  assert.equal(styleOf(n, 1, '443'), 'number');
  assert.equal(styleOf(n, 2, '~*'), 'operator');
  assert.equal(styleOf(n, 3, '$upstream'), 'variableName.special');
  const p = tokenize(protoParser, 'syntax = "proto3";\nmessage Bet {\n  string id = 1; // the id\n  repeated Leg legs = 2;\n}\nservice Bets { rpc Place(Bet) returns (Bet); }');
  assert.equal(styleOf(p, 1, 'message'), 'keyword');
  assert.equal(styleOf(p, 1, 'Bet'), 'typeName');
  assert.equal(styleOf(p, 2, 'string'), 'typeName');
  assert.equal(styleOf(p, 2, 'id'), 'propertyName');
  assert.equal(styleOf(p, 2, '// the id'), 'comment');
  assert.equal(styleOf(p, 5, 'Place'), 'variableName.function.definition');
  const h = tokenize(hclParser, 'resource "aws_s3_bucket" "logs" {\n  bucket = "logs-${var.env}"\n  tags   = merge(local.tags, { a = true })\n  policy = <<EOF\n{ "not": "hcl" }\nEOF\n}');
  assert.equal(styleOf(h, 0, 'resource'), 'keyword');
  assert.equal(styleOf(h, 1, 'bucket'), 'propertyName');
  assert.equal(styleOf(h, 1, '${'), 'escape');
  assert.equal(styleOf(h, 1, 'var'), 'keyword');
  assert.equal(styleOf(h, 2, 'merge'), 'variableName.function');
  assert.equal(styleOf(h, 2, 'true'), 'atom');
  assert.equal(h[4][0][1], 'string', 'a heredoc body is text');
  const d = tokenize(diffParser, 'diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@ func main\n-old\n+new\n same');
  assert.equal(d[0][0][1], 'meta');
  assert.equal(d[1][0][1], 'heading');
  assert.equal(d[3][0][1], 'typeName');
  assert.equal(d[4][0][1], 'deleted');
  assert.equal(d[5][0][1], 'inserted');
  assert.equal(d[6][0][1], null);
  const ps = tokenize(powershellParser, '<# help #>\nparam([string]$Name = "x")\nif ($Name -eq $null) { Write-Host "hi" -ForegroundColor Green }');
  assert.equal(ps[0][0][1], 'comment');
  assert.equal(styleOf(ps, 1, '[string]'), 'typeName');
  assert.equal(styleOf(ps, 1, '$Name'), 'variableName.special');
  assert.equal(styleOf(ps, 2, '-eq'), 'operator');
  assert.equal(styleOf(ps, 2, '$null'), 'atom');
  assert.equal(styleOf(ps, 2, 'Write-Host'), 'variableName.function');
  assert.equal(styleOf(ps, 2, '-ForegroundColor'), 'attributeName');
  const b = tokenize(batchParser, '@echo off\nREM build\n:loop\nset NAME=%1\nif "%NAME%"=="" goto loop');
  assert.equal(styleOf(b, 0, '@'), 'meta');
  assert.equal(styleOf(b, 0, 'echo'), 'keyword');
  assert.equal(b[1][0][1], 'comment');
  assert.equal(b[2][0][1], 'labelName');
  assert.equal(styleOf(b, 3, '%1'), 'variableName.special');
  assert.equal(styleOf(b, 4, 'goto'), 'keyword');
  const v = tokenize(vttParser, 'WEBVTT\n\nNOTE a comment\n\n1\n00:00:01.000 --> 00:00:04.000 align:start\n<v Bob>Hello</v>');
  assert.equal(v[0][0][1], 'meta');
  assert.equal(v[2][0][1], 'comment');
  assert.equal(styleOf(v, 5, '00:00:01.000'), 'number');
  assert.equal(styleOf(v, 5, '-->'), 'operator');
  assert.equal(styleOf(v, 5, 'align:start'), 'attributeName');
  assert.equal(styleOf(v, 6, '<v Bob>'), 'labelName');
});
