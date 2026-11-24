// Which tool rows are MCP calls (modules/threads/runtime/timeline.ts
// `mcpToolName`): every provider's spelling is "server · tool", and an ACP
// tool row titled with a file name is never mistaken for one.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mcpToolName } from '../src/modules/threads/runtime/timeline.ts';

test('each provider’s MCP spelling', () => {
  assert.deepEqual(mcpToolName('mcp__github__list_pulls'), { server: 'github', tool: 'list_pulls' });
  assert.deepEqual(mcpToolName('github.list_pulls'), { server: 'github', tool: 'list_pulls' });
  assert.deepEqual(mcpToolName('Run mcp_github_list_pulls'), { server: 'github', tool: 'list_pulls' });
});

test('a file name is not "server · tool"', () => {
  for (const f of ['pom.xml', 'styles.scss', 'main.c', 'util.h', 'app.cpp', 'App.kt', 'View.swift', 'build.gradle', 'app.properties', 'Card.vue', 'index.php', 'Program.cs', 'README.md']) {
    assert.equal(mcpToolName(f), null, f);
  }
});
