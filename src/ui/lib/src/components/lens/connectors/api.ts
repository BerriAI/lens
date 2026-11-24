import { apiClient } from "../../../lib/http/requests";
import type {
  McpConnectionCreated,
  McpCreateRequest,
  McpOverview,
} from "../../../lib/http/worker";

export const connectorOverview = (
  accessToken: string,
  hours: number,
  connection?: string | null,
  cursor?: string | null,
  toMs?: number,
  signal?: AbortSignal,
) =>
  apiClient.get<McpOverview>("/lens/connectors/mcp", {
    accessToken,
    signal,
    cache: "no-store",
    query: { hours, connection, cursor, to_ms: toMs },
  });
export const createConnection = (accessToken: string, body: McpCreateRequest) =>
  apiClient.post<McpConnectionCreated>("/lens/connectors/mcp", {
    accessToken,
    body,
    cache: "no-store",
  });
export const revokeConnection = (accessToken: string, id: string) =>
  apiClient.delete<boolean>(`/lens/connectors/mcp/${encodeURIComponent(id)}`, {
    accessToken,
  });
