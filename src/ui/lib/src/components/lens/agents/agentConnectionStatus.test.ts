import { describe, expect, it } from "vitest";
import { agentConnectionState, observeAgentConnection, RECEIVING_TRACES_WINDOW_MS } from "./agentConnectionStatus";
import type { AgentSummary } from "./agentRollup";

const now = Date.parse("2026-10-09T12:00:00Z");
const agent: AgentSummary = {
  name: "moyai",
  runs: 1,
  failed_runs: 0,
  frameworks: [],
  last_seen: new Date(now).toISOString(),
};

describe("agent connection evidence", () => {
  it.each([now, now - 7 * 86_400_000, now + 86_400_000])(
    "should treat the initial matching row as historical even when its timestamp is %s",
    (startedAt) => {
      const baseline = observeAgentConnection(
        undefined,
        { ...agent, last_seen: new Date(startedAt).toISOString() },
        now,
      );
      expect(agentConnectionState(baseline, now)).toBe("waiting-for-new-traces");
    },
  );

  it("should recognize a first real row after an empty baseline and expire without new evidence", () => {
    const baseline = observeAgentConnection(undefined, null, now);
    expect(agentConnectionState(baseline, now)).toBe("waiting");
    const received = observeAgentConnection(baseline, agent, now + 3000);
    expect(agentConnectionState(received, now + 3000)).toBe("receiving");
    const unchanged = observeAgentConnection(received, agent, now + 30_000);
    expect(agentConnectionState(unchanged, now + 3000 + RECEIVING_TRACES_WINDOW_MS)).toBe("waiting-for-new-traces");
  });

  it.each([
    { ...agent, runs: 2 },
    { ...agent, last_seen: new Date(now + 1000).toISOString() },
  ])("should recognize additional runs or a newer run timestamp", (next) => {
    const baseline = observeAgentConnection(undefined, agent, now);
    expect(agentConnectionState(observeAgentConnection(baseline, next, now + 3000), now + 3000)).toBe("receiving");
  });

  it("should prioritize check errors and never count a shrinking rollup as progress", () => {
    const baseline = observeAgentConnection(undefined, { ...agent, runs: 3 }, now);
    const unchanged = observeAgentConnection(baseline, agent, now + 3000);
    expect(agentConnectionState(unchanged, now + 3000)).toBe("waiting-for-new-traces");
    const received = observeAgentConnection(unchanged, { ...agent, runs: 2 }, now + 6000);
    expect(agentConnectionState(received, now + 6000, new Error("Unavailable"))).toBe("error");
  });
});
