import { z } from "zod";
import { apiClient } from "../../../lib/http/requests";
import type { TracesApi } from "../traces/api";
import type { TraceQueryBody } from "../traces/types";

export const connectionProgressSchema = z.object({
  keyId: z.string().nullable(),
  startedAt: z.number().int().nonnegative().nullable(),
  existing: z.boolean(),
});
export type ConnectionProgress = z.infer<typeof connectionProgressSchema>;
export const emptyConnection: ConnectionProgress = {
  keyId: null,
  startedAt: null,
  existing: false,
};

export function restoreConnection(value: string): ConnectionProgress {
  try {
    return connectionProgressSchema.parse(JSON.parse(value));
  } catch {
    return emptyConnection;
  }
}

export function connectionTraceSql(
  keyHash: string,
  startedAt: number,
): string | null {
  if (
    !/^[a-f0-9]{64}$/i.test(keyHash) ||
    !Number.isSafeInteger(startedAt) ||
    startedAt < 0
  )
    return null;
  return `SELECT TraceId AS trace_id,
hex(SHA256(concat(TeamId, char(0), ApiKeyHash, char(0), TraceId))) AS trace_ref
FROM agent_traces_by_key
WHERE ApiKeyHash = '${keyHash}'
GROUP BY TeamId, ApiKeyHash, TraceId
HAVING min(StartTs) >= fromUnixTimestamp64Milli(${startedAt})
ORDER BY min(StartTs) ASC
LIMIT 1`;
}

const traceResult = z.object({
  data: z.array(
    z.object({
      trace_id: z.string().regex(/^[a-f0-9]{32}$/i),
      trace_ref: z.string().regex(/^[a-f0-9]{64}$/i),
    }),
  ),
});

export async function findConnectionTrace(
  accessToken: string,
  traces: TracesApi,
  keyHash: string,
  startedAt: number,
  signal: AbortSignal,
) {
  const sql = connectionTraceSql(keyHash, startedAt);
  if (!sql) throw new Error("Select a valid tracing key to check delivery.");
  const response = await apiClient.post<unknown>("/v1/traces/query", {
    accessToken,
    body: { sql } satisfies TraceQueryBody,
    signal,
  });
  const match = traceResult.parse(response).data[0];
  if (!match) return null;
  const trace = await traces.trace(match.trace_id, match.trace_ref);
  return trace.summary;
}
