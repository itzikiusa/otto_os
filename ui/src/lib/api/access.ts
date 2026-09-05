import { api } from './client';
import type { AccessPolicy, AccessPreview, AccessGroup, AccessRole, AccessSubjects, EffectiveAccess, Id, ResourceKind } from './types';
const resourcePath = (kind: ResourceKind, id: Id) => `/access/${kind}/${encodeURIComponent(id)}`;
const childQuery = (child?: string) => child === undefined ? '' : `?child=${encodeURIComponent(child)}`;
export const accessApi = {
  policy: (kind: ResourceKind, id: Id) => api.get<AccessPolicy>(resourcePath(kind, id)),
  save: (policy: AccessPolicy, preview_token?: string) => api.put<AccessPolicy>(resourcePath(policy.kind, policy.resource_id), {policy, preview_token}),
  preview: (policy: AccessPolicy) => api.post<AccessPreview>(`${resourcePath(policy.kind, policy.resource_id)}/preview`, {policy}),
  capabilities: (kind: ResourceKind, id: Id, child?: string) => api.get<EffectiveAccess>(`${resourcePath(kind, id)}/capabilities${childQuery(child)}`),
  effective: (kind: ResourceKind, id: Id, userId: Id, child?: string) => api.get<EffectiveAccess>(`${resourcePath(kind, id)}/effective?user_id=${encodeURIComponent(userId)}${child === undefined ? '' : `&child=${encodeURIComponent(child)}`}`),
  subjects: (kind: ResourceKind, id: Id) => api.get<AccessSubjects>(`${resourcePath(kind, id)}/subjects`),
  groups: () => api.get<AccessGroup[]>('/access/groups'),
  createGroup: (name: string, description?: string) => api.post<AccessGroup>('/access/groups', {name, description}),
  updateGroup: (id: Id, name: string, description?: string) => api.put<AccessGroup>(`/access/groups/${encodeURIComponent(id)}`, {name, description}),
  deleteGroup: (id: Id) => api.del<void>(`/access/groups/${encodeURIComponent(id)}`),
  members: (id: Id) => api.get<Id[]>(`/access/groups/${encodeURIComponent(id)}/members`),
  addMember: (id: Id, userId: Id) => api.put<void>(`/access/groups/${encodeURIComponent(id)}/members/${encodeURIComponent(userId)}`, {}),
  removeMember: (id: Id, userId: Id) => api.del<void>(`/access/groups/${encodeURIComponent(id)}/members/${encodeURIComponent(userId)}`),
  roles: () => api.get<AccessRole[]>('/access/roles'),
  createRole: (role: Pick<AccessRole,'name'|'description'|'kind'|'operations'|'grantable_operations'>) => api.post<AccessRole>('/access/roles', role),
  updateRole: (id: Id, role: Pick<AccessRole,'name'|'description'|'kind'|'operations'|'grantable_operations'>) => api.put<AccessRole>(`/access/roles/${encodeURIComponent(id)}`, role),
  deleteRole: (id: Id) => api.del<void>(`/access/roles/${encodeURIComponent(id)}`),
};
