import { useQuery } from "@tanstack/react-query";
import { apiClient } from "../../../../lib/http/requests";
import type { components } from "../../../../lib/http/schema";

export type TracingKeyRecord = components["schemas"]["IngestionKey"];
export const tracingKeysQuery = (accessToken: string) =>
  ["lens-tracing-keys", accessToken] as const;
export const maskSecret = (secret: string): string =>
  secret.length > 10
    ? `${secret.slice(0, 5)}${"•".repeat(16)}${secret.slice(-4)}`
    : "•".repeat(secret.length);

export function useTracingKeys(accessToken: string, enabled: boolean) {
  return useQuery({
    queryKey: tracingKeysQuery(accessToken),
    queryFn: async () => {
      const keys = await apiClient.get<TracingKeyRecord[]>(
        "/lens/tracing/keys",
        { accessToken },
      );
      if (!Array.isArray(keys))
        throw new Error("Lens returned an invalid tracing key list");
      return keys;
    },
    enabled,
    retry: false,
  });
}
