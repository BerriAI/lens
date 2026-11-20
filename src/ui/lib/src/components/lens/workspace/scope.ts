import type { TraceFilters } from "../traces/types";

export type WorkspaceKind = "organization" | "team" | "project" | "key";

export interface WorkspaceItem {
  readonly id: string;
  readonly name: string;
  readonly organization_id?: string | null;
  readonly team_id?: string | null;
  readonly project_id?: string | null;
  readonly description?: string;
}

export function workspaceScopeAfterSelect(
  scope: TraceFilters,
  kind: WorkspaceKind,
  item: WorkspaceItem | null,
) {
  const next = {
    organization_id: scope.organization_id ?? null,
    team_id: scope.team_id ?? null,
    project_id: scope.project_id ?? null,
    api_key_hash: null as string | null,
  };
  if (kind === "organization") {
    next.organization_id = item?.id ?? null;
    next.team_id = null;
    next.project_id = null;
    return next;
  }
  if (item?.organization_id !== undefined)
    next.organization_id = item.organization_id;
  if (kind === "team") {
    next.team_id = item?.id ?? null;
    next.project_id = null;
    return next;
  }
  if (item?.team_id !== undefined) next.team_id = item.team_id;
  if (kind === "project") {
    next.project_id = item?.id ?? null;
    return next;
  }
  if (item?.project_id !== undefined) next.project_id = item.project_id;
  next.api_key_hash = item?.id ?? null;
  return next;
}
