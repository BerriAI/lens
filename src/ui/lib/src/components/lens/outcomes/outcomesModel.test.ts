import { describe, expect, it } from "vitest";
import {
  userOutcomes,
  taskBreakdown,
  taskActivity,
  CODING_AGENTS,
  createOutcomeDemo,
  filterOutcomeSessions,
  outcomeStatus,
  summarizeOutcomes,
  type OutcomeSession,
} from "./outcomesModel";

const NOW = Date.parse("2026-11-05T12:00:00Z");
const HOUR_MS = 3_600_000;

const session = (overrides: Partial<OutcomeSession> = {}): OutcomeSession => ({
  id: "session-1",
  user: "alex@example.test",
  team: "Platform",
  codingAgent: "codex",
  taskCategory: "Coding",
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

describe("sample outcome cohorts", () => {
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

describe("user spend and PR work", () => {
  it("splits PR-linked and non-PR spend and counts shared merged PRs once within each user", () => {
    const rows = [
      session({ costUSD: 12 }),
      session({ id: "follow-up", costUSD: 8 }),
      session({
        id: "draft",
        costUSD: 10,
        pullRequest: { id: 99, title: "Draft", status: "open" },
        issue: null,
      }),
      session({
        id: "research",
        costUSD: 9,
        taskCategory: "Research",
        pullRequest: null,
        issue: null,
      }),
      session({
        id: "documentation",
        costUSD: 6,
        taskCategory: "Documentation",
        pullRequest: null,
        issue: null,
      }),
      session({
        id: "review",
        costUSD: 5,
        taskCategory: "Research",
        pullRequest: null,
        issue: null,
      }),
      session({ id: "another-user", user: "sam@example.test", costUSD: 25 }),
    ];
    const users = userOutcomes(rows);
    expect(users.map((user) => user.user)).toEqual([
      "alex@example.test",
      "sam@example.test",
    ]);
    expect(users[0]).toEqual({
      user: "alex@example.test",
      sessions: 6,
      totalCostUSD: 50,
      spendShare: 50 / 75,
      prSessions: 3,
      prSpendUSD: 30,
      mergedPRs: 1,
      prCostPerMerge: 30,
      nonPRSessions: 3,
      nonPRSpendUSD: 20,
      nonPRShare: 0.4,
      taskKinds: [
        { kind: "Coding", sessions: 3, sessionShare: 0.5 },
        { kind: "Research", sessions: 2, sessionShare: 2 / 6 },
        { kind: "Documentation", sessions: 1, sessionShare: 1 / 6 },
      ],
      closedIssues: 1,
      nonPRTaskKinds: [
        { kind: "Research", sessions: 2, sessionShare: 2 / 3 },
        { kind: "Documentation", sessions: 1, sessionShare: 1 / 3 },
      ],
    });
    expect(users[1].mergedPRs).toBe(1);
  });

  it("keeps same-number merged PRs in different repositories distinct for a user", () => {
    const [user] = userOutcomes([
      session(),
      session({ id: "other-repo", repository: "acme/console" }),
    ]);
    expect(user.mergedPRs).toBe(2);
    expect(user.prCostPerMerge).toBe(10);
  });

  it("leaves PR cost per merge unavailable when a user has no merged PR", () => {
    const [user] = userOutcomes([
      session({
        pullRequest: { id: 42, title: "Open", status: "open" },
        issue: null,
      }),
      session({
        id: "notes",
        pullRequest: null,
        issue: null,
        taskCategory: "Documentation",
        costUSD: 0,
      }),
    ]);
    expect(user.prSessions).toBe(1);
    expect(user.mergedPRs).toBe(0);
    expect(user.prCostPerMerge).toBeNull();
    expect(user.nonPRTaskKinds).toEqual([
      { kind: "Documentation", sessions: 1, sessionShare: 1 },
    ]);
  });

  it("counts issue-only sessions as non-PR work and handles zero spend without invalid shares", () => {
    const [user] = userOutcomes([
      session({ pullRequest: null, taskCategory: "Research", costUSD: 0 }),
    ]);
    expect(user).toMatchObject({
      prSessions: 0,
      prSpendUSD: 0,
      nonPRSessions: 1,
      nonPRSpendUSD: 0,
      nonPRShare: 0,
      spendShare: 0,
    });
    expect(user.nonPRTaskKinds).toEqual([
      { kind: "Research", sessions: 1, sessionShare: 1 },
    ]);
    expect(userOutcomes([])).toEqual([]);
  });
});

describe("coding agent, team and date scopes", () => {
  it("intersects coding agent, team and exact date boundaries", () => {
    const rows = [
      session({ id: "start", startedAt: "2026-11-01T00:00:00Z" }),
      session({ id: "end", startedAt: "2026-11-03T23:59:59.999Z" }),
      session({ id: "before", startedAt: "2026-10-31T23:59:59.999Z" }),
      session({ id: "after", startedAt: "2026-11-04T00:00:00Z" }),
      session({
        id: "other-team",
        team: "Product",
        startedAt: "2026-11-02T12:00:00Z",
      }),
      session({
        id: "other-agent",
        codingAgent: "claude_code",
        startedAt: "2026-11-02T12:00:00Z",
      }),
    ];
    expect(
      filterOutcomeSessions(
        rows,
        {
          days: 7,
          status: "all",
          team: "Platform",
          codingAgent: "codex",
          from: "2026-11-01",
          to: "2026-11-03",
        },
        NOW,
      ).map((row) => row.id),
    ).toEqual(["start", "end"]);
  });

  it.each([
    ["reversed", "2026-11-03", "2026-11-01"],
    ["invalid calendar date", "2026-02-30", "2026-11-03"],
    ["invalid format", "November 1", "2026-11-03"],
  ])(
    "keeps invalid %s ranges empty in filters and activity",
    (_label, from, to) => {
      expect(
        filterOutcomeSessions(
          [session()],
          { days: 7, status: "all", from, to },
          NOW,
        ),
      ).toEqual([]);
      expect(taskActivity([session()], 7, NOW, from, to)).toEqual([]);
    },
  );

  it("uses the selected end day for a rolling fallback when only to is supplied", () => {
    const row = session({ startedAt: "2026-10-27T00:00:00Z" });
    expect(
      filterOutcomeSessions(
        [row],
        { days: 7, status: "all", to: "2026-11-02" },
        NOW,
      ),
    ).toEqual([row]);
    expect(
      filterOutcomeSessions([row], { days: 7, status: "all" }, NOW),
    ).toEqual([]);
  });

  it("keeps all work in each person's task mix and deduplicates closed issues", () => {
    const rows = [
      session({ taskCategory: "Bug fixes" }),
      session({ id: "follow-up", taskCategory: "Testing" }),
      session({
        id: "notes",
        taskCategory: "Docs & research",
        pullRequest: null,
        issue: null,
      }),
    ];
    const [user] = userOutcomes(rows);
    expect(user.closedIssues).toBe(1);
    expect(user.mergedPRs).toBe(1);
    expect(user.taskKinds).toEqual([
      { kind: "Bug fixes", sessions: 1, sessionShare: 1 / 3 },
      { kind: "Docs & research", sessions: 1, sessionShare: 1 / 3 },
      { kind: "Testing", sessions: 1, sessionShare: 1 / 3 },
    ]);
    expect(taskBreakdown(rows)).toEqual(user.taskKinds);
    expect(user.nonPRTaskKinds).toEqual([
      { kind: "Docs & research", sessions: 1, sessionShare: 1 },
    ]);
  });

  it("counts each session once in daily activity including partial boundary days", () => {
    const start = NOW - 7 * 24 * HOUR_MS;
    const rows = [
      session({
        id: "partial-first",
        startedAt: new Date(start).toISOString(),
        taskCategory: "Bug fixes",
      }),
      session({ id: "last-day", taskCategory: "Testing" }),
      session({ id: "another-last-day", taskCategory: "Bug fixes" }),
      session({ id: "outside", startedAt: new Date(start - 1).toISOString() }),
    ];
    const activity = taskActivity(rows, 7, NOW);
    expect(activity).toHaveLength(8);
    expect(activity[0]).toEqual({
      date: "2026-10-29",
      endDate: "2026-10-29",
      counts: { "Bug fixes": 1, Testing: 0 },
      sessions: 1,
    });
    expect(activity.at(-1)).toEqual({
      date: "2026-11-05",
      endDate: "2026-11-05",
      counts: { "Bug fixes": 1, Testing: 1 },
      sessions: 2,
    });
    expect(activity.reduce((sum, bucket) => sum + bucket.sessions, 0)).toBe(3);
  });

  it("bounds long-range activity without dropping selected sessions or task kinds", () => {
    const rows = [
      session({
        id: "oldest",
        startedAt: "2020-01-01T00:00:00Z",
        taskCategory: "Bug fixes",
      }),
      session({
        id: "middle",
        startedAt: "2023-06-02T12:00:00Z",
        taskCategory: "Features",
      }),
      session({ id: "newest", taskCategory: "Code review" }),
    ];
    const activity = taskActivity(rows, 7, NOW, "2020-01-01", "2026-11-05");
    expect(activity.length).toBeLessThanOrEqual(366);
    expect(activity[0].date).toBe("2020-01-01");
    expect(activity.at(-1)?.endDate).toBe("2026-11-05");
    expect(activity.reduce((sum, bucket) => sum + bucket.sessions, 0)).toBe(
      rows.length,
    );
    expect(
      activity
        .flatMap((bucket) => Object.values(bucket.counts))
        .reduce((sum, count) => sum + count, 0),
    ).toBe(rows.length);
  });

  it("includes both coding agents and stable team membership in the sample cohort", () => {
    const rows = createOutcomeDemo(NOW);
    expect(rows.filter((row) => row.codingAgent === "codex").length).toBe(
      rows.filter((row) => row.codingAgent === "claude_code").length,
    );
    expect(new Set(rows.map((row) => row.codingAgent))).toEqual(
      new Set(CODING_AGENTS.map((agent) => agent.value)),
    );
    expect(new Set(rows.map((row) => row.team))).toEqual(
      new Set(["Platform", "Product", "Developer experience"]),
    );
    expect(
      new Set(
        rows.filter((row) => row.user === rows[0].user).map((row) => row.team),
      ).size,
    ).toBe(1);
    expect(new Set(rows.map((row) => row.taskCategory))).toEqual(
      new Set([
        "Bug fixes",
        "Features",
        "Testing",
        "Code review",
        "Docs & research",
      ]),
    );
  });
  it("keeps representative delivered and non-PR work in both coding-agent previews", () => {
    const rows = createOutcomeDemo(NOW);
    for (const agent of CODING_AGENTS) {
      const selected = rows.filter((row) => row.codingAgent === agent.value);
      expect(selected.some((row) => row.pullRequest?.status === "merged")).toBe(
        true,
      );
      expect(selected.some((row) => row.issue?.status === "closed")).toBe(true);
      expect(selected.some((row) => row.pullRequest === null)).toBe(true);
    }
  });
});
