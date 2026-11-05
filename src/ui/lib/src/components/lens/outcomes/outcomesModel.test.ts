import { describe, expect, it } from "vitest";
import {
  agentOutcomes,
  createOutcomeDemo,
  dailyOutcomeTrend,
  dailyAgentOutcomeTrend,
  filterOutcomeSessions,
  outcomeStatus,
  repositoryOutcomes,
  summarizeOutcomes,
  type OutcomeSession,
} from "./outcomesModel";

const NOW = Date.parse("2026-11-05T12:00:00Z");
const HOUR_MS = 3_600_000;

const session = (overrides: Partial<OutcomeSession> = {}): OutcomeSession => ({
  id: "session-1",
  agent: "release_agent",
  repository: "acme/platform",
  title: "Fix webhook retries",
  startedAt: new Date(NOW - 2 * HOUR_MS).toISOString(),
  finishedAt: new Date(NOW - 1.5 * HOUR_MS).toISOString(),
  costUSD: 10,
  pullRequest: {
    id: 42,
    title: "Fix webhook retries",
    status: "merged",
    mergedAt: new Date(NOW - HOUR_MS).toISOString(),
  },
  issue: {
    id: 12,
    title: "Webhook retry failures",
    status: "closed",
    closedAt: new Date(NOW - HOUR_MS).toISOString(),
  },
  evidence: {
    method: "explicit",
    source: "GitHub webhook",
    description: "Recorded artifact identifiers",
  },
  events: [],
  ...overrides,
});

describe("observed delivery outcomes", () => {
  it("counts one artifact once across sessions and keeps same-number artifacts in different repositories distinct", () => {
    const rows = [
      session({ costUSD: 12 }),
      session({ id: "follow-up", costUSD: 8 }),
      session({
        id: "another-repository",
        repository: "acme/console",
        costUSD: 20,
      }),
    ];
    expect(summarizeOutcomes(rows)).toEqual({
      sessions: 3,
      totalCostUSD: 40,
      mergedPRs: 2,
      closedIssues: 2,
      linkedSessions: 3,
      coverage: 1,
      costPerMergedPR: 20,
    });
  });

  it("includes unlinked spend in the cost denominator without treating missing links as failures", () => {
    const unlinked = session({
      id: "unknown",
      pullRequest: null,
      issue: null,
      evidence: null,
      costUSD: 6,
    });
    expect(outcomeStatus(unlinked)).toBe("unlinked");
    expect(summarizeOutcomes([session(), unlinked])).toEqual({
      sessions: 2,
      totalCostUSD: 16,
      mergedPRs: 1,
      closedIssues: 1,
      linkedSessions: 1,
      coverage: 0.5,
      costPerMergedPR: 16,
    });
  });

  it("leaves cost per merged PR unavailable when none merged", () => {
    const row = session({
      pullRequest: { id: 42, title: "Draft fix", status: "open" },
      issue: null,
    });
    expect(summarizeOutcomes([row]).costPerMergedPR).toBeNull();
    expect(summarizeOutcomes([])).toEqual({
      sessions: 0,
      totalCostUSD: 0,
      mergedPRs: 0,
      closedIssues: 0,
      linkedSessions: 0,
      coverage: 0,
      costPerMergedPR: null,
    });
  });

  it.each([
    ["merged", "shipped"],
    ["open", "in_progress"],
    ["closed", "closed_without_merge"],
  ] as const)(
    "classifies a %s PR without conflating closed and merged",
    (status, expected) => {
      expect(
        outcomeStatus(
          session({
            pullRequest: { id: 42, title: "Fix", status },
            issue: null,
          }),
        ),
      ).toBe(expected);
    },
  );

  it("treats a closed issue as a shipped outcome even when its linked PR remains open", () => {
    expect(
      outcomeStatus(
        session({ pullRequest: { id: 42, title: "Fix", status: "open" } }),
      ),
    ).toBe("shipped");
  });
});

describe("outcome cohort filters", () => {
  it("keeps agent, repository and time filters intersected, with an inclusive start and exclusive end", () => {
    const rows = [
      session({
        id: "match",
        startedAt: new Date(NOW - 7 * 24 * HOUR_MS).toISOString(),
      }),
      session({
        id: "too-old",
        startedAt: new Date(NOW - 7 * 24 * HOUR_MS - 1).toISOString(),
      }),
      session({ id: "future", startedAt: new Date(NOW).toISOString() }),
      session({ id: "other-agent", agent: "research_agent" }),
      session({ id: "other-repository", repository: "acme/console" }),
    ];
    expect(
      filterOutcomeSessions(
        rows,
        {
          agent: "release_agent",
          repository: "acme/platform",
          days: 7,
          status: "all",
        },
        NOW,
      ).map((row) => row.id),
    ).toEqual(["match"]);
  });

  it.each(["shipped", "in_progress", "unlinked"] as const)(
    "filters %s after agent and date scoping",
    (status) => {
      const rows = [
        session({ id: "shipped" }),
        session({
          id: "in_progress",
          pullRequest: { id: 43, title: "Draft", status: "open" },
          issue: null,
        }),
        session({
          id: "closed",
          pullRequest: { id: 44, title: "Abandoned", status: "closed" },
          issue: null,
        }),
        session({
          id: "unlinked",
          pullRequest: null,
          issue: null,
          evidence: null,
        }),
      ];
      expect(
        filterOutcomeSessions(rows, { days: 7, status }, NOW).map(
          (row) => row.id,
        ),
      ).toEqual([status]);
    },
  );
});

describe("delivery trend and repository rollups", () => {
  it("attributes shared artifacts once across agents while retaining each agent's actual spend", () => {
    const rows = [
      session({
        id: "first",
        startedAt: new Date(NOW - 3 * HOUR_MS).toISOString(),
      }),
      session({ id: "follow-up", agent: "research_agent", costUSD: 20 }),
    ];
    const agents = agentOutcomes(rows);
    expect(
      agents.find((agent) => agent.agent === "research_agent"),
    ).toMatchObject({
      mergedPRs: 0,
      closedIssues: 0,
      totalCostUSD: 20,
      costPerMergedPR: null,
    });
    expect(
      agents.find((agent) => agent.agent === "release_agent"),
    ).toMatchObject({
      mergedPRs: 1,
      closedIssues: 1,
      totalCostUSD: 10,
      costPerMergedPR: 10,
    });
    expect(
      dailyAgentOutcomeTrend(rows, 7, NOW, "mergedPRs").at(-1)?.values,
    ).toEqual({ release_agent: 1, research_agent: 0 });
    expect(
      dailyAgentOutcomeTrend(rows, 7, NOW, "closedIssues").at(-1)?.values,
    ).toEqual({ release_agent: 1, research_agent: 0 });
    expect(
      dailyAgentOutcomeTrend(rows, 7, NOW, "costUSD").at(-1)?.values,
    ).toEqual({ release_agent: 10, research_agent: 20 });
  });
  it("counts merge and close events once on their event date, independently of session date", () => {
    const rows = [
      session({ startedAt: new Date(NOW - 26 * HOUR_MS).toISOString() }),
      session({ id: "follow-up" }),
    ];
    const trend = dailyOutcomeTrend(rows, 7, NOW);
    expect(trend).toHaveLength(8);
    expect(trend.at(-1)).toEqual({
      date: "2026-11-05",
      sessions: 1,
      mergedPRs: 1,
      closedIssues: 1,
      costUSD: 10,
    });
    expect(trend.at(-2)).toEqual({
      date: "2026-11-04",
      sessions: 1,
      mergedPRs: 0,
      closedIssues: 0,
      costUSD: 10,
    });
  });

  it.each([
    [
      "partial boundary days",
      "2026-11-05T12:00:00Z",
      8,
      "2026-10-29",
      "2026-11-05",
    ],
    ["midnight end", "2026-11-05T00:00:00Z", 7, "2026-10-29", "2026-11-04"],
  ] as const)(
    "keeps rolling-period spend and merge totals consistent for %s",
    (_name, end, buckets, firstDate, lastDate) => {
      const now = Date.parse(end);
      const start = now - 7 * 24 * HOUR_MS;
      const rows = filterOutcomeSessions(
        [
          session({
            id: "first-partial-day",
            startedAt: new Date(start + HOUR_MS).toISOString(),
            finishedAt: new Date(start + 1.5 * HOUR_MS).toISOString(),
            costUSD: 12,
            pullRequest: {
              id: 7,
              title: "First-day fix",
              status: "merged",
              mergedAt: new Date(start + 2 * HOUR_MS).toISOString(),
            },
            issue: null,
          }),
          session({
            id: "last-day",
            startedAt: new Date(now - 3 * HOUR_MS).toISOString(),
            finishedAt: new Date(now - 2.5 * HOUR_MS).toISOString(),
            costUSD: 8,
            pullRequest: {
              id: 8,
              title: "Last-day fix",
              status: "merged",
              mergedAt: new Date(now - HOUR_MS).toISOString(),
            },
            issue: null,
          }),
        ],
        { days: 7, status: "all" },
        now,
      );
      const summary = summarizeOutcomes(rows);
      const daily = dailyOutcomeTrend(rows, 7, now);
      expect(daily).toHaveLength(buckets);
      expect(daily[0].date).toBe(firstDate);
      expect(daily.at(-1)?.date).toBe(lastDate);
      expect(daily[0]).toMatchObject({
        sessions: 1,
        costUSD: 12,
        mergedPRs: 1,
      });
      expect(daily.reduce((total, day) => total + day.costUSD, 0)).toBe(
        summary.totalCostUSD,
      );
      expect(daily.reduce((total, day) => total + day.mergedPRs, 0)).toBe(
        summary.mergedPRs,
      );
      const spend = dailyAgentOutcomeTrend(rows, 7, now, "costUSD");
      const merges = dailyAgentOutcomeTrend(rows, 7, now, "mergedPRs");
      expect(spend[0].values.release_agent).toBe(12);
      expect(merges[0].values.release_agent).toBe(1);
      expect(
        spend
          .flatMap((day) => Object.values(day.values))
          .reduce((total, value) => total + value, 0),
      ).toBe(summary.totalCostUSD);
      expect(
        merges
          .flatMap((day) => Object.values(day.values))
          .reduce((total, value) => total + value, 0),
      ).toBe(summary.mergedPRs);
    },
  );

  it("ranks repositories by spend and derives shares against all filtered spend", () => {
    const rows = [
      session(),
      session({ id: "follow-up" }),
      session({ repository: "acme/console", costUSD: 25 }),
    ];
    const repos = repositoryOutcomes(rows);
    expect(repos.map((repo) => repo.repository)).toEqual([
      "acme/console",
      "acme/platform",
    ]);
    expect(repos[0].spendShare).toBeCloseTo(25 / 45);
    expect(repos[1].mergedPRs).toBe(1);
    expect(repos[1].sessions).toBe(2);
  });

  it("creates repeatable sample cohorts with valid timestamps across date ranges", () => {
    const rows = createOutcomeDemo(NOW);
    expect(rows).toEqual(createOutcomeDemo(NOW));
    expect(new Set(rows.map((row) => row.agent))).toEqual(
      new Set(["release_agent", "research_agent", "support_agent"]),
    );
    expect(summarizeOutcomes(rows).mergedPRs).toBeLessThan(
      rows.filter((row) => row.pullRequest?.status === "merged").length,
    );
    expect(
      rows.every(
        (row) =>
          Date.parse(row.startedAt) < Date.parse(row.finishedAt) &&
          Date.parse(row.finishedAt) < NOW,
      ),
    ).toBe(true);
    expect(
      filterOutcomeSessions(rows, { days: 7, status: "all" }, NOW).length,
    ).toBeLessThan(
      filterOutcomeSessions(rows, { days: 30, status: "all" }, NOW).length,
    );
  });
});
