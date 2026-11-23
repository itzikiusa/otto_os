// Media API client — 3D Generation Providers & Jobs (`/api/v1/design/3d/*`),
// Workspace-confined 3D/Audio Export, and Built-in/Extensible TTS (`/api/v1/product/tts/*`).
// Mirrors crates/otto-media/src/http.rs and ./types/media.ts.

import { api, ApiError, baseUrl, getToken } from './client';
import { qs } from './design';
import type {
  CreateMedia3dProviderReq,
  CreateTtsProviderReq,
  ExportWorkspaceReq,
  ExportWorkspaceResp,
  GameAssetPreset,
  Id,
  Media3dJob,
  Media3dProvider,
  Problem,
  ProviderTestResult,
  StartMedia3dJobReq,
  SynthesizeTtsReq,
  AudioEngine,
  AudioJob,
  AudioKind,
  StartAudioJobReq,
  MediaClipKind,
  TtsClip,
  TtsProvider,
  TtsVoice,
  UpdateMedia3dProviderReq,
  UpdateTtsProviderReq,
} from './types';

const enc = encodeURIComponent;

// ── 3D Generation Providers & Presets ───────────────────────────────────────

export function list3dProviders() {
  return api.get<Media3dProvider[]>('/design/3d/providers');
}

export function create3dProvider(body: CreateMedia3dProviderReq) {
  return api.post<Media3dProvider>('/design/3d/providers', body);
}

export function update3dProvider(id: Id, body: UpdateMedia3dProviderReq) {
  return api.patch<Media3dProvider>(`/design/3d/providers/${enc(id)}`, body);
}

export function delete3dProvider(id: Id) {
  return api.del<void>(`/design/3d/providers/${enc(id)}`);
}

export function test3dProvider(id: Id) {
  return api.post<ProviderTestResult>(`/design/3d/providers/${enc(id)}/test`, {});
}

export function list3dPresets() {
  return api.get<GameAssetPreset[]>('/design/3d/presets');
}

// ── 3D Generation Jobs & Workspace Export ───────────────────────────────────

export function start3dJob(body: StartMedia3dJobReq) {
  return api.post<Media3dJob>('/design/3d/jobs', body);
}

export function list3dJobs(p: { workspace_id?: Id; artifact_id?: Id; limit?: number } = {}) {
  return api.get<Media3dJob[]>(`/design/3d/jobs${qs({ ...p })}`);
}

export function get3dJob(id: Id) {
  return api.get<Media3dJob>(`/design/3d/jobs/${enc(id)}`);
}

export function cancel3dJob(id: Id) {
  return api.post<Media3dJob>(`/design/3d/jobs/${enc(id)}/cancel`, {});
}

export function export3dArtifactToWorkspace(artifactId: Id, body: ExportWorkspaceReq) {
  return api.post<ExportWorkspaceResp>(
    `/design/3d/artifacts/${enc(artifactId)}/export-workspace`,
    body,
  );
}

// ── TTS Providers, Voices, Synthesis & Clips ────────────────────────────────

export function listTtsProviders() {
  return api.get<TtsProvider[]>('/product/tts/providers');
}

export function createTtsProvider(body: CreateTtsProviderReq) {
  return api.post<TtsProvider>('/product/tts/providers', body);
}

export function updateTtsProvider(id: Id, body: UpdateTtsProviderReq) {
  return api.patch<TtsProvider>(`/product/tts/providers/${enc(id)}`, body);
}

export function deleteTtsProvider(id: Id) {
  return api.del<void>(`/product/tts/providers/${enc(id)}`);
}

export function testTtsProvider(id: Id) {
  return api.post<ProviderTestResult>(`/product/tts/providers/${enc(id)}/test`, {});
}

export function listTtsVoices(providerId: Id) {
  return api.get<TtsVoice[]>(`/product/tts/providers/${enc(providerId)}/voices`);
}

export function synthesizeTts(body: SynthesizeTtsReq) {
  return api.post<TtsClip>('/product/tts/synthesize', body);
}

export function listTtsClips(p: {
  workspace_id: Id;
  story_id?: Id;
  artifact_id?: Id;
  kind?: MediaClipKind;
  limit?: number;
}) {
  return api.get<TtsClip[]>(`/product/tts/clips${qs({ ...p })}`);
}

export function getTtsClip(id: Id) {
  return api.get<TtsClip>(`/product/tts/clips/${enc(id)}`);
}

export function deleteTtsClip(id: Id) {
  return api.del<void>(`/product/tts/clips/${enc(id)}`);
}

export function exportTtsClipToWorkspace(clipId: Id, body: ExportWorkspaceReq) {
  return api.post<ExportWorkspaceResp>(
    `/product/tts/clips/${enc(clipId)}/export-workspace`,
    body,
  );
}

// ── Sounds and music (Otto Forge audio stage) ─────────────────────────────────

export function getAudioEngine() {
  return api.get<AudioEngine>('/product/audio/engine');
}

export function startAudioJob(body: StartAudioJobReq) {
  return api.post<AudioJob>('/product/audio/jobs', body);
}

export function listAudioJobs(p: { workspace_id: Id; kind?: AudioKind; limit?: number }) {
  return api.get<AudioJob[]>(`/product/audio/jobs${qs({ ...p })}`);
}

export function getAudioJob(id: Id) {
  return api.get<AudioJob>(`/product/audio/jobs/${enc(id)}`);
}

export function cancelAudioJob(id: Id) {
  return api.post<AudioJob>(`/product/audio/jobs/${enc(id)}/cancel`, {});
}

/** Fetch the authenticated raw audio bytes of a TTS clip. */
export async function fetchTtsClipAudioBlob(clipId: Id, signal?: AbortSignal): Promise<Blob> {
  const token = getToken();
  const headers: Record<string, string> = token ? { Authorization: `Bearer ${token}` } : {};
  const resp = await fetch(`${baseUrl()}/api/v1/product/tts/clips/${enc(clipId)}/audio`, {
    headers,
    signal,
  });
  if (!resp.ok) {
    let problem: Problem = { code: 'internal', message: resp.statusText };
    try {
      problem = await resp.json();
    } catch {
      /* non-JSON error body */
    }
    throw new ApiError(resp.status, problem);
  }
  return await resp.blob();
}

/** Fetch an authenticated object URL for a TTS clip (caller revokes via `URL.revokeObjectURL`). */
export async function fetchTtsClipAudioUrl(clipId: Id, signal?: AbortSignal): Promise<string> {
  const blob = await fetchTtsClipAudioBlob(clipId, signal);
  return URL.createObjectURL(blob);
}
