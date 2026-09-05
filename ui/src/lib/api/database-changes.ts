import { api } from './client';
import type { ChangeInput, DatabaseChange, ChangeDetail } from './types';
export type { ChangeTarget, ChangeInput, DatabaseChange, ChangeAttempt, ChangeEvent, ChangeDetail } from './types';
const path=(id:string)=>`/database-changes/${encodeURIComponent(id)}`;
export const databaseChangesApi = {
  list:(connectionId:string,before?:string)=>api.get<DatabaseChange[]>(`/database-changes?connection_id=${encodeURIComponent(connectionId)}${before ? `&before=${encodeURIComponent(before)}` : ''}`),
  get:(id:string)=>api.get<ChangeDetail>(path(id)),
  create:(input:ChangeInput)=>api.post<DatabaseChange>('/database-changes',input),
  revise:(id:string,revision:number,input:ChangeInput)=>api.put<DatabaseChange>(path(id),{revision,...input}),
  executors:(id:string)=>api.get<{id:string;display_name:string;username:string}[]>(`${path(id)}/executors`),
  validate:(id:string,revision:number,executor_id:string)=>api.post<DatabaseChange>(`${path(id)}/validate`,{revision,executor_id}),
  action:(id:string,revision:number,action:'submit'|'approve'|'reject'|'execute'|'cancel',note='')=>api.post<DatabaseChange>(`${path(id)}/${action}`,{revision,note}),
  reconcile:(id:string,revision:number,attempt_id:string,outcome:'succeeded'|'failed'|'partially_applied',note:string)=>api.post<DatabaseChange>(`${path(id)}/reconcile`,{revision,attempt_id,outcome,note}),
};
