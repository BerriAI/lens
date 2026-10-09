import type { AgentSummary } from "./agentRollup";

export const RECEIVING_TRACES_WINDOW_MS = 60_000;

export type AgentConnectionState = "waiting" | "receiving" | "waiting-for-new-traces" | "error";

export interface AgentConnectionObservation {
  readonly match: AgentSummary | null;
  readonly checkedAt: number;
  readonly observedAt: number | null;
}

export function observeAgentConnection(
  previous: AgentConnectionObservation | undefined,
  match: AgentSummary | null,
  checkedAt: number,
): AgentConnectionObservation {
  const progressed =
    previous !== undefined &&
    match !== null &&
    (previous.match === null ||
      match.runs > previous.match.runs ||
      Date.parse(match.last_seen) > Date.parse(previous.match.last_seen));
  return {
    match,
    checkedAt,
    observedAt: match === null ? null : progressed ? checkedAt : previous?.observedAt ?? null,
  };
}

export function discoverAgentConnection(
  previous: AgentConnectionObservation | undefined,
  agents: readonly AgentSummary[],
  startedAfter: number,
  checkedAt: number,
): AgentConnectionObservation {
  const eligible = agents.filter((agent) => {
    const startedAt = Date.parse(agent.last_seen);
    return agent.name.trim().length > 0 && agent.runs > 0 && startedAt >= startedAfter && startedAt <= checkedAt;
  });
  const match = previous?.match
    ? eligible.find((agent) => agent.name === previous.match?.name) ?? previous.match
    : eligible[0] ?? null;
  return observeAgentConnection(previous ?? { match: null, checkedAt, observedAt: null }, match, checkedAt);
}

export function agentConnectionState(
  observation: AgentConnectionObservation | undefined,
  now: number,
  error: Error | null = null,
): AgentConnectionState {
  if (error) return "error";
  if (!observation?.match) return "waiting";
  const age = observation.observedAt === null ? Infinity : now - observation.observedAt;
  return age >= 0 && age < RECEIVING_TRACES_WINDOW_MS ? "receiving" : "waiting-for-new-traces";
}
