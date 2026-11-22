"use client";

import { useTracesLive } from "../traces/api";
import { useAgents } from "./useAgents";
import { useAgentSelection } from "./useAgentSelection";

export interface LensAgents {
  readonly agent: string | null;
  readonly all: boolean;
  select(agent: string): void;
  readonly list: ReturnType<typeof useAgents>;
}

export function useLensAgents(accessToken: string): LensAgents {
  const list = useAgents(accessToken);
  const { agent, all, select } = useAgentSelection(
    !useTracesLive(),
    list.agents.map((item) => item.name),
  );
  return { agent, all, select, list };
}
