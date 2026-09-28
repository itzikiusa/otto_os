import {getToken, laneFetch} from '../../lib/api/client';
/** Owner-only archive transport. Never use room credentials for these endpoints. */
export async function recapRequest<T>(path: string, body?: unknown, signal?: AbortSignal, method = body === undefined ? 'GET' : 'POST'): Promise<T> {
  const token = getToken();
  if (!token) throw new Error('Sign in as the room owner to access recaps.');
  const controller = new AbortController();
  const abort = () => controller.abort();
  if (signal?.aborted) controller.abort();
  signal?.addEventListener('abort', abort, {once: true});
  const timer = setTimeout(abort, 20000);
  try {
  const response = await laneFetch('long', path, {method, signal: controller.signal, headers: {Authorization: `Bearer ${token}`, ...(body === undefined ? {} : {'Content-Type': 'application/json'})}, body: body === undefined ? undefined : JSON.stringify(body)});
  if (!response.ok) { const problem = await response.json().catch(() => ({})) as {message?: string}; throw new Error(problem.message ?? 'The recap request failed. Try again.'); }
  if (response.status === 204) return undefined as T;
  return await response.json() as T;
  } finally { clearTimeout(timer); signal?.removeEventListener('abort', abort); }
}
export async function recapBlob(path: string): Promise<Blob> {
  const token = getToken(); if (!token) throw new Error('Sign in as the room owner to access recaps.');
  const response = await laneFetch('long', path, {headers: {Authorization: `Bearer ${token}`}});
  if (!response.ok) throw new Error('Could not load the recap file. Try again.');
  return response.blob();
}
