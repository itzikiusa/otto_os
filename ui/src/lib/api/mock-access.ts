// Mock-mode resource access (`/access/<kind>/…/capabilities`): every decision
// is `legacy`, so the per-resource gates fall back to the mock user's feature
// grants (admin in mock mode) instead of reading "No access" for lack of a route.
import type { EffectiveAccess } from './types';

type Out = { status?: number; json?: unknown } | undefined;
type MockRoute = { method: string; re: RegExp; handle: (m: RegExpMatchArray, body: unknown, q: URLSearchParams) => Out };

const legacy = (kind: string, id: string, child: string | null = null) =>
  ({ kind, resource_id: id, user_id: 'usr_root', child, mode: 'legacy', operations: {} }) as EffectiveAccess;

export const accessMockRoutes: MockRoute[] = [
  { method: 'POST', re: /^\/access\/([^/]+)\/capabilities\/batch$/, handle: (m, b) => ({ json: ((b as { ids?: string[] })?.ids ?? []).map((id) => legacy(m[1], id)) }) },
  { method: 'POST', re: /^\/access\/([^/]+)\/([^/]+)\/capabilities\/batch$/, handle: (m, b) => ({ json: ((b as { children?: string[] })?.children ?? []).map((c) => legacy(m[1], decodeURIComponent(m[2]), c)) }) },
  { method: 'GET', re: /^\/access\/([^/]+)\/([^/]+)\/capabilities$/, handle: (m, _b, q) => ({ json: legacy(m[1], decodeURIComponent(m[2]), q.get('child')) }) },
];
