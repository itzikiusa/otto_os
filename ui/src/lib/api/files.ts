// File transfer on the long request lane: server-rendered downloads and raw
// uploads (the Database Explorer's export / import of whole files).
import { rawRequest } from './client';

/** POST JSON and return the response body as a Blob — a server-rendered
 *  file download (the DB export streams uncapped results). */
export async function postForBlob(path: string, body: unknown, signal?: AbortSignal): Promise<Blob> {
  const resp = await rawRequest('long', path, { method: 'POST', body: JSON.stringify(body), contentType: 'application/json', signal });
  return await resp.blob();
}

/** Upload a file as the raw request body (an import can run for minutes) and
 *  read the JSON answer. */
export async function postFile<T>(path: string, file: Blob, contentType: string, signal?: AbortSignal): Promise<T> {
  const resp = await rawRequest('long', path, { method: 'POST', body: file, contentType, signal });
  return (await resp.json()) as T;
}
