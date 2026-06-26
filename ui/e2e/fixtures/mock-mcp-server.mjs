#!/usr/bin/env node
// A tiny stdio MCP server for E2E: newline-delimited JSON-RPC 2.0 over stdin/stdout.
// Implements just enough of the protocol for the control plane to discover, health
// check, and invoke tools. Two tools: a read-only `list_items` and a dangerous
// `delete_thing` (dangerous by name → require_approval on discovery).
import { createInterface } from 'node:readline';

const TOOLS = [
  {
    name: 'list_items',
    description: 'list items',
    inputSchema: { type: 'object', properties: {} },
    annotations: { readOnlyHint: true },
  },
  {
    name: 'delete_thing',
    description: 'delete a thing',
    inputSchema: { type: 'object', properties: { id: { type: 'number' } } },
    annotations: {},
  },
];

const reply = (msg) => process.stdout.write(JSON.stringify(msg) + '\n');

const rl = createInterface({ input: process.stdin });
rl.on('line', (line) => {
  const trimmed = line.trim();
  if (!trimmed) return;
  let req;
  try {
    req = JSON.parse(trimmed);
  } catch {
    return; // tolerate stray non-JSON noise
  }
  switch (req.method) {
    case 'initialize':
      reply({
        jsonrpc: '2.0',
        id: req.id,
        result: { protocolVersion: '2024-11-05', capabilities: {}, serverInfo: { name: 'mock', version: '1' } },
      });
      break;
    case 'tools/list':
      reply({ jsonrpc: '2.0', id: req.id, result: { tools: TOOLS } });
      break;
    case 'tools/call':
      reply({ jsonrpc: '2.0', id: req.id, result: { content: [{ type: 'text', text: 'done' }], isError: false } });
      break;
    case 'notifications/initialized':
      break; // a notification — no reply
    default:
      if (req.id !== undefined) reply({ jsonrpc: '2.0', id: req.id, error: { code: -32601, message: 'method not found' } });
  }
});
