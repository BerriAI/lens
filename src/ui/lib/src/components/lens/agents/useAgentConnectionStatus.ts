"use client";

import { useCallback, useEffect, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useLensAccessToken } from "../data/LensServices";
import { useTracesApi } from "../traces/api";
import { AGENT_WINDOW_DAYS } from "./useAgents";
import {
  agentConnectionState,
  observeAgentConnection,
  RECEIVING_TRACES_WINDOW_MS,
  type AgentConnectionObservation,
} from "./agentConnectionStatus";

const CONNECTION_POLL_MS = 3000;

export function useAgentConnectionStatus(name: string, enabled = true) {
  const accessToken = useLensAccessToken();
  const traces = useTracesApi(accessToken);
  const queryClient = useQueryClient();
  const active = enabled && name.trim().length > 0 && traces.live;
  const [clock, setClock] = useState(Date.now);
  const queryKey = ["agent-connection-status", accessToken, traces.live, name] as const;
  const query = useQuery({
    queryKey,
    queryFn: async () => {
      const previous = queryClient.getQueryData<AgentConnectionObservation>(queryKey);
      const endMs = Date.now();
      const agents = await traces.agents({ startMs: endMs - AGENT_WINDOW_DAYS * 86_400_000, endMs });
      const match = agents.find((agent) => agent.name === name && agent.runs > 0) ?? null;
      const observation = observeAgentConnection(previous, match, Date.now());
      if (observation.observedAt !== null && observation.observedAt !== previous?.observedAt) {
        void queryClient.invalidateQueries({ queryKey: ["lensAgents", accessToken, traces.live] });
      }
      return observation;
    },
    enabled: active,
    retry: false,
    staleTime: 0,
    gcTime: 0,
    refetchInterval: active ? CONNECTION_POLL_MS : false,
    refetchIntervalInBackground: false,
  });
  const observedAt = query.data?.observedAt;
  useEffect(() => {
    if (!active || observedAt == null) return;
    const remaining = observedAt + RECEIVING_TRACES_WINDOW_MS - Date.now();
    if (remaining <= 0) return;
    const timer = window.setTimeout(() => setClock(Date.now()), remaining);
    return () => window.clearTimeout(timer);
  }, [active, observedAt]);
  const refresh = useCallback(() => {
    if (active && !query.isFetching) void query.refetch();
  }, [active, query.isFetching, query.refetch]);
  return {
    status: agentConnectionState(
      active ? query.data : undefined,
      Math.max(clock, Date.now()),
      active ? query.error : null,
    ),
    match: query.data?.match ?? null,
    error: query.error,
    isChecking: active && query.isFetching,
    lastChecked: Math.max(query.dataUpdatedAt, query.errorUpdatedAt) || null,
    refresh,
  };
}
