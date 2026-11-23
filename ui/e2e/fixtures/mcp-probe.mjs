#!/usr/bin/env node
// Tiny stdio MCP probe for live checks: echo (with its env mark), a PNG, a
// resource_link, an isError result, and an elicitation round trip.
import { createInterface } from 'node:readline';
const PNG = 'iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAIAAABLbSncAAAAEklEQVR4nGP4z8CAFWEXHbQSACj/P8Fu7N9hAAAAAElFTkSuQmCC';
const send = (m) => process.stdout.write(JSON.stringify(m) + '\n');
let nextId = 1000; const waiting = new Map();
const tools = [
  { name: 'probe_echo', description: 'Echo the text back with this server\'s PROBE_MARK env', inputSchema: { type: 'object', properties: { text: { type: 'string' } } } },
  { name: 'probe_image', description: 'Return a tiny PNG image', inputSchema: { type: 'object', properties: {} } },
  { name: 'probe_link', description: 'Return a resource link', inputSchema: { type: 'object', properties: {} } },
  { name: 'probe_fail', description: 'Always fails', inputSchema: { type: 'object', properties: {} } },
  { name: 'probe_ask', description: 'Ask the user a question (elicitation) and return the answer', inputSchema: { type: 'object', properties: {} } },
];
async function call(name, args) {
  switch (name) {
    case 'probe_echo': return { content: [{ type: 'text', text: `probe says: ${args?.text ?? ''} [mark=${process.env.PROBE_MARK ?? 'none'}]` }] };
    case 'probe_image': return { content: [{ type: 'text', text: 'a 8x8 red square' }, { type: 'image', data: PNG, mimeType: 'image/png' }] };
    case 'probe_link': return { content: [{ type: 'resource_link', uri: 'https://example.com/report.pdf', name: 'report.pdf', mimeType: 'application/pdf', description: 'The probe report' }] };
    case 'probe_fail': return { content: [{ type: 'text', text: 'probe failure on purpose' }], isError: true };
    case 'probe_ask': {
      const id = nextId++;
      const p = new Promise((r) => waiting.set(id, r));
      send({ jsonrpc: '2.0', id, method: 'elicitation/create', params: { message: 'Which colour should the probe use?', requestedSchema: { type: 'object', properties: { colour: { type: 'string', title: 'Colour' } }, required: ['colour'] } } });
      const res = await p;
      return { content: [{ type: 'text', text: 'elicitation answer: ' + JSON.stringify(res) }] };
    }
  }
  return { content: [{ type: 'text', text: 'unknown tool' }], isError: true };
}
let clientCaps = {};
createInterface({ input: process.stdin }).on('line', async (line) => {
  let m; try { m = JSON.parse(line); } catch { return; }
  if (m.id !== undefined && !m.method) { const r = waiting.get(m.id); if (r) { waiting.delete(m.id); r(m.result ?? m.error); } return; }
  if (m.method === 'initialize') { clientCaps = m.params?.capabilities ?? {}; return send({ jsonrpc: '2.0', id: m.id, result: { protocolVersion: m.params?.protocolVersion ?? '2025-06-18', capabilities: { tools: {} }, serverInfo: { name: 'probe', version: '1' } } }); }
  if (m.method === 'tools/list') return send({ jsonrpc: '2.0', id: m.id, result: { tools } });
  if (m.method === 'tools/call') return send({ jsonrpc: '2.0', id: m.id, result: await call(m.params.name, m.params.arguments) });
  if (m.id !== undefined) send({ jsonrpc: '2.0', id: m.id, error: { code: -32601, message: 'no ' + m.method } });
});
