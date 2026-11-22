"use client";

import { parseAsString, useQueryStates } from "nuqs";
import { useCallback, useEffect } from "react";
import { useLocalStorage } from "usehooks-ts";
import { useLensRoute } from "../route";
import { useLensHost } from "../../../host/LensHost";
import { ALL_AGENTS } from "../traces/routing";

const SELECTED_AGENT_KEY = "litellm.lens.agent";
export const selectedAgentKey = (demo: boolean): string => (demo ? `${SELECTED_AGENT_KEY}.demo` : SELECTED_AGENT_KEY);

const AGENT_PARSERS = { agent: parseAsString.withDefault("") };

/**
 * A shared link's choice first, then this browser's last pick if it still exists, then the most recently active
 * agent. `ALL_AGENTS` is a choice like any agent name once there is an agent to choose between.
 */
export function resolveAgent(fromUrl: string, remembered: string, available: readonly string[]): string | null {
  if (fromUrl) return fromUrl;
  if (available.length > 0 && (remembered === ALL_AGENTS || available.includes(remembered))) return remembered;
  return available[0] ?? null;
}

export interface AgentSelection {
  readonly agent: string | null;
  readonly all: boolean;
  select(agent: string): void;
}

/** In the URL for sharing, and remembered per browser (separately for the sample session) across refresh and login. */
export function useAgentSelection(demo: boolean, available: readonly string[]): AgentSelection {
  const [{ agent: fromUrl }, setParams] = useQueryStates(AGENT_PARSERS, { history: "push" });
  const { tab, defaultTab, openAgent } = useLensRoute();
  const embedded = useLensHost().surface === "embedded";
  const [remembered, setRemembered] = useLocalStorage(selectedAgentKey(demo), "", {
    serializer: (value) => value,
    deserializer: (raw) => raw,
  });
  const choice = resolveAgent(fromUrl, remembered, available);
  const all = choice === ALL_AGENTS;
  const agent = all ? null : choice;
  const effectiveTab = tab ?? (embedded && defaultTab === "home" ? "traces" : defaultTab);
  const implied = !fromUrl && effectiveTab === "traces" ? choice : null;
  useEffect(() => {
    if (implied) void setParams({ agent: implied }, { history: "replace" });
  }, [implied, setParams]);
  const select = useCallback(
    (next: string) => {
      setRemembered(next);
      openAgent(next);
    },
    [openAgent, setRemembered],
  );
  return { agent, all, select };
}
